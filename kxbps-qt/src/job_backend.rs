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
//! The process runs on a worker thread ([`pump`]), but the window that shows
//! its output lives on the GUI thread. The worker therefore asks the GUI for an
//! event-loop pass and hands over one line per pass: a 5 000-line transaction
//! costs 5 000 short queues spread over the same number of event-loop
//! iterations, never a burst that freezes the window. The pass is requested by
//! starting the job's own 250 ms timer, which also moves the progress bar.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, QObject, Threading};
use cxx_qt_lib::{QObjectMutPtr, QString};
use kxbps_core::transaction::Outcome;
use kxbps_core::{PlanItem, TransactionPlan};
use serde::{Deserialize, Serialize};

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

/// Upper platform timer interval: anything above `i32::MAX` cannot be a
/// millisecond interval and is clamped so the conversion cannot wrap.
pub fn timer_interval(tick_ms: u32) -> i32 {
    i32::try_from(tick_ms).unwrap_or(i32::MAX)
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

        include!("backend_shim.h");

        #[cxx_name = "kxbps_create_log_model"]
        fn create_log_model(parent: QObjectMutPtr) -> QObjectMutPtr;

        #[cxx_name = "kxbps_log_model_append"]
        fn log_model_append(model: QObjectMutPtr, line: &QString);

        #[cxx_name = "kxbps_installed_apply_plan"]
        fn installed_apply_plan(owner: QObjectMutPtr, items_json: &QString);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, progress, READ, NOTIFY)]
        #[qproperty(QString, label_key, READ, NOTIFY)]
        #[qproperty(QString, label_arg, READ, NOTIFY)]
        #[qproperty(bool, done, READ, NOTIFY)]
        #[qproperty(bool, cancelled, READ, NOTIFY)]
        #[qproperty(bool, failed, READ, NOTIFY)]
        #[qproperty(i32, exit_code, READ, NOTIFY)]
        #[qproperty(bool, cancellable, READ, NOTIFY)]
        #[qproperty(QObjectMutPtr, lines, READ, CONSTANT)]
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

        /// One GUI-side pass while the job runs: appends the queued lines and
        /// moves the progress bar. Answers `false` once the job is gone.
        fn pump(self: Pin<&mut JobBackend>, elapsed_ms: i64) -> bool;

        /// Queues one line for the next [`pump`](Self::pump) pass.
        fn push_line(self: Pin<&mut JobBackend>, line: &QString);

        /// Records the final state and applies the plan on success.
        fn finish(self: Pin<&mut JobBackend>, outcome_json: &QString);
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
}

/// What the job thread needs: the plan the dialog was built from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobPayload {
    pub items: Vec<PlanItem>,
}

/// The job's final state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum End {
    /// Stopped before the process existed; nothing was applied.
    Cancelled,
    /// The process ran; `code` is its raw exit code.
    Exit { code: i32 },
}

impl End {
    /// The outcome of an exit code, or a cancel.
    pub fn of(exit_code: i32) -> Self {
        Self::Exit { code: exit_code }
    }

    /// The mapped outcome; a cancel has none.
    pub fn outcome(self) -> Option<Outcome> {
        match self {
            Self::Cancelled => None,
            Self::Exit { code } => Some(Outcome::from_exit_code(code)),
        }
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
        match end.outcome() {
            None => Self {
                done: true,
                failed: false,
                cancelled: true,
                exit_code: 0,
                key: keys::CANCELLED,
                args: Vec::new(),
            },
            Some(outcome) if outcome.success() => Self {
                done: true,
                failed: false,
                cancelled: false,
                exit_code: 0,
                key: keys::DONE,
                args: Vec::new(),
            },
            Some(outcome) => Self {
                done: true,
                failed: true,
                cancelled: false,
                exit_code: end_code(end),
                key: outcome.key(),
                args: outcome.args(),
            },
        }
    }

    /// Whether the plan reached the system and the store may be updated.
    pub fn applied(&self) -> bool {
        self.done && !self.failed && !self.cancelled
    }
}

/// The raw exit code of a finished end; `0` for a cancel, which never ran.
fn end_code(end: End) -> i32 {
    match end {
        End::Cancelled => 0,
        End::Exit { code } => code,
    }
}

/// A job the worker thread can drive without holding the QObject.
///
/// The job's thread is the only one that touches the QObject; every call here
/// queues onto it and waits for the answer, so the worker learns "the job is
/// gone" from a failed queue rather than from a dangling pointer.
pub struct Job {
    thread: cxx_qt::CxxQtThread<qobject::JobBackend>,
    /// Set by `cancel` on the GUI thread.
    cancel: Arc<AtomicBool>,
    /// Set by the worker once the process exists.
    spawned: Arc<AtomicBool>,
    /// The lines the next pass shows, filled through `line`. The receiver sits
    /// behind a mutex so the `Job` is `Sync` and can be shared through the
    /// `Arc` the transport's `'static` callbacks carry.
    lines: (Sender<String>, Mutex<Receiver<String>>),
}

impl Job {
    /// Whether the user asked to stop before the process existed.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) && !self.spawned.load(Ordering::SeqCst)
    }

    /// Takes the single-transaction slot; `false` means another job owns it
    /// and this one has already finished as `jobBusy`.
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
        // A job whose thread never answers has been destroyed; the slot stays
        // free because `begin_transaction` never ran.
        rx.recv_timeout(END_SLICE).unwrap_or(false)
    }

    /// The process exists; a later cancel is ignored.
    pub fn note_spawned(&self) {
        self.spawned.store(true, Ordering::SeqCst);
        let _ = self.thread.queue(|job| job.mark_spawned());
    }

    /// Queues one line for the next GUI pass.
    pub fn line(&self, line: &str) {
        // The channel keeps the line until the next `pump` hands it over; the
        // bound is what stops a chatty process from growing the queue.
        self.lines.0.send(line.to_owned()).ok();
    }

    /// Takes every line waiting for the next GUI pass.
    fn drain(&self) -> Vec<String> {
        let receiver = match self.lines.1.lock() {
            Ok(receiver) => receiver,
            Err(poison) => poison.into_inner(),
        };
        receiver.try_iter().collect()
    }

    /// One GUI pass; `false` once the job is gone.
    pub fn pump(&self, elapsed_ms: i64) -> bool {
        let queued = self.drain();
        let (tx, rx) = mpsc::channel();
        if self
            .thread
            .queue(move |mut job| {
                for line in &queued {
                    job.as_mut().push_line(&QString::from(line.as_str()));
                }
                tx.send(job.pump(elapsed_ms)).ok();
            })
            .is_err()
        {
            return false;
        }
        rx.recv_timeout(END_SLICE).unwrap_or(false)
    }

    /// Ends the job; the outcome travels as JSON so the shim stays dumb.
    pub fn finish(&self, end: End) {
        let queued = self.drain();
        let json =
            serde_json::to_string(&end).unwrap_or_else(|_| String::from(r#"{"kind":"cancelled"}"#));
        let _ = self.thread.queue(move |mut job| {
            for line in &queued {
                job.as_mut().push_line(&QString::from(line.as_str()));
            }
            job.finish(&QString::from(json.as_str()));
        });
    }
}

/// The worker thread: spawns the process, streams its output, ends the job.
///
/// `run` is the transport, injected so the ordering rules below are testable
/// with a fake: it receives the argv, a "may I still start?" check, a spawn
/// notification and a line callback, and returns the raw exit code. The real
/// one is [`crate::xbps_bridge::run_streaming`], whose callbacks must be
/// `'static`, so the job is shared through an `Arc` and each callback carries
/// its own clone.
pub fn pump<F>(job: Arc<Job>, argv: &[String], run: F)
where
    F: FnOnce(
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

    let started = Instant::now();
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
    let code = run(argv, may_start, spawned, line);
    // The transport reports -1 for a process killed before it ran anything;
    // that is a cancel, not a pkexec failure.
    let end = if job.cancelled() && !job.spawned.load(Ordering::SeqCst) {
        End::Cancelled
    } else {
        End::of(code)
    };
    let elapsed = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    if !job.pump(elapsed) {
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
    /// The transaction as one argv list; `None` for an empty plan.
    argv: Option<Vec<String>>,
    /// Whether the job finished: `cancel`, `pump` and `finish` are no-ops then.
    done: bool,
    /// Whether this job holds the single-transaction slot.
    owner: bool,
    progress: i32,
    label_key: QString,
    label_arg: QString,
    cancelled: bool,
    failed: bool,
    exit_code: i32,
    cancellable: bool,
    /// Lines the worker queued and the next `pump` shows.
    pending: Vec<String>,
    /// The log model; Qt owns it and destroys it with the job.
    lines: QObjectMutPtr,
    /// The store owner, as an address: `start` copies it, `finish` uses it.
    owner_addr: usize,
    /// The plan `finish` applies on success.
    items_json: String,
    /// The slot holding the cancel flag, so `cancel` can set it.
    cancel: Arc<AtomicBool>,
}

impl JobBackendRust {
    /// Builds the job from its payload; an empty plan has nothing to run.
    fn new(payload: &JobPayload) -> Self {
        let argv = TransactionPlan::for_items(&payload.items).map(|plan| plan.argv());
        Self {
            argv,
            done: false,
            owner: false,
            progress: 0,
            label_key: QString::from(""),
            label_arg: QString::from(""),
            cancelled: false,
            failed: false,
            exit_code: 0,
            cancellable: false,
            pending: Vec::new(),
            lines: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            owner_addr: 0,
            items_json: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The argv list the worker runs, or `None` for an empty literal plan.
    pub fn argv(&self) -> Option<&[String]> {
        self.argv.as_deref()
    }

    /// Whether this job holds the single-transaction slot.
    pub fn owns_slot(&self) -> bool {
        self.owner
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
            settle(
                self.as_mut(),
                &Verdict {
                    done: true,
                    failed: true,
                    cancelled: false,
                    exit_code: 0,
                    key: keys::BUSY,
                    args: Vec::new(),
                },
            );
            return false;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.owner = true;
            data.cancellable = true;
        }
        self.as_mut().cancellable_changed();
        true
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

    /// One GUI pass: shows the queued lines and moves the bar.
    ///
    /// The answer tells the worker whether another pass may be requested; a
    /// finished job answers `false` so a late queue stops.
    fn pump(mut self: Pin<&mut Self>, elapsed_ms: i64) -> bool {
        let pending = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            std::mem::take(&mut data.pending)
        };
        for line in pending {
            append_line(self.as_ref(), &line);
        }
        if !self.done && !self.cancellable {
            let progress = progress_at(elapsed_ms);
            if self.progress != progress {
                {
                    let data = Pin::get_mut(self.as_mut().rust_mut());
                    data.progress = progress;
                }
                self.as_mut().progress_changed();
            }
        }
        !self.done
    }

    /// Queues one line for the next [`Self::pump`].
    fn push_line(mut self: Pin<&mut Self>, line: &QString) {
        let line = line.to_string();
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.pending.push(line);
    }

    /// The process is gone: records the result, applies the plan on success.
    fn finish(mut self: Pin<&mut Self>, outcome_json: &QString) {
        let end: End = serde_json::from_str(&outcome_json.to_string()).unwrap_or(End::Cancelled);
        let pending = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            std::mem::take(&mut data.pending)
        };
        for line in pending {
            append_line(self.as_ref(), &line);
        }
        // A cancel that raced the spawn shows as a cancel, never as failure.
        let end = if self.done || self.cancelled {
            End::Cancelled
        } else {
            end
        };
        let verdict = Verdict::of(end);
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
            append_line(
                self.as_ref(),
                &format!("{} packages processed.", self.item_count()),
            );
        }
        settle(self.as_mut(), &verdict);
        if verdict.applied() {
            // The worker only ever queues onto the GUI thread, so this call
            // lands where the store and the models live.
            let owner = unsafe { QObjectMutPtr::from_raw(self.owner_addr as *mut QObject) };
            let items = QString::from(self.items_json.as_str());
            qobject::installed_apply_plan(owner, &items);
        }
    }

    /// Starts the privileged transaction on a worker thread.
    fn start(mut self: Pin<&mut Self>, payload_json: &cxx_qt_lib::QString, owner: QObjectMutPtr) {
        let payload: JobPayload =
            serde_json::from_str(&payload_json.to_string()).unwrap_or_default();
        let Some(argv) = self.argv().map(<[String]>::to_vec) else {
            settle(self.as_mut(), &Verdict::of(End::Cancelled));
            return;
        };
        let items_json =
            serde_json::to_string(&payload.items).unwrap_or_else(|_| String::from("[]"));

        let self_ptr = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::JobBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        let lines = qobject::create_log_model(self_ptr);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.lines = lines;
            data.owner_addr = owner.as_mut_ptr() as usize;
            data.items_json = items_json.clone();
        }

        set_label(self.as_mut(), keys::START, "");
        self.as_mut().progress_changed();

        let thread = self.qt_thread();
        let (sender, receiver) = mpsc::channel();
        let job = Arc::new(Job {
            thread,
            cancel: self.cancel_flag(),
            spawned: Arc::new(AtomicBool::new(false)),
            lines: (sender, Mutex::new(receiver)),
        });

        std::thread::spawn(move || {
            if !job.begin() {
                return;
            }
            pump(job, &argv, crate::xbps_bridge::run_streaming);
        });
    }
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
        data.label_key = QString::from(verdict.key);
        data.label_arg = QString::from(verdict.args.first().map_or("", String::as_str));
        if data.owner {
            data.owner = false;
            RUNNING.store(false, Ordering::SeqCst);
        }
    }
    job.as_mut().cancellable_changed();
    job.as_mut().label_key_changed();
    job.as_mut().label_arg_changed();
    job.as_mut().exit_code_changed();
    job.as_mut().failed_changed();
    job.as_mut().cancelled_changed();
    job.as_mut().done_changed();
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

    #[test]
    fn a_payload_becomes_the_pkexec_argv_of_its_transaction() {
        let payload = JobPayload {
            items: vec![item("vim", PlanAction::Install, None, Some("9.2.1031_1"))],
        };
        let argv = TransactionPlan::for_items(&payload.items)
            .map(|plan| plan.argv())
            .expect("a non-empty plan has a transaction");
        assert_eq!(
            argv,
            ["pkexec", "/usr/bin/xbps-install", "-y", "vim-9.2.1031_1"]
        );
        assert!(
            argv.iter()
                .all(|arg| arg != "sudo" && !arg.starts_with("sudo")),
            "escalation is polkit only: {argv:?}"
        );
    }

    #[test]
    fn an_empty_payload_has_nothing_to_run() {
        assert_eq!(
            TransactionPlan::for_items(&JobPayload::default().items),
            None
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
}
