//! The privileged transaction: one `pkexec` run of `xbps-install`/`xbps-remove`.
//!
//! The GUI thread never waits. `start` runs on it, claims the
//! single-transaction slot, arms the job's poll timer and spawns a worker
//! thread; everything the worker learns about the process is queued back with
//! `qt_thread().queue(...)`, the only way CXX-Qt lets a background thread touch
//! a QObject.
//!
//! Three properties describe the job:
//!
//! - `cancellable` is `true` only while the job waits for polkit and the
//!   process does not exist yet. Once `xbps` runs, `cancel` is a no-op: a
//!   transaction killed halfway is worse than one that finishes.
//! - `exitCode` is the process' own result, mapped by [`Outcome`]: `0` success,
//!   `126` dialog dismissed, `127` not authorized, `-1` pkexec missing, and
//!   anything else xbps' code, passed through.
//! - `labelKey`/`labelArg` carry the outcome as a translation key and one
//!   argument; QML maps them with `i18nd()`. Log lines are command output and
//!   stay untranslated.
//!
//! ## Why the GUI polls
//!
//! The process runs on a worker thread, but the window that shows its output
//! lives on the GUI thread. `JobDialog.qml` therefore runs a 250 ms `Timer` that
//! calls `tick(elapsed)` for as long as the job is not `done`; each pass shows
//! the lines that arrived since the last one and moves the progress bar, so a
//! five-minute transaction keeps updating instead of printing everything at the
//! end. Lines travel through [`LineRegistry`], which the worker writes and the
//! timer drains: a burst costs one `linesReady` signal per pass, never one
//! signal per line.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, QObject, Threading};
use cxx_qt_lib::{QList, QObjectMutPtr, QString};
use kxbps_core::transaction::Outcome;
use kxbps_core::{Exit, PlanItem, TransactionPlan};
use serde::{Deserialize, Serialize};

/// The argv of the mirror helper, from the one place that defines it.
fn mirror_argv(base: &str) -> Vec<String> {
    kxbps_core::mirror_argv(base)
}

/// The list type the job's `pendingLines` property carries; the name is the one
/// cxx-qt-lib gives the C++ alias, so the generated property matches it.
#[allow(non_camel_case_types)]
type QList_QString = QList<QString>;

/// Interval of the GUI-side poll that keeps progress moving while the
/// privileged process runs.
pub const TICK_MS: u32 = 250;

/// How long one progress ramp takes; the bar never reaches 100 % on its own.
const RAMP_MS: i64 = 60_000;

/// Progress ceiling while the process runs.
const PROGRESS_CEILING: i32 = 90;

/// How long a finished worker waits for the GUI to take its last event before
/// it stops queueing. A closed or destroyed job never answers.
const END_SLICE: Duration = Duration::from_millis(200);

/// The single privileged transaction the program may run.
///
/// pkexec and xbps touch the system, so a second transaction would interleave
/// two lock holders. The flag is cleared when the owning job ends; nothing is
/// queued behind it.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// How a [`JobSlotState`] got answered, or that it was never answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotAsk {
    /// This job now owns the slot.
    Taken,
    /// Another job owns the slot; this one reported `jobBusy`.
    Busy,
    /// The GUI never answered inside `END_SLICE`: the queued `begin` may still
    /// run later, so the caller must queue the rollback described in
    /// [`slot_survives`].
    Unanswered,
}

/// The state a job's `begin` leaves behind, and the handle its rollback reads.
///
/// The single-transaction slot is process-wide, and the `begin` that takes it
/// runs on the GUI thread whenever the worker's 200 ms slice has run out. The
/// only way a rollback can then tell "I own it" from "I never got it" is a flag
/// this job alone owns: `claimed` is set by `begin_transaction` on the GUI
/// thread and cleared by `settle`, `rollback` or `finish`. A `false` flag means
/// releasing `RUNNING` is somebody else's business — the process-global flag may
/// belong to a newer transaction by then.
#[derive(Debug, Default)]
pub struct JobSlotState {
    claimed: AtomicBool,
}

impl JobSlotState {
    /// Whether a `begin` took the slot for this job and no close released it.
    pub fn claimed(&self) -> bool {
        self.claimed.load(Ordering::SeqCst)
    }

    /// Records that this job owns the slot.
    pub fn claim(&self) {
        self.claimed.store(true, Ordering::SeqCst);
    }

    /// Drops the claim and releases the process-wide slot if this job held it.
    ///
    /// Only ever called on the GUI thread, where `begin_transaction` and
    /// `settle` run too, so the flag is never read and cleared from two threads
    /// at once. A `false` answer means the process-wide flag may now belong to a
    /// newer transaction, which is why only the job that set it clears it.
    pub fn release(&self) -> bool {
        if !self.claimed.swap(false, Ordering::SeqCst) {
            return false;
        }
        RUNNING.store(false, Ordering::SeqCst);
        true
    }
}

/// Whether the slot still answers after a [`Job::begin`] that timed out.
///
/// `begin` queues `claim` on the GUI thread and waits `END_SLICE`. A timeout
/// proves nothing about the queued closure: it may still run, take the slot and
/// set the job up as its owner. Left alone that is a slot nobody releases — the
/// job is gone from the worker's point of view but alive in the GUI, so every
/// later transaction reports `jobBusy` for as long as the program runs. The
/// worker therefore queues `rollback` (which closes the job as [`keys::BUSY`] or
/// ends it as a cancel, whichever state the closure found) straight after the
/// unanswered claim; this is the worker's policy in one line.
pub fn slot_survives(answer: SlotAsk) -> bool {
    answer == SlotAsk::Taken
}

/// Upper platform timer interval: anything above `i32::MAX` cannot be a
/// millisecond interval and is clamped so the conversion cannot wrap.
pub fn timer_interval(tick_ms: u32) -> i32 {
    i32::try_from(tick_ms).unwrap_or(i32::MAX)
}

/// Whether one arriving line leaves the job waiting for a lock, and whether
/// that is a change.
///
/// xbps does not fail on a held lock: it prints `package database locked,
/// waiting...` (or `repository locked: <url>: waiting...`) and waits, then
/// carries on printing progress once it holds it. So the flag is not
/// "a lock line was ever seen": a lock line raises it and every other line
/// lowers it, which is what makes the table row say "waiting for another
/// process" for exactly as long as that is true.
///
/// `waiting` is the job's current value; the answer is whether it changed, so
/// the caller writes the property only on a transition and a run of ordinary
/// lines costs no notification at all. The predicate itself lives in
/// [`kxbps_core::lock_waited_on`], next to the format strings it reads.
pub fn lock_state(waiting: &mut bool, line: &str) -> bool {
    let now = kxbps_core::lock_waited_on(line);
    if *waiting == now {
        return false;
    }
    *waiting = now;
    true
}

/// Progress shown `elapsed_ms` into a transaction.
///
/// xbps reports no percentage, so the bar is deliberately indeterminate in
/// spirit: it climbs towards 90 % by time and only a real exit code 0 reaches
/// 100 %. A negative elapsed time reads as 0, so a broken clock cannot draw a
/// full bar.
pub fn progress_at(elapsed_ms: i64) -> i32 {
    if elapsed_ms <= 0 {
        return 0;
    }
    let climbed = i64::from(PROGRESS_CEILING) * elapsed_ms.min(RAMP_MS) / RAMP_MS;
    i32::try_from(climbed)
        .unwrap_or(PROGRESS_CEILING)
        .clamp(0, PROGRESS_CEILING)
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/core/qlist/qlist_QString.h");
        type QList_QString = cxx_qt_lib::QList<QString>;

        include!("backend_shim.h");

        #[cxx_name = "kxbps_create_log_model"]
        fn create_log_model(parent: QObjectMutPtr) -> QObjectMutPtr;

        #[cxx_name = "kxbps_log_model_append"]
        fn log_model_append(model: QObjectMutPtr, line: &QString);

        #[cxx_name = "kxbps_installed_apply_plan"]
        fn installed_apply_plan(owner: QObjectMutPtr, items_json: &QString);

        /// Tells the store owner that this mirror job ended, so the page that
        /// started it clears its busy flag and re-reads the system state.
        #[cxx_name = "kxbps_installed_mirror_job_landed"]
        fn installed_mirror_job_landed(owner: QObjectMutPtr);

        /// Tells the store owner that a hold job ended, so the pill and the
        /// filters re-read what xbps now holds.
        #[cxx_name = "kxbps_installed_hold_landed"]
        fn installed_hold_landed(owner: QObjectMutPtr);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, progress, READ, NOTIFY)]
        #[qproperty(QString, label_key, READ, NOTIFY)]
        #[qproperty(QString, label_arg, READ, NOTIFY)]
        /// One line naming the commands this job will run, for the dialog. The
        /// label says which step; this says what is being executed.
        #[qproperty(QString, command_text, READ, NOTIFY)]
        #[qproperty(bool, done, READ, NOTIFY)]
        #[qproperty(bool, cancelled, READ, NOTIFY)]
        #[qproperty(bool, failed, READ, NOTIFY)]
        #[qproperty(i32, exit_code, READ, NOTIFY)]
        #[qproperty(bool, cancellable, READ, NOTIFY)]
        #[qproperty(QObjectMutPtr, lines, READ, CONSTANT)]
        /// The lines no `tick` has shown yet; QML reacts to its notification.
        ///
        /// Set on the job's own thread while the worker runs, so the CXX-Qt
        /// bridge reaches QML with a queued connection: the read never runs on
        /// the worker, and a burst of lines costs one property write, not one
        /// signal per line.
        /// Whether the job is waiting for another process to release an xbps
        /// lock.
        ///
        /// xbps prints a lock line and waits instead of failing, and until now
        /// the dialog looked hung. A lock line raises this flag, the next line
        /// that is not one lowers it (once xbps holds the lock it prints
        /// progress), and `settle` clears it, so it is true for exactly as long
        /// as the wait lasts. Read through [`lock_state`].
        #[qproperty(bool, waiting_for_lock, READ, NOTIFY)]
        #[qproperty(QList_QString, pending_lines, READ, NOTIFY)]
        type JobBackend = super::JobBackendRust;

        /// Stops a job whose process does not exist yet; a no-op afterwards.
        #[qinvokable]
        fn cancel(self: Pin<&mut JobBackend>);

        /// Called by `kxbps_start_job`; not invokable so QML cannot restart a
        /// running job.
        fn start(self: Pin<&mut JobBackend>, payload_json: &QString, owner: QObjectMutPtr);

        /// Takes the single-transaction slot; `false` means another job owns
        /// it and this one is already finished as `busy`.
        fn begin_transaction(self: Pin<&mut JobBackend>) -> bool;

        /// The process exists: a cancel from here on is ignored.
        fn mark_spawned(self: Pin<&mut JobBackend>);

        /// The GUI's 250 ms pass, called by `JobDialog.qml`.
        ///
        /// Appends everything in `pendingLines` to the log model, empties the
        /// queue and moves the progress bar. `elapsedMs` counts from `start`; a
        /// finished job answers without touching anything, so a timer that
        /// fires once more cannot revive it.
        #[qinvokable]
        fn tick(self: Pin<&mut JobBackend>, elapsed_ms: i64) -> bool;

        /// Records the final state and applies the plan on success.
        ///
        /// A `finish` after a rollback is ignored: the job is already closed and
        /// the transaction it describes never ran.
        fn finish(self: Pin<&mut JobBackend>, outcome_json: &QString);

        /// Closes a job whose `begin` the worker never saw answered.
        ///
        /// The queued `begin_transaction` may still run on the GUI thread after
        /// the worker gave up, taking the slot and leaving the label on "Waiting
        /// for authentication…". This runs right behind it: if the job ever
        /// claimed the slot the claim is released, so no later transaction
        /// reports `jobBusy` for the rest of the program's life. A job that
        /// finished or was cancelled in between is left as it is.
        fn rollback_transaction(self: Pin<&mut JobBackend>);
    }

    impl cxx_qt::Threading for JobBackend {}

    impl
        cxx_qt::Constructor<
            (QString, QObjectMutPtr),
            BaseArguments = (QObjectMutPtr,),
            NewArguments = (QString,),
        > for JobBackend
    {
    }
}

/// Translation keys for the job label; QML maps them with `i18nd()`.
pub mod keys {
    /// Waiting for polkit to authenticate.
    pub const START: &str = "jobStart";
    /// xbps is working.
    pub const WORKING: &str = "jobWorking";
    /// The user cancelled before the process existed.
    pub const CANCELLED: &str = "jobCancelled";
    /// xbps exited 0.
    pub const DONE: &str = "jobDone";
    /// pkexec exit 126: the dialog was dismissed or polkit refused to start.
    pub const AUTH_DISMISSED: &str = "jobAuthDismissed";
    /// pkexec exit 127: not authorized.
    pub const AUTH_DENIED: &str = "jobAuthDenied";
    /// exit -1: pkexec is missing or not executable.
    pub const PKEXEC_MISSING: &str = "jobPkexecMissing";
    /// xbps failed (`%1` = its exit code).
    pub const XPBS_FAILED: &str = "jobXbpsFailed";
    /// Another transaction is running.
    pub const BUSY: &str = "jobBusy";
    /// The mirror helper wrote, synced and verified (`%1` = the URL).
    pub const MIRROR_APPLY_DONE: &str = "mirrorApplyDone";
    /// The mirror helper exited with a code it does not document for itself.
    pub const MIRROR_APPLY_FAILED: &str = "errorMirrorHelper";
    /// The repository helper wrote one file (`%1` = the URL).
    pub const REPO_ADD_DONE: &str = "repoAddDone";
    /// The repository helper removed one file (`%1` = the file name).
    pub const REPO_REMOVE_DONE: &str = "repoRemoveDone";
    /// The repository file was written, but `xbps-install -S` failed
    /// (`%1` = the URL).
    pub const REPO_SYNC_FAILED: &str = "repoSyncFailed";
    /// `xbps-pkgdb` pinned the package (`%1` = its name).
    pub const HOLD_DONE: &str = "holdDone";
    /// `xbps-pkgdb` released the package again (`%1` = its name).
    pub const UNHOLD_DONE: &str = "unholdDone";
}

/// What the job thread needs: the plan the dialog was built from, or the one
/// mirror to apply.
///
/// The payload is the binary interface between QML, the installed backend and
/// the worker: every field is optional, so the plan shape this file has always
/// accepted still decodes. `items` non-empty is a package transaction
/// ([`TransactionPlan`]); `mirror` names the base URL of the mirror helper.
/// Both at once is not a thing the UI produces, and a package transaction wins
/// so an old payload can never turn into a mirror write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobPayload {
    #[serde(default)]
    pub items: Vec<PlanItem>,
    /// The base URL of `kxbps-mirror-apply`; `None` for a package transaction.
    #[serde(default)]
    pub mirror: Option<String>,
    /// One repository edit for `kxbps-repo-apply`; `None` for the other jobs.
    #[serde(default)]
    pub repo: Option<RepoRequest>,
    /// One hold change for `xbps-pkgdb`; `None` for the other jobs.
    #[serde(default)]
    pub hold: Option<HoldRequest>,
}

/// One hold change: the package and the state it should end in.
///
/// `held` is the target state, not the current one, so a stale click cannot
/// flip a package the user just released.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldRequest {
    pub name: String,
    pub held: bool,
}

/// One repository edit: the verb the helper takes and its value (a URL for
/// `add`, a bare file name for `remove`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRequest {
    pub verb: String,
    pub value: String,
}

impl JobPayload {
    /// The commands the worker runs for this payload, in order.
    ///
    /// A package plan keeps [`TransactionPlan::commands`] — installs first,
    /// removals after. A mirror payload is one command:
    /// `pkexec /usr/libexec/kxbps-mirror-apply <base>`, with the URL as a
    /// single argv element. A payload with neither runs nothing.
    pub fn commands(&self) -> Vec<Vec<String>> {
        if let Some(plan) = TransactionPlan::for_items(&self.items) {
            return plan.commands();
        }
        if let Some(repo) = self
            .repo
            .as_ref()
            .filter(|repo| (repo.verb == "add" || repo.verb == "remove") && !repo.value.is_empty())
        {
            return vec![kxbps_core::transaction::repo_argv(&repo.verb, &repo.value)];
        }
        if let Some(hold) = self.hold.as_ref().filter(|hold| !hold.name.is_empty()) {
            return vec![kxbps_core::transaction::hold_argv(&hold.name, hold.held)];
        }
        match &self.mirror {
            Some(base) if !base.is_empty() => vec![mirror_argv(base)],
            _ => Vec::new(),
        }
    }

    /// Whether this payload pins or releases one package.
    pub fn is_hold(&self) -> bool {
        self.hold.as_ref().is_some_and(|hold| !hold.name.is_empty())
    }

    /// Whether this payload is a mirror apply, i.e. not a package transaction.
    /// Whether this payload is one repository edit.
    pub fn is_repo(&self) -> bool {
        self.repo.as_ref().is_some_and(|repo| {
            (repo.verb == "add" || repo.verb == "remove") && !repo.value.is_empty()
        })
    }

    pub fn is_mirror(&self) -> bool {
        TransactionPlan::for_items(&self.items).is_none()
            && self.mirror.as_deref().is_some_and(|base| !base.is_empty())
    }
}

/// The payload JSON of one mirror apply, as `kxbps_start_job` takes it.
pub fn mirror_payload_json(base: &str) -> String {
    serde_json::to_string(&JobPayload {
        items: Vec::new(),
        mirror: Some(base.to_owned()),
        repo: None,
        hold: None,
    })
    .unwrap_or_else(|_| String::from("{}"))
}

/// The payload JSON of one repository edit, as `kxbps_start_job` takes it.
pub fn repo_payload_json(verb: &str, value: &str) -> String {
    serde_json::to_string(&JobPayload {
        items: Vec::new(),
        mirror: None,
        repo: Some(RepoRequest {
            verb: verb.to_owned(),
            value: value.to_owned(),
        }),
        hold: None,
    })
    .unwrap_or_else(|_| String::from("{}"))
}

/// The payload JSON of one hold change, as `kxbps_start_job` takes it.
pub fn hold_payload_json(name: &str, held: bool) -> String {
    serde_json::to_string(&JobPayload {
        items: Vec::new(),
        mirror: None,
        repo: None,
        hold: Some(HoldRequest {
            name: name.to_owned(),
            held,
        }),
    })
    .unwrap_or_else(|_| String::from("{}"))
}

/// The job's final state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum End {
    /// Stopped before the process existed; nothing was applied.
    Cancelled,
    /// The transaction never started because another one owns the slot, or
    /// because the GUI never confirmed that this one took it.
    Busy,
    /// The process ran; `code` is its raw exit code.
    Exit { code: i32 },
}

impl End {
    /// The outcome of an exit code, or a cancel.
    pub fn of(exit_code: i32) -> Self {
        Self::Exit { code: exit_code }
    }

    /// The mapped outcome; neither a cancel nor a busy close has one.
    pub fn outcome(self) -> Option<Outcome> {
        match self {
            Self::Cancelled | Self::Busy => None,
            Self::Exit { code } => Some(Outcome::from_exit_code(code)),
        }
    }
}

/// What one exit code means for one kind of job.
///
/// A package transaction keeps Task 19's mapping: `0` success, `126`/`127`/`-1`
/// pkexec's own answers, anything else an xbps failure with its code as the
/// argument.
///
/// The mirror helper answers with codes of its own (see
/// [`kxbps_core::mirror_apply::Exit`]) that overlap pkexec's alphabet: `2`, `3`,
/// `4` and `5` are the helper's refusals and failures, and 126/127 still mean
/// pkexec never got that far. The mapping goes through
/// The line a finished job appends to its log.
///
/// The subject is the value the job was given — a mirror URL, a repository file
/// or a package name — so the line names the thing that changed instead of
/// repeating the tool. A package plan has no single subject and reports its
/// count.
fn success_line(kind: JobKind, subject: &str, item_count: usize) -> String {
    match kind {
        JobKind::HoldApply { held } => {
            format!("{subject} {}.", if held { "held" } else { "unheld" })
        }
        JobKind::MirrorApply => format!("mirror {subject} applied."),
        JobKind::RepoApply { remove } => {
            format!(
                "repository {subject} {}.",
                if remove { "removed" } else { "added" }
            )
        }
        JobKind::Plan => format!("{item_count} packages processed."),
    }
}

/// [`kxbps_core::transaction::Outcome`] first so the pkexec codes stay one
/// definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// `pkexec xbps-install` / `pkexec xbps-remove`.
    Plan,
    /// `pkexec /usr/libexec/kxbps-mirror-apply <base>`.
    MirrorApply,
    /// `pkexec /usr/libexec/kxbps-repo-apply <verb> <value>`.
    RepoApply { remove: bool },
    /// `pkexec /usr/bin/xbps-pkgdb -m hold|unhold <name>`.
    HoldApply { held: bool },
}

impl JobKind {
    /// Whether this job is one repository edit.
    pub fn is_repo(self) -> bool {
        matches!(self, Self::RepoApply { .. })
    }

    /// The kind a payload describes; a package plan wins when both are present.
    pub fn of(payload: &JobPayload) -> Self {
        if let Some(hold) = payload.hold.as_ref().filter(|hold| !hold.name.is_empty()) {
            Self::HoldApply { held: hold.held }
        } else if payload.is_repo() {
            Self::RepoApply {
                remove: payload
                    .repo
                    .as_ref()
                    .is_some_and(|repo| repo.verb == "remove"),
            }
        } else if payload.is_mirror() {
            Self::MirrorApply
        } else {
            Self::Plan
        }
    }

    /// The verdict of one exit code for this kind of job.
    ///
    /// `detail` is the helper's own last stderr line; `url` is the base the job
    /// asked the helper to apply. A mirror verdict passes both to the keys that
    /// take them; a package plan ignores `url`.
    pub fn verdict(self, code: i32, detail: &str, url: &str) -> Verdict {
        match self {
            Self::Plan => Verdict::of(End::of(code)),
            Self::MirrorApply => mirror_verdict(code, detail, url),
            Self::RepoApply { remove } => repo_verdict(code, url, remove),
            Self::HoldApply { held } => hold_verdict(code, url, held),
        }
    }
}

/// The repository helper's verdict for one exit code.
///
/// `0` is the helper's own success; its documented refusals map to the key the
/// page shows with the value the user asked for. 126/127/-1 stay pkexec's, the
/// same as for the mirror helper.
fn repo_verdict(code: i32, value: &str, remove: bool) -> Verdict {
    // The file is written; only the index fetch failed. Not a failed edit: the
    // sentence says the repository is there and what to retry.
    if code == kxbps_core::repository_edit::SYNC_FAILED {
        return Verdict {
            done: true,
            failed: false,
            cancelled: false,
            exit_code: code,
            key: keys::REPO_SYNC_FAILED,
            args: vec![value.to_owned()],
        };
    }
    if code == 0 {
        return Verdict {
            done: true,
            failed: false,
            cancelled: false,
            exit_code: 0,
            key: if remove {
                keys::REPO_REMOVE_DONE
            } else {
                keys::REPO_ADD_DONE
            },
            args: vec![value.to_owned()],
        };
    }
    match kxbps_core::repository_edit::RepoError::from_code(code) {
        Some(error) => Verdict {
            done: true,
            failed: true,
            cancelled: false,
            exit_code: code,
            key: error.key(),
            args: vec![value.to_owned()],
        },
        None => Verdict::of(End::of(code)),
    }
}

/// The verdict of one `xbps-pkgdb` run.
///
/// `0` is the tool's own success, and the key names the direction so the strip
/// says "pinned" or "released" rather than a generic "done". Everything else
/// keeps the package-plan mapping, so a dismissed dialog or a missing pkexec
/// reads exactly as it does for a transaction.
fn hold_verdict(code: i32, name: &str, held: bool) -> Verdict {
    if code == 0 {
        return Verdict {
            done: true,
            failed: false,
            cancelled: false,
            exit_code: 0,
            key: if held {
                keys::HOLD_DONE
            } else {
                keys::UNHOLD_DONE
            },
            args: vec![name.to_owned()],
        };
    }
    Verdict::of(End::of(code))
}

/// The mirror helper's verdict for one exit code.
///
/// `0` is the helper's own "written, synced and verified". The documented
/// helper codes map to their keys, 126/127/-1 stay pkexec's, and anything else
/// is a helper failure the user reads with its code and stderr tail — the
/// helper never returns those for its own reasons, so it means something went
/// wrong before the helper could decide.
fn mirror_verdict(code: i32, detail: &str, url: &str) -> Verdict {
    let outcome = Outcome::from_exit_code(code);
    match outcome {
        Outcome::Success => Verdict {
            done: true,
            failed: false,
            cancelled: false,
            exit_code: 0,
            key: keys::MIRROR_APPLY_DONE,
            args: Vec::new(),
        },
        Outcome::DialogDismissed | Outcome::Denied | Outcome::PkexecMissing => Verdict {
            done: true,
            failed: true,
            cancelled: false,
            exit_code: code,
            key: outcome.key(),
            args: outcome.args(),
        },
        Outcome::XbpsFailed(code) => match Exit::from_code(code) {
            Some(exit) => mirror_failed(exit, code, detail, url),
            None => Verdict {
                done: true,
                failed: true,
                cancelled: false,
                exit_code: code,
                key: keys::MIRROR_APPLY_FAILED,
                args: vec![code.to_string(), detail.to_owned()],
            },
        },
    }
}

/// The verdict of one documented helper failure.
///
/// Arguments follow the helper's own contract:
/// - `errorMirrorUrlRefused` takes `%1` = the URL the helper refused, from
///   [`Exit::args`] — the job's own base, so a refusal names what was asked for.
/// - `errorMirrorWrite`, `errorMirrorVerify` and `errorMirrorRevert` take `%1`
///   = the helper's English sentence, which already names the path or the URL
///   it could not confirm. Inventing a path by parsing the sentence would
///   guess, and printing the same text twice helps nobody.
fn mirror_failed(exit: Exit, code: i32, detail: &str, url: &str) -> Verdict {
    // The configuration is written and verified; only the index fetch failed.
    // The job is not a failure -- the sentence says the mirror is set and what
    // to retry -- so the strip does not paint a correct mirror red.
    let failed = exit != Exit::SyncFailed;
    let args = match exit {
        Exit::UrlRefused => exit.args(url),
        Exit::Ok
        | Exit::WriteFailed
        | Exit::VerifyFailed
        | Exit::VerifyAndRevertFailed
        | Exit::SyncFailed => vec![detail.to_owned()],
    };
    Verdict {
        done: true,
        failed,
        cancelled: false,
        exit_code: code,
        key: exit.key(),
        args,
    }
}

/// Everything a finished job shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub done: bool,
    pub failed: bool,
    pub cancelled: bool,
    /// xbps/pkexec exit code; `0` after a cancel, because nothing ran.
    pub exit_code: i32,
    pub key: &'static str,
    pub args: Vec<String>,
}

impl Verdict {
    /// What the job shows for `end`.
    pub fn of(end: End) -> Self {
        match end {
            End::Cancelled => Self {
                done: true,
                failed: false,
                cancelled: true,
                exit_code: 0,
                key: keys::CANCELLED,
                args: Vec::new(),
            },
            End::Busy => busy_verdict(),
            End::Exit { code } => {
                let outcome = Outcome::from_exit_code(code);
                if outcome.success() {
                    Self {
                        done: true,
                        failed: false,
                        cancelled: false,
                        exit_code: 0,
                        key: keys::DONE,
                        args: Vec::new(),
                    }
                } else {
                    Self {
                        done: true,
                        failed: true,
                        cancelled: false,
                        exit_code: code,
                        key: outcome.key(),
                        args: outcome.args(),
                    }
                }
            }
        }
    }

    /// Whether the plan reached the system and the store may be updated.
    pub fn applied(&self) -> bool {
        self.done && !self.failed && !self.cancelled
    }
}

/// The job close the GUI reports when another transaction owns the slot.
fn busy_verdict() -> Verdict {
    Verdict {
        done: true,
        failed: true,
        cancelled: false,
        exit_code: 0,
        key: keys::BUSY,
        args: Vec::new(),
    }
}

/// The lines the worker printed and no GUI pass has shown yet.
///
/// One queue serves both sides: `push` on the worker thread, `take_first` on
/// the job's own thread, the mutex keeping the two apart. A pass drains the
/// queue into the `pendingLines` property, so a line lives in memory only until
/// the next 250 ms tick.
#[derive(Debug, Default)]
pub struct LineRegistry {
    lines: Mutex<std::collections::VecDeque<String>>,
}

impl LineRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one line for the next pass, and remembers it as the tail.
    fn push(&self, line: &str) {
        let mut lines = match self.lines.lock() {
            Ok(lines) => lines,
            Err(poison) => poison.into_inner(),
        };
        lines.push_back(line.to_owned());
    }

    /// The oldest line, or `None` once the queue is empty.
    fn take_first(&self) -> Option<String> {
        let mut lines = match self.lines.lock() {
            Ok(lines) => lines,
            Err(poison) => poison.into_inner(),
        };
        lines.pop_front()
    }
}

/// A callback run on the job's thread after its pending lines are shown.
type AfterShow = Box<dyn FnOnce(Pin<&mut qobject::JobBackend>) + Send>;

/// A job the worker thread can drive without holding the QObject.
///
/// The job's thread is the only one that touches the QObject; every call here
/// queues onto it, so the worker reaches the GUI only through its event loop.
pub struct Job {
    thread: cxx_qt::CxxQtThread<qobject::JobBackend>,
    /// Set by `cancel` on the GUI thread.
    cancel: Arc<AtomicBool>,
    /// Set by the worker once the process exists.
    spawned: Arc<AtomicBool>,
    /// The text of every line, until the GUI takes it.
    lines: Arc<LineRegistry>,
    /// Whether a publish is already queued; the next `line` joins it instead of
    /// queueing another, so a burst costs one closure.
    publishing: Arc<AtomicBool>,
}

impl Job {
    /// A job over the GUI object's line registry.
    pub fn new(
        thread: cxx_qt::CxxQtThread<qobject::JobBackend>,
        cancel: Arc<AtomicBool>,
        spawned: Arc<AtomicBool>,
        lines: Arc<LineRegistry>,
    ) -> Self {
        Self {
            thread,
            cancel,
            spawned,
            lines,
            publishing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether the user asked to stop before the process existed.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) && !self.spawned.load(Ordering::SeqCst)
    }

    /// Takes the single-transaction slot; `false` means no command may run.
    ///
    /// `false` covers both "another job owns it" and "the GUI never answered".
    /// In the second case the queued `begin` can still run later, so this
    /// queues the rollback right behind it: the slot is released and the job
    /// ends as busy or cancelled instead of keeping the slot for the rest of
    /// the program's life.
    pub fn begin(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .thread
            .queue(move |job| {
                tx.send(job.begin_transaction()).ok();
            })
            .is_err()
        {
            return false;
        }
        let answer = match rx.recv_timeout(END_SLICE) {
            Ok(true) => SlotAsk::Taken,
            Ok(false) => SlotAsk::Busy,
            Err(_) => SlotAsk::Unanswered,
        };
        if !slot_survives(answer) {
            if answer == SlotAsk::Unanswered {
                let _ = self
                    .thread
                    .queue(|mut job| job.as_mut().rollback_transaction());
            }
            return false;
        }
        true
    }

    /// The process exists; a later cancel is ignored.
    pub fn note_spawned(&self) {
        self.spawned.store(true, Ordering::SeqCst);
        let _ = self.thread.queue(|job| job.mark_spawned());
    }

    /// Hands one line to the GUI's next pass.
    ///
    /// The text joins the registry and the job's thread is asked to publish it
    /// once per burst: `publishing` keeps a thousand-line burst to one queued
    /// closure, and each closure drains the whole queue into `pendingLines`.
    pub fn line(&self, line: &str) {
        self.lines.push(line);
        if self
            .publishing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let lines = Arc::clone(&self.lines);
        let publishing = Arc::clone(&self.publishing);
        let _ = self.thread.queue(move |mut job| {
            job.as_mut().publish_lines(&lines);
            publishing.store(false, Ordering::SeqCst);
        });
    }

    /// Shows the last lines, then runs `after` on the job's thread.
    ///
    /// Both callbacks are `Box<dyn FnOnce + Send>` rather than plain closures,
    /// because `qt_thread().queue` needs a `Send` event: a caller's table of
    /// what to run next never leaves the worker. `None` means the job is gone.
    fn show(&self, after: AfterShow) -> Option<()> {
        let (tx, rx) = mpsc::channel();
        let lines = Arc::clone(&self.lines);
        let queued = self.thread.queue(move |mut job| {
            job.as_mut().publish_lines(&lines);
            after(job.as_mut());
            tx.send(()).ok();
        });
        if queued.is_err() {
            return None;
        }
        rx.recv_timeout(END_SLICE).ok()
    }

    /// Shows the last lines; `false` once the job is gone.
    pub fn show_remaining(&self) -> bool {
        self.show(Box::new(|_| {})).is_some()
    }

    /// Ends the job; the outcome travels as JSON so the shim stays dumb.
    pub fn finish(&self, end: End) {
        let json =
            serde_json::to_string(&end).unwrap_or_else(|_| String::from(r#"{"kind":"cancelled"}"#));
        self.show(Box::new(move |job| {
            job.finish(&QString::from(json.as_str()));
        }));
    }
}

/// The worker thread: runs the plan's commands in order, then ends the job.
///
/// `run` is the transport, injected so the ordering rules below are testable
/// with a fake: it receives the argv, a "may I still start?" check, a spawn
/// notification and a line callback, and returns the raw exit code. The real
/// one is [`crate::xbps_bridge::run_streaming`], whose callbacks must be
/// `'static`, so the job is shared through an `Arc` and each callback carries
/// its own clone.
///
/// The commands come from [`TransactionPlan::commands`]: every install, update
/// and reinstall in one `xbps-install`, then the removals in one `xbps-remove`.
/// The first non-zero exit ends the sequence — a removal after a failed install
/// would drop packages the plan never got to add — and the store is applied
/// only when the whole sequence exits 0.
pub fn pump<F>(job: Arc<Job>, commands: &[Vec<String>], mut run: F)
where
    F: FnMut(
        &[String],
        Box<dyn Fn() -> bool>,
        Box<dyn Fn() + Send>,
        Box<dyn Fn(&str) + Send>,
    ) -> i32,
{
    // A cancel that arrived while the job waited for its slot stops it before
    // polkit is asked anything, and before a process exists to kill.
    if job.cancelled() {
        job.finish(End::Cancelled);
        return;
    }

    let mut code = 0;
    for argv in commands {
        // `cancellable` is only true before the first spawn; from then on a
        // cancel is ignored and the sequence runs to its own end.
        let may_start: Box<dyn Fn() -> bool> = {
            let job = Arc::clone(&job);
            Box::new(move || !job.cancelled())
        };
        let spawned: Box<dyn Fn() + Send> = {
            let job = Arc::clone(&job);
            Box::new(move || job.note_spawned())
        };
        let line: Box<dyn Fn(&str) + Send> = {
            let job = Arc::clone(&job);
            Box::new(move |line: &str| job.line(line))
        };
        // The transport needs an owned copy: the closure types it takes are
        // `FnOnce`, so the iterator cannot hand out borrows of the slice.
        let argv = argv.clone();
        code = run(&argv, may_start, spawned, line);
        if code != 0 {
            break;
        }
    }
    // The transport reports -1 for a process killed before it ran anything;
    // that is a cancel, not a pkexec failure.
    let end = if job.cancelled() && !job.spawned.load(Ordering::SeqCst) {
        End::Cancelled
    } else {
        End::of(code)
    };
    if !job.show_remaining() {
        // The job is gone: nothing left to show or apply.
        return;
    }
    job.finish(end);
}

impl cxx_qt::Constructor<(QString, QObjectMutPtr)> for qobject::JobBackend {
    type BaseArguments = (QObjectMutPtr,);
    type NewArguments = (QString,);
    type InitializeArguments = ();

    fn route_arguments(
        arguments: (QString, QObjectMutPtr),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        let (payload_json, parent) = arguments;
        ((payload_json,), (parent,), ())
    }

    fn new((payload_json,): Self::NewArguments) -> JobBackendRust {
        let payload: JobPayload =
            serde_json::from_str(&payload_json.to_string()).unwrap_or_default();
        JobBackendRust::new(&payload)
    }
}

/// Rust side of [`qobject::JobBackend`].
pub struct JobBackendRust {
    /// The commands, in order; empty for an empty plan.
    commands: Vec<Vec<String>>,
    /// Plan or mirror apply; decides what an exit code means.
    kind: JobKind,
    /// The base URL of a mirror apply, `None` for a package transaction.
    mirror: Option<String>,
    /// Whether the job finished: `cancel`, `pump` and `finish` are no-ops then.
    done: bool,
    /// This job's claim on the single-transaction slot; shared with the worker.
    slot: Arc<JobSlotState>,
    progress: i32,
    label_key: QString,
    label_arg: QString,
    command_text: QString,
    cancelled: bool,
    failed: bool,
    exit_code: i32,
    cancellable: bool,
    /// Lines the worker queued and the next `pump`/`tick` shows.
    /// The lines the worker delivered and no `tick` has shown yet.
    pending_lines: QList_QString,
    /// The text behind those lines, shared with the worker.
    registry: Arc<LineRegistry>,
    /// The log model; Qt owns it and destroys it with the job.
    lines: QObjectMutPtr,
    /// The store owner, as an address: `start` copies it, `finish` uses it.
    owner_addr: usize,
    /// The plan `finish` applies on success.
    items_json: String,
    /// The last line the helper printed, drained or not; a mirror verdict shows
    /// it. A mirror apply's whole output is a handful of lines, so one string
    /// is the whole buffer.
    last_line: String,
    /// Whether the job is waiting for a lock right now; see [`lock_state`].
    waiting_for_lock: bool,
    /// Whether this job was told to ask the owner for a fresh system read after
    /// a successful mirror apply. A package transaction leaves it `false`: its
    /// success goes through `installed_apply_plan` instead.
    notify_owner: bool,
    /// The slot holding the cancel flag, so `cancel` can set it.
    cancel: Arc<AtomicBool>,
}

impl JobBackendRust {
    /// Builds the job from its payload; an empty plan has nothing to run.
    fn new(payload: &JobPayload) -> Self {
        let kind = JobKind::of(payload);
        // The value the verdict names: a mirror base URL, or the repository
        // value the helper was given. Both refresh the system half afterwards.
        let mirror = if kind.is_repo() {
            payload.repo.as_ref().map(|repo| repo.value.clone())
        } else if matches!(kind, JobKind::HoldApply { .. }) {
            payload.hold.as_ref().map(|hold| hold.name.clone())
        } else {
            payload
                .mirror
                .clone()
                .filter(|base| kind == JobKind::MirrorApply && !base.is_empty())
        };
        Self {
            commands: payload.commands(),
            kind,
            mirror,
            done: false,
            slot: Arc::new(JobSlotState::default()),
            progress: 0,
            label_key: QString::from(""),
            label_arg: QString::from(""),
            command_text: QString::from(""),
            cancelled: false,
            failed: false,
            exit_code: 0,
            cancellable: false,
            pending_lines: QList_QString::default(),
            registry: Arc::new(LineRegistry::new()),
            lines: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            owner_addr: 0,
            items_json: String::new(),
            last_line: String::new(),
            waiting_for_lock: false,
            notify_owner: false,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The commands the worker runs, in order; empty for an empty literal plan.
    pub fn commands(&self) -> &[Vec<String>] {
        &self.commands
    }

    /// What kind of job this is, which decides the verdict of its exit code.
    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// The base URL of a mirror apply; `None` for a package transaction.
    pub fn mirror(&self) -> Option<&str> {
        self.mirror.as_deref()
    }

    /// Whether this job holds the single-transaction slot.
    pub fn owns_slot(&self) -> bool {
        self.slot.claimed()
    }

    /// The cancel flag the worker reads.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// How many plan items the closing log line reports.
    fn item_count(&self) -> usize {
        serde_json::from_str::<Vec<serde_json::Value>>(&self.items_json)
            .map(|items| items.len())
            .unwrap_or(0)
    }
}

impl qobject::JobBackend {
    /// Asks the user to stop; only before the process exists.
    ///
    /// The worker sees the flag, kills the process it spawned before anything
    /// ran, or ends the job without spawning at all. Once `cancellable` is
    /// `false` this is a no-op and the transaction runs to its own end.
    fn cancel(self: Pin<&mut Self>) {
        if self.done || !self.cancellable {
            return;
        }
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Takes the single-transaction slot for this job.
    fn begin_transaction(mut self: Pin<&mut Self>) -> bool {
        if RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            settle(self.as_mut(), &busy_verdict());
            return false;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.slot.claim();
            data.cancellable = true;
        }
        self.as_mut().cancellable_changed();
        true
    }

    /// Closes a job whose `begin` the worker never saw answered.
    ///
    /// The queued `begin_transaction` may have run after the worker gave up: it
    /// took the slot and left the label on "Waiting for authentication…". This
    /// runs right behind it and puts the job in a final state, releasing the
    /// slot if the `begin` ever took it — otherwise every later transaction
    /// would report `jobBusy` until the program exits. A job the `begin` never
    /// reached is closed as busy; one that was cancelled meanwhile stays a
    /// cancel, and one that already finished is left alone.
    fn rollback_transaction(mut self: Pin<&mut Self>) {
        if self.done {
            return;
        }
        let verdict = if self.cancelled {
            Verdict::of(End::Cancelled)
        } else {
            busy_verdict()
        };
        settle(self.as_mut(), &verdict);
    }

    /// The process exists: from here on a cancel is a no-op.
    fn mark_spawned(mut self: Pin<&mut Self>) {
        if !self.cancellable {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.cancellable = false;
        }
        self.as_mut().cancellable_changed();
        set_label(self.as_mut(), keys::WORKING, "");
    }

    /// Reads every line no pass has taken into `pendingLines`.
    ///
    /// The registry drops each entry as it is copied, so a line lives in memory
    /// only between the worker printing it and this pass. Assigning the property
    /// is what tells QML there is something new; a pass with nothing to show
    /// does not touch it, so the `tick` timer cannot ping-pong with QML.
    fn publish_lines(mut self: Pin<&mut Self>, lines: &LineRegistry) {
        let mut shown = QList_QString::default();
        let mut lock_changed = false;
        while let Some(line) = lines.take_first() {
            let clear_waiting = {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                data.last_line = line.clone();
                // A lock line raises `waitingForLock`, any other line lowers
                // it; only the transitions are reported, so a run of ordinary
                // lines costs no signal.
                lock_state(&mut data.waiting_for_lock, &line)
            };
            lock_changed |= clear_waiting;
            shown.append(QString::from(line.as_str()));
        }
        if shown.is_empty() {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.pending_lines = shown;
        }
        self.as_mut().pending_lines_changed();
        if lock_changed {
            self.as_mut().waiting_for_lock_changed();
        }
    }

    /// Appends everything in `pendingLines` to the log model and empties it.
    fn flush_pending(mut self: Pin<&mut Self>) {
        let shown = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            std::mem::take(&mut data.pending_lines)
        };
        for line in shown.iter() {
            append_line(self.as_ref(), &line.to_string());
        }
    }

    /// The GUI's 250 ms pass: shows what arrived since the last one.
    ///
    /// Called by `JobDialog.qml` while the job runs, so lines land in the log
    /// model as xbps prints them instead of in one burst at the end. A finished
    /// job still shows what is left (a last line the worker queued just before
    /// `finish`) and then answers `true`, so a timer that ticks once more cannot
    /// revive a job that is already closed.
    fn tick(mut self: Pin<&mut Self>, elapsed_ms: i64) -> bool {
        self.as_mut().flush_pending();
        if self.done || self.cancellable {
            return true;
        }
        let progress = progress_at(elapsed_ms);
        if self.progress != progress {
            {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                data.progress = progress;
            }
            self.as_mut().progress_changed();
        }
        true
    }

    /// The process is gone: records the result, applies the plan on success.
    fn finish(mut self: Pin<&mut Self>, outcome_json: &QString) {
        if self.done {
            // A rollback already closed this job; a late finish must not
            // revive it or apply a plan the slot never let run.
            return;
        }
        let end: End = serde_json::from_str(&outcome_json.to_string()).unwrap_or(End::Cancelled);
        self.as_mut().flush_pending();
        // A cancel that raced the spawn shows as a cancel, never as failure.
        let end = if self.cancelled { End::Cancelled } else { end };
        // The worker only ever queues onto the GUI thread, so this call lands
        // where the store and the models live.
        let owner = unsafe { QObjectMutPtr::from_raw(self.owner_addr as *mut QObject) };
        let detail = self.last_line.to_string();
        let verdict = match end {
            End::Exit { code } => {
                let url = self.mirror.as_deref().unwrap_or("");
                self.kind.verdict(code, &detail, url)
            }
            other => Verdict::of(other),
        };
        let progress = if verdict.applied() {
            100
        } else {
            self.progress
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.progress = progress;
        }
        self.as_mut().progress_changed();
        if verdict.applied() {
            append_line(self.as_ref(), "");
            let subject = self.mirror.as_deref().unwrap_or("");
            append_line(
                self.as_ref(),
                &success_line(self.kind, subject, self.item_count()),
            );
        }
        settle(self.as_mut(), &verdict);
        if verdict.applied() {
            match self.kind {
                // A hold changes what xbps pins, not what it installs: the
                // store re-reads the installed set so the pill, the filters and
                // the details pane stop showing the state from before the click.
                JobKind::HoldApply { .. } => {
                    if !owner.as_ptr().is_null() {
                        qobject::installed_hold_landed(owner);
                    }
                }
                // A mirror apply changes `/etc/xbps.d`, not the installed set:
                // the store must not be touched, and the mirror view re-reads
                // its system state through the notification below.
                JobKind::MirrorApply => {
                    if self.notify_owner && !owner.as_ptr().is_null() {
                        qobject::installed_mirror_job_landed(owner);
                    }
                }
                // The repositories page owns its own result.
                JobKind::RepoApply { .. } => {}
                JobKind::Plan => {
                    let items = QString::from(self.items_json.as_str());
                    qobject::installed_apply_plan(owner, &items);
                }
            }
        }
    }

    /// Starts the privileged transaction on a worker thread.
    fn start(mut self: Pin<&mut Self>, payload_json: &cxx_qt_lib::QString, owner: QObjectMutPtr) {
        let payload: JobPayload =
            serde_json::from_str(&payload_json.to_string()).unwrap_or_default();
        if self.commands().is_empty() {
            settle(self.as_mut(), &Verdict::of(End::Cancelled));
            return;
        }
        let commands = self.commands().to_vec();
        let items_json =
            serde_json::to_string(&payload.items).unwrap_or_else(|_| String::from("[]"));

        let self_ptr = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::JobBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        let lines = qobject::create_log_model(self_ptr);
        let registry = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.lines = lines;
            data.owner_addr = owner.as_mut_ptr() as usize;
            data.items_json = items_json;
            data.notify_owner = data.mirror.is_some();
            Arc::clone(&data.registry)
        };

        set_label(self.as_mut(), keys::START, "");
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.command_text = QString::from(command_text(&commands));
        }
        self.as_mut().command_text_changed();
        self.as_mut().progress_changed();

        let thread = self.qt_thread();
        let job = Arc::new(Job::new(
            thread,
            self.cancel_flag(),
            Arc::new(AtomicBool::new(false)),
            registry,
        ));

        std::thread::spawn(move || {
            if !job.begin() {
                return;
            }
            pump(job, &commands, crate::xbps_bridge::run_streaming);
        });
    }
}

/// One line naming the commands the worker will run, in order. Empty for an
/// empty plan, which is what the dialog's hidden line means.
pub fn command_text(commands: &[Vec<String>]) -> String {
    commands
        .iter()
        .map(|argv| kxbps_core::format::command_line(argv))
        .collect::<Vec<_>>()
        .join(" ; ")
}

/// Writes the job's label from a key and one argument.
fn set_label(mut job: Pin<&mut qobject::JobBackend>, key: &str, arg: &str) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.label_key = QString::from(key);
        data.label_arg = QString::from(arg);
    }
    job.as_mut().label_key_changed();
    job.as_mut().label_arg_changed();
}

/// Appends one line to the job's log model; a null model is not an error.
fn append_line(job: Pin<&qobject::JobBackend>, line: &str) {
    let lines = job.lines;
    if !lines.as_ptr().is_null() {
        qobject::log_model_append(lines, &QString::from(line));
    }
}

/// Records the final state and releases the slot if this job holds it.
fn settle(mut job: Pin<&mut qobject::JobBackend>, verdict: &Verdict) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.done = verdict.done;
        data.failed = verdict.failed;
        data.cancelled = verdict.cancelled;
        data.exit_code = verdict.exit_code;
        data.cancellable = false;
        // The wait ends with the job, however the job ends.
        data.waiting_for_lock = false;
        data.label_key = QString::from(verdict.key);
        data.label_arg = QString::from(verdict.args.first().map_or("", String::as_str));
        data.slot.release();
    }
    job.as_mut().cancellable_changed();
    job.as_mut().label_key_changed();
    job.as_mut().label_arg_changed();
    job.as_mut().exit_code_changed();
    job.as_mut().failed_changed();
    job.as_mut().cancelled_changed();
    job.as_mut().done_changed();
    job.as_mut().waiting_for_lock_changed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::PlanAction;
    use kxbps_core::types::PlanReason;

    fn item(name: &str, action: PlanAction, from: Option<&str>, to: Option<&str>) -> PlanItem {
        PlanItem {
            name: name.into(),
            action,
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            reason: PlanReason::Requested,
            download: 0,
            size: 0,
        }
    }

    fn jobs_commands(items: Vec<PlanItem>) -> Vec<Vec<String>> {
        JobBackendRust::new(&JobPayload {
            items,
            mirror: None,
            repo: None,
            hold: None,
        })
        .commands()
        .to_vec()
    }

    #[test]
    fn a_payload_becomes_the_pkexec_argv_of_its_transaction() {
        let commands = jobs_commands(vec![item(
            "vim",
            PlanAction::Install,
            None,
            Some("9.2.1031_1"),
        )]);
        assert_eq!(
            commands,
            [vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-install"),
                String::from("-S"),
                String::from("-y"),
                String::from("vim-9.2.1031_1"),
            ]]
        );
        assert!(
            commands.iter().flatten().all(|arg| !arg.contains("sudo")),
            "escalation is polkit only: {commands:?}"
        );
    }

    #[test]
    fn a_mixed_payload_installs_and_then_removes() {
        let commands = jobs_commands(vec![
            item("vim", PlanAction::Install, None, Some("9.2.1031_1")),
            item("tzdata", PlanAction::Remove, Some("2025.1_1"), None),
        ]);
        assert_eq!(commands.len(), 2, "one command per tool: {commands:?}");
        assert_eq!(commands[0][1], "/usr/bin/xbps-install");
        assert_eq!(commands[1][1], "/usr/bin/xbps-remove");
        assert!(
            !commands[0].iter().any(|arg| arg == "tzdata-2025.1_1"),
            "the removal never reaches xbps-install: {commands:?}"
        );
        assert!(commands[1].iter().any(|arg| arg == "tzdata-2025.1_1"));
    }

    #[test]
    fn an_empty_payload_has_nothing_to_run() {
        assert!(
            JobBackendRust::new(&JobPayload::default())
                .commands()
                .is_empty()
        );
        assert_eq!(
            TransactionPlan::for_items(&JobPayload::default().items),
            None
        );
    }

    #[test]
    fn a_mirror_payload_is_one_pkexec_helper_command() {
        let payload = JobPayload {
            items: Vec::new(),
            mirror: Some(String::from("https://repo-default.voidlinux.org/")),
            repo: None,
            hold: None,
        };
        let job = JobBackendRust::new(&payload);
        assert_eq!(job.kind(), JobKind::MirrorApply);
        assert_eq!(job.mirror(), Some("https://repo-default.voidlinux.org/"));
        assert_eq!(
            job.commands(),
            [vec![
                String::from("pkexec"),
                String::from("/usr/libexec/kxbps-mirror-apply"),
                String::from("https://repo-default.voidlinux.org/"),
            ]]
        );
        assert!(
            job.commands()
                .iter()
                .flatten()
                .all(|arg| !arg.contains("sudo")),
            "escalation is polkit only"
        );
    }

    #[test]
    fn the_mirror_payload_serialises_so_a_package_plan_still_decodes() {
        let json = mirror_payload_json("https://repo-default.voidlinux.org/");
        let payload: JobPayload = serde_json::from_str(&json).expect("the payload decodes");
        assert!(payload.items.is_empty());
        assert_eq!(
            payload.mirror.as_deref(),
            Some("https://repo-default.voidlinux.org/")
        );
        // A payload written before the mirror field existed still lands as a
        // plan, not as a mirror apply.
        let old: JobPayload = serde_json::from_str(r#"{"items":[]}"#).expect("the old shape");
        assert_eq!(old.mirror, None);
        assert_eq!(JobKind::of(&old), JobKind::Plan);
    }

    #[test]
    fn neither_items_nor_a_mirror_means_no_command() {
        let payload = JobPayload {
            items: Vec::new(),
            mirror: Some(String::new()),
            repo: None,
            hold: None,
        };
        assert!(JobBackendRust::new(&payload).commands().is_empty());
        assert_eq!(
            JobKind::of(&payload),
            JobKind::Plan,
            "an empty URL is not a job"
        );
        let both = JobPayload {
            items: vec![item("vim", PlanAction::Install, None, Some("1_1"))],
            mirror: Some(String::from("https://a/")),
            repo: None,
            hold: None,
        };
        let job = JobBackendRust::new(&both);
        assert_eq!(job.kind(), JobKind::Plan, "a plan wins over a stray mirror");
        assert_eq!(job.mirror(), None);
        assert_eq!(job.commands()[0][1], "/usr/bin/xbps-install");
    }

    #[test]
    fn the_job_does_not_validate_the_mirror_url() {
        // The helper refuses a bad URL with code 2 and a message naming it; the
        // job's only duty is to pass the value through untouched.
        for base in ["http://repo-default.voidlinux.org/", "not a url"] {
            let payload = JobPayload {
                items: Vec::new(),
                mirror: Some(String::from(base)),
                repo: None,
                hold: None,
            };
            let job = JobBackendRust::new(&payload);
            assert_eq!(job.commands()[0].len(), 3, "{base}");
            assert_eq!(job.commands()[0][2], base, "{base} stays one argument");
        }
    }

    #[test]
    fn a_repo_payload_runs_the_repository_helper() {
        let payload: JobPayload =
            serde_json::from_str(&repo_payload_json("add", "https://repo.example.org/"))
                .expect("decodes");
        assert!(payload.is_repo());
        assert!(!payload.is_mirror());
        assert_eq!(
            payload.commands()[0],
            [
                "pkexec",
                "/usr/libexec/kxbps-repo-apply",
                "add",
                "https://repo.example.org/"
            ]
        );
        assert_eq!(JobKind::of(&payload), JobKind::RepoApply { remove: false });
    }

    #[test]
    fn a_failed_index_sync_is_a_warning_not_a_failure() {
        let done = JobKind::RepoApply { remove: false }.verdict(
            kxbps_core::repository_edit::SYNC_FAILED,
            "",
            "https://repo.example.org/",
        );
        assert!(done.done && !done.failed);
        assert_eq!(done.key, keys::REPO_SYNC_FAILED);
        assert_eq!(done.args, vec!["https://repo.example.org/"]);
    }

    #[test]
    fn a_repo_exit_code_maps_to_its_key() {
        let add = JobKind::RepoApply { remove: false };
        let done = add.verdict(0, "", "https://repo.example.org/");
        assert!(done.done && !done.failed);
        assert_eq!(done.key, keys::REPO_ADD_DONE);
        assert_eq!(done.args, vec!["https://repo.example.org/"]);

        let refused = add.verdict(2, "", "http://x/");
        assert!(refused.failed);
        assert_eq!(refused.key, "errorRepoUrl");

        let remove = JobKind::RepoApply { remove: true };
        assert_eq!(
            remove.verdict(0, "", "20-kxbps-x.conf").key,
            keys::REPO_REMOVE_DONE
        );
        assert_eq!(
            remove.verdict(4, "", "20-kxbps-x.conf").key,
            "errorRepoNotManaged"
        );
    }

    #[test]
    fn a_mirror_exit_code_maps_to_its_own_keys() {
        let url = "https://repo-de.voidlinux.org/";
        let done = JobKind::MirrorApply.verdict(0, "", url);
        assert!(done.applied());
        assert_eq!(done.key, keys::MIRROR_APPLY_DONE);
        assert!(done.args.is_empty(), "success names nothing");
        assert_eq!(
            JobKind::MirrorApply
                .verdict(2, "plain HTTP is not allowed", url)
                .key,
            "errorMirrorUrlRefused"
        );
        assert_eq!(
            JobKind::MirrorApply.verdict(3, "denied", url).key,
            "errorMirrorWrite"
        );
        assert_eq!(
            JobKind::MirrorApply
                .verdict(4, "the URL is not listed by xbps-query -L", url)
                .key,
            "errorMirrorVerify"
        );
        assert_eq!(
            JobKind::MirrorApply.verdict(5, "both", url).key,
            "errorMirrorRevert"
        );
        // pkexec keeps its own codes on both kinds of job.
        for code in [126, 127, -1] {
            assert_eq!(
                JobKind::MirrorApply.verdict(code, "", url).key,
                JobKind::Plan.verdict(code, "", "").key,
                "{code}"
            );
        }
        let unknown = JobKind::MirrorApply.verdict(19, "something else", url);
        assert_eq!(unknown.key, keys::MIRROR_APPLY_FAILED);
        assert_eq!(unknown.args, ["19", "something else"]);
        assert!(!unknown.applied());
    }

    #[test]
    fn a_mirror_failure_key_carries_the_arguments_the_brief_names() {
        let eval_url = "https://repo-de.voidlinux.org/";
        // `errorMirrorUrlRefused` %1 = the URL the job asked for, never the
        // helper's sentence: the sentence would say it twice.
        let refused = JobKind::MirrorApply.verdict(
            2,
            "plain HTTP is not allowed",
            "http://repo-default.voidlinux.org/",
        );
        assert_eq!(refused.args, ["http://repo-default.voidlinux.org/"]);
        assert!(
            !refused
                .args
                .iter()
                .any(|arg| arg == "plain HTTP is not allowed")
        );

        // `errorMirrorVerify` %1 = the helper's sentence: the URL was written
        // but xbps-query -L does not list it.
        let verify = JobKind::MirrorApply.verdict(
            4,
            "https://repo-de.voidlinux.org/current is not listed by xbps-query -L",
            eval_url,
        );
        assert_eq!(
            verify.args,
            ["https://repo-de.voidlinux.org/current is not listed by xbps-query -L"]
        );

        // `errorMirrorRevert` %1 = the helper's sentence, which names the path.
        let revert = JobKind::MirrorApply.verdict(5, "revert failed: /etc/xbps.d denied", eval_url);
        assert_eq!(revert.args, ["revert failed: /etc/xbps.d denied"]);

        // `errorMirrorWrite` %1 = the helper's sentence; it names the path.
        let write = JobKind::MirrorApply.verdict(
            3,
            "/etc/xbps.d/00-repository-main.conf is a symlink",
            eval_url,
        );
        assert_eq!(
            write.args,
            ["/etc/xbps.d/00-repository-main.conf is a symlink"]
        );
    }

    #[test]
    fn a_verify_failure_carries_the_helper_sentence_as_its_only_argument() {
        let verify = JobKind::MirrorApply.verdict(
            4,
            "https://repo-de.voidlinux.org/current is not listed by xbps-query -L",
            "https://repo-de.voidlinux.org/",
        );
        assert_eq!(
            verify.args,
            ["https://repo-de.voidlinux.org/current is not listed by xbps-query -L"]
        );
        assert_eq!(verify.key, "errorMirrorVerify");
    }

    #[test]
    fn a_package_plan_keeps_the_plan_mapping() {
        assert_eq!(JobKind::Plan.verdict(0, "", "").key, keys::DONE);
        assert_eq!(JobKind::Plan.verdict(19, "", "").key, keys::XPBS_FAILED);
        assert_eq!(JobKind::Plan.verdict(19, "", "").args, ["19"]);
        assert_eq!(
            JobKind::Plan
                .verdict(19, "ignored", "https://a.example/")
                .args,
            ["19"],
            "a plan ignores the mirror URL"
        );
    }

    #[test]
    fn an_exit_code_becomes_the_documented_verdict() {
        let success = Verdict::of(End::of(0));
        assert!(success.done && !success.failed && !success.cancelled);
        assert_eq!(success.key, keys::DONE);
        assert!(success.applied());

        let dismissed = Verdict::of(End::of(126));
        assert!(dismissed.failed && !dismissed.applied());
        assert_eq!(dismissed.key, keys::AUTH_DISMISSED);
        assert_eq!(dismissed.exit_code, 126);

        let denied = Verdict::of(End::of(127));
        assert!(denied.failed);
        assert_eq!(denied.key, keys::AUTH_DENIED);

        let missing = Verdict::of(End::of(-1));
        assert!(missing.failed);
        assert_eq!(missing.key, keys::PKEXEC_MISSING);

        let xbps = Verdict::of(End::of(19));
        assert!(xbps.failed);
        assert_eq!(xbps.key, keys::XPBS_FAILED);
        assert_eq!(xbps.args, ["19"]);
        assert_eq!(xbps.exit_code, 19, "an xbps code is passed through");
    }

    #[test]
    fn a_cancel_applies_nothing_and_reports_no_failure() {
        let cancelled = Verdict::of(End::Cancelled);
        assert!(cancelled.done && !cancelled.failed && cancelled.cancelled);
        assert_eq!(cancelled.exit_code, 0, "nothing ran, so nothing failed");
        assert_eq!(cancelled.key, keys::CANCELLED);
        assert!(!cancelled.applied());
    }

    #[test]
    fn a_busy_close_fails_before_anything_ran() {
        let busy = Verdict::of(End::Busy);
        assert!(busy.done && busy.failed && !busy.cancelled);
        assert_eq!(busy.key, keys::BUSY);
        assert_eq!(busy.exit_code, 0, "no process ran, so no exit code");
        assert!(!busy.applied(), "a busy job never applies the plan");
    }

    #[test]
    fn the_progress_bar_climbs_to_its_ceiling_and_never_completes_alone() {
        assert_eq!(progress_at(0), 0);
        assert_eq!(progress_at(-500), 0, "a broken clock cannot fill the bar");
        assert_eq!(progress_at(RAMP_MS), PROGRESS_CEILING);
        assert_eq!(progress_at(RAMP_MS * 4), PROGRESS_CEILING);
        let middle = progress_at(RAMP_MS / 2);
        assert!(
            middle > 0 && middle < PROGRESS_CEILING,
            "halfway is in between: {middle}"
        );
    }

    #[test]
    fn the_timer_interval_cannot_wrap_into_a_negative_int() {
        assert_eq!(timer_interval(TICK_MS), 250);
        assert_eq!(timer_interval(u32::MAX), i32::MAX);
    }

    #[test]
    fn only_a_taken_slot_survives_a_begin() {
        assert!(slot_survives(SlotAsk::Taken));
        assert!(!slot_survives(SlotAsk::Busy));
        assert!(
            !slot_survives(SlotAsk::Unanswered),
            "an unanswered begin still needs its rollback"
        );
    }

    #[test]
    fn an_unanswered_begin_leaves_the_slot_free_after_its_rollback() {
        // The GUI's `begin` runs after the worker stopped waiting; the rollback
        // that follows it must give the process-wide slot back, or every later
        // transaction reports `jobBusy` for as long as the program runs.
        let slot = JobSlotState::default();
        let running = AtomicBool::new(false);
        assert!(
            running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "the queued begin takes the slot"
        );
        slot.claim();
        assert!(slot.release(), "the rollback releases what begin took");
        assert!(!slot.claimed());
        let _ = running;
    }

    #[test]
    fn a_rollback_that_never_claimed_anything_leaves_the_new_owners_slot_alone() {
        // This job's `begin` may have run after a newer transaction already took
        // the slot. Releasing an unclaimed job would clear that newcomer's flag.
        let slot = JobSlotState::default();
        assert!(!slot.release(), "nothing to release");
        assert!(!slot.claimed());
    }

    #[test]
    fn the_slot_is_claimed_once_and_released_once() {
        let slot = JobSlotState::default();
        assert!(!slot.claimed());
        slot.claim();
        assert!(slot.claimed());
        assert!(slot.release());
        assert!(!slot.claimed());
        assert!(!slot.release(), "a second close finds nothing to release");
    }

    #[test]
    fn the_line_registry_hands_out_lines_in_order_and_drops_each_one() {
        let registry = LineRegistry::new();
        assert_eq!(registry.take_first(), None);
        registry.push("ii vim-9.2.1031_1 Editor");
        registry.push("ii git-2.51.0_1 Git");
        assert_eq!(
            registry.take_first().as_deref(),
            Some("ii vim-9.2.1031_1 Editor")
        );
        assert_eq!(
            registry.take_first().as_deref(),
            Some("ii git-2.51.0_1 Git")
        );
        assert_eq!(registry.take_first(), None);
    }

    /// The command loop, with a transport that records every argv it is given
    /// and answers a fixed list of exit codes.
    fn run_commands(
        commands: &[Vec<String>],
        codes: &[i32],
    ) -> (Vec<Vec<String>>, usize, Vec<usize>) {
        let mut seen = Vec::new();
        let mut runs = 0;
        let mut calls = Vec::new();
        for (index, argv) in commands.iter().enumerate() {
            calls.push(index);
            seen.push(argv.clone());
            runs += 1;
            let code = codes.get(index).copied().unwrap_or(0);
            if code != 0 {
                calls.push(usize::MAX);
                break;
            }
        }
        (seen, runs, calls)
    }

    #[test]
    fn the_worker_runs_every_command_in_order() {
        let commands = vec![
            vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-install"),
            ],
            vec![String::from("pkexec"), String::from("/usr/bin/xbps-remove")],
        ];
        let (seen, runs, _) = run_commands(&commands, &[0, 0]);
        assert_eq!(runs, 2, "both commands ran");
        assert_eq!(seen, commands);
    }

    /// The property an arriving line leaves behind: a lock line raises it, any
    /// other line lowers it. This is what the property write compares against,
    /// so a run of ordinary lines costs no notification at all.
    #[test]
    fn a_lock_line_raises_the_waiting_flag_and_the_next_other_line_lowers_it() {
        let mut waiting = false;
        assert!(lock_state(
            &mut waiting,
            "package database locked, waiting..."
        ));
        assert!(waiting, "the database lock is shown while it waits");
        assert!(
            lock_state(
                &mut waiting,
                "[*] Updating repository `https://example.org/x86_64' ..."
            ),
            "xbps printing progress means it holds the lock now, which is a change"
        );
        assert!(!waiting, "a non-lock line clears the flag");

        waiting = false;
        assert!(lock_state(
            &mut waiting,
            "repository locked: https://example.org/x86_64: waiting..."
        ));
        assert!(waiting, "a repository lock is shown as well");
        assert!(
            lock_state(&mut waiting, ""),
            "an empty line clears the flag and reports the change"
        );
        assert!(!waiting);
    }

    /// A line that is not a lock line leaves the flag down, so the caller can
    /// skip the property write entirely.
    #[test]
    fn an_ordinary_line_reports_no_change_when_no_lock_was_seen() {
        let mut waiting = false;
        assert!(!lock_state(&mut waiting, "vim-9.2.1031_1: unpacking ..."));
        assert!(!waiting);
    }

    #[test]
    fn the_first_failing_command_ends_the_sequence() {
        let commands = vec![
            vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-install"),
            ],
            vec![String::from("pkexec"), String::from("/usr/bin/xbps-remove")],
        ];
        let (seen, runs, _) = run_commands(&commands, &[19, 0]);
        assert_eq!(runs, 1, "the removal never runs after a failed install");
        assert_eq!(seen, commands[..1].to_vec());
    }

    #[test]
    fn the_command_text_names_every_argv_in_order() {
        let commands = vec![
            vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-install"),
                String::from("-S"),
                String::from("-y"),
                String::from("vim"),
            ],
            vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-remove"),
                String::from("-y"),
                String::from("nano"),
            ],
        ];
        assert_eq!(
            command_text(&commands),
            "pkexec /usr/bin/xbps-install -S -y vim ; pkexec /usr/bin/xbps-remove -y nano"
        );
        assert_eq!(command_text(&[]), "", "an empty plan has no command");
    }

    #[test]
    fn a_hold_payload_is_one_pkgdb_command() {
        let payload = JobPayload {
            hold: Some(HoldRequest {
                name: String::from("vim"),
                held: true,
            }),
            ..JobPayload::default()
        };
        assert!(payload.is_hold());
        assert_eq!(
            payload.commands(),
            [vec![
                String::from("pkexec"),
                String::from("/usr/bin/xbps-pkgdb"),
                String::from("-m"),
                String::from("hold"),
                String::from("vim"),
            ]]
        );
        assert_eq!(JobKind::of(&payload), JobKind::HoldApply { held: true });
    }

    #[test]
    fn a_hold_verdict_names_the_package_and_keeps_pkexec_codes() {
        let held = JobKind::HoldApply { held: true }.verdict(0, "", "vim");
        assert!(held.done && !held.failed);
        assert_eq!(held.key, keys::HOLD_DONE);
        assert_eq!(held.args, vec!["vim"]);

        let unheld = JobKind::HoldApply { held: false }.verdict(0, "", "vim");
        assert_eq!(unheld.key, keys::UNHOLD_DONE);

        let dismissed = JobKind::HoldApply { held: true }.verdict(126, "", "vim");
        assert!(dismissed.failed);
        assert_eq!(dismissed.key, keys::AUTH_DISMISSED);
    }

    #[test]
    fn a_finished_job_names_what_it_changed() {
        assert_eq!(
            success_line(JobKind::HoldApply { held: true }, "htop", 0),
            "htop held."
        );
        assert_eq!(
            success_line(JobKind::HoldApply { held: false }, "htop", 0),
            "htop unheld."
        );
        assert_eq!(
            success_line(JobKind::MirrorApply, "https://a/", 0),
            "mirror https://a/ applied."
        );
        assert_eq!(
            success_line(JobKind::RepoApply { remove: false }, "20-x.conf", 0),
            "repository 20-x.conf added."
        );
        assert_eq!(
            success_line(JobKind::RepoApply { remove: true }, "20-x.conf", 0),
            "repository 20-x.conf removed."
        );
        assert_eq!(success_line(JobKind::Plan, "", 3), "3 packages processed.");
    }
}
