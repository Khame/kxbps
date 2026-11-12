//! Simulated `xbps` transaction: a worker thread, a progress property and a log.
//!
//! The GUI thread never sleeps: the script below is executed on a
//! `std::thread`, and every state change is queued onto the Qt event loop with
//! `qt_thread().queue(...)`, which is the only way CXX-Qt lets a background
//! thread touch a QObject.
//!
//! The wording and the progress curve are ports of the web app's `runJob`
//! (`store.ts`): log lines are command output, so they stay English and
//! untranslated; the `label` is a translation key plus one argument.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, QObject, Threading};
use cxx_qt_lib::{QObjectMutPtr, QString};
use kxbps_core::{PlanAction, PlanItem, Repos};
use serde::{Deserialize, Serialize};

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
        #[qproperty(QObjectMutPtr, lines, READ, CONSTANT)]
        type JobBackend = super::JobBackendRust;

        #[qinvokable]
        fn cancel(self: Pin<&mut JobBackend>);

        // Called by `kxbps_start_job`; not invokable so QML cannot restart a
        // running job.
        fn start(self: Pin<&mut JobBackend>, payload_json: &QString, owner: QObjectMutPtr);
    }

    impl cxx_qt::Threading for JobBackend {}
    impl cxx_qt::Constructor<()> for JobBackend {}
}

/// Translation keys for the job label; QML maps them with `i18nd()`.
pub mod keys {
    /// Synchronising the repository index.
    pub const SYNC: &str = "jobSync";
    /// Downloading packages (`%1` = package name).
    pub const DOWNLOAD: &str = "jobDownload";
    /// Unpacking and configuring (`%1` = package name).
    pub const UNPACK: &str = "jobUnpack";
    /// Removing a package (`%1` = package name).
    pub const REMOVE: &str = "jobRemove";
    /// The transaction finished.
    pub const DONE: &str = "jobDone";
    /// The transaction was cancelled and nothing was saved.
    pub const CANCELLED: &str = "jobCancelled";
    /// The transaction failed; the log holds the reason.
    pub const FAILED: &str = "jobFailed";
}

/// What the worker thread needs: the steps, the mirror and the enabled repos.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobPayload {
    pub items: Vec<PlanItem>,
    pub mirror: String,
    pub repos: Repos,
}

/// One visible step of the simulated transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobStep {
    pub progress: i32,
    pub label_key: &'static str,
    pub label_arg: String,
    pub lines: Vec<String>,
    pub sleep_ms: u64,
}

/// Expands a plan into the same steps, lines and progress curve `runJob` uses.
///
/// `steps = items + 2`, one tick per phase plus one per package, clamped at 100
/// — the web app overshoots the same way and `Math.min(100, …)` hides it.
///
/// The second value is `true` when the mirror is not `https://`: the script
/// then stops after the sync step with one `ERROR:` line, because the same
/// plain-HTTP rule that refuses a probe must refuse a transaction.
pub fn script(items: &[PlanItem], mirror: &str, repos: Repos) -> (Vec<JobStep>, bool) {
    let mut ticker = Ticker::new(items.len() + 2);
    let root = mirror_root(mirror);
    let host = host_of(&root);
    let mut steps = Vec::new();

    let mut sync_lines = vec![format!(
        "[*] Updating repository `{root}current/x86_64-repodata' ..."
    )];
    if repos.nonfree {
        sync_lines.push(format!(
            "[*] Updating repository `{root}current/nonfree/x86_64-repodata' ..."
        ));
    }
    if repos.multilib {
        sync_lines.push(format!(
            "[*] Updating repository `{root}current/multilib/x86_64-repodata' ..."
        ));
    }
    if repos.debug {
        sync_lines.push(format!(
            "[*] Updating repository `{root}current/debug/x86_64-repodata' ..."
        ));
    }
    steps.push(JobStep {
        progress: ticker.tick(),
        label_key: keys::SYNC,
        label_arg: String::new(),
        lines: sync_lines,
        sleep_ms: 280,
    });

    if !secure_mirror(mirror) {
        steps.push(JobStep {
            progress: steps.last().map_or(0, |step| step.progress),
            label_key: keys::FAILED,
            label_arg: String::new(),
            lines: vec![format!(
                "ERROR: Repository '{root}current' is not secure; refusing to continue."
            )],
            sleep_ms: 0,
        });
        return (steps, true);
    }

    steps.push(JobStep {
        progress: ticker.tick(),
        label_key: keys::DOWNLOAD,
        label_arg: String::new(),
        lines: Vec::new(),
        sleep_ms: 0,
    });

    for item in items
        .iter()
        .filter(|item| item.action != PlanAction::Remove)
    {
        let version = item.to.as_deref().unwrap_or("");
        let sleep_ms = if item.download > 40 * 1024 * 1024 {
            420
        } else {
            180
        };
        steps.push(JobStep {
            progress: steps.last().map_or(0, |step| step.progress),
            label_key: keys::DOWNLOAD,
            label_arg: item.name.clone(),
            lines: vec![format!(
                "{}-{version}.x86_64.xbps: downloading from {host} ...",
                item.name
            )],
            sleep_ms,
        });
        steps.push(JobStep {
            progress: steps.last().map_or(0, |step| step.progress),
            label_key: keys::DOWNLOAD,
            label_arg: item.name.clone(),
            lines: vec![format!("{}-{version}.x86_64.xbps: download OK", item.name)],
            sleep_ms: 0,
        });
    }

    steps.push(JobStep {
        progress: ticker.tick(),
        label_key: keys::UNPACK,
        label_arg: String::new(),
        lines: vec![
            String::from("[*] Verifying package integrity..."),
            String::from("[*] Collecting package files..."),
        ],
        sleep_ms: 0,
    });

    for item in items {
        let last_progress = steps.last().map_or(0, |step| step.progress);
        if item.action == PlanAction::Remove {
            let from = item.from.as_deref().unwrap_or("");
            steps.push(JobStep {
                progress: last_progress,
                label_key: keys::REMOVE,
                label_arg: item.name.clone(),
                lines: vec![format!("{}-{from}: removing ...", item.name)],
                sleep_ms: 140,
            });
            steps.push(JobStep {
                progress: ticker.tick(),
                label_key: keys::REMOVE,
                label_arg: item.name.clone(),
                lines: vec![format!("{}: removed successfully.", item.name)],
                sleep_ms: 0,
            });
        } else {
            let version = item.to.as_deref().unwrap_or("");
            steps.push(JobStep {
                progress: last_progress,
                label_key: keys::UNPACK,
                label_arg: item.name.clone(),
                lines: vec![format!("{}-{version}: unpacking ...", item.name)],
                sleep_ms: 160,
            });
            steps.push(JobStep {
                progress: ticker.tick(),
                label_key: keys::UNPACK,
                label_arg: item.name.clone(),
                lines: vec![
                    format!("{}-{version}: configuring ...", item.name),
                    format!("{}-{version}: installed successfully.", item.name),
                ],
                sleep_ms: 0,
            });
        }
    }

    (steps, false)
}

/// Whether a mirror may carry a transaction: `https://` with a non-empty host.
///
/// Deliberately stricter than [`kxbps_core::check_probe_url`], which only
/// refuses plain HTTP and lets an empty string through: a probe of a missing
/// mirror is harmless, a transaction against one is not. The host check is a
/// split on `/`, not a URL parse — a mirror is a base URL, not a user-supplied
/// link, and `https:///foo` must not pass.
pub fn secure_mirror(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| !host.is_empty())
}

/// Progress curve: one tick per phase and per package, clamped at 100.
struct Ticker {
    done: i32,
    total: i32,
}

impl Ticker {
    fn new(total: usize) -> Self {
        Self {
            done: 0,
            total: i32::try_from(total).unwrap_or(i32::MAX).max(1),
        }
    }

    fn tick(&mut self) -> i32 {
        self.done += 1;
        let progress = (f64::from(self.done) / f64::from(self.total) * 100.0).round();
        progress.clamp(0.0, 100.0) as i32
    }
}

/// `https://repo-default.voidlinux.org/` from `https://repo-default.voidlinux.org`.
fn mirror_root(mirror: &str) -> String {
    format!("{}/", mirror.trim_end_matches('/'))
}

/// Host part of a URL, as the web app's `new URL(mirror).host` renders it.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or("")
}

impl cxx_qt::Constructor<()> for qobject::JobBackend {
    type BaseArguments = ();
    type NewArguments = ();
    type InitializeArguments = ();

    fn route_arguments(
        _: (),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        ((), (), ())
    }

    fn new(_: Self::NewArguments) -> JobBackendRust {
        JobBackendRust::default()
    }
}

/// Rust side of [`qobject::JobBackend`].
pub struct JobBackendRust {
    cancel: Arc<AtomicBool>,
    /// The child log model; Qt owns it and destroys it with the job.
    lines: QObjectMutPtr,
    progress: i32,
    label_key: QString,
    label_arg: QString,
    done: bool,
    cancelled: bool,
    failed: bool,
    exit_code: i32,
}

impl Default for JobBackendRust {
    /// `QObjectMutPtr` has no `Default`; the log model is attached by the C++
    /// factory before `start` runs.
    fn default() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            lines: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            progress: 0,
            label_key: QString::from(""),
            label_arg: QString::from(""),
            done: false,
            cancelled: false,
            failed: false,
            exit_code: 0,
        }
    }
}

impl qobject::JobBackend {
    /// Sets the shared cancel flag; the worker thread notices it after its
    /// current step (at most one sleep, ~420 ms) and finalises the job.
    fn cancel(mut self: Pin<&mut Self>) {
        if self.done {
            return;
        }
        self.cancel.store(true, Ordering::Relaxed);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.cancelled = true;
        }
        self.as_mut().cancelled_changed();
    }

    /// Starts the simulated transaction on a worker thread.
    fn start(mut self: Pin<&mut Self>, payload_json: &cxx_qt_lib::QString, owner: QObjectMutPtr) {
        let payload: JobPayload =
            serde_json::from_str(&payload_json.to_string()).unwrap_or_default();
        let (steps, insecure) = script(&payload.items, &payload.mirror, payload.repos);
        let total = payload.items.len();
        let items_json =
            serde_json::to_string(&payload.items).unwrap_or_else(|_| String::from("[]"));

        // The log model is a child of the job, created here because CXX-Qt
        // cannot construct a QObject from Rust.
        let self_ptr = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::JobBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        let lines = qobject::create_log_model(self_ptr);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.lines = lines;
        }

        let thread = self.qt_thread();
        let cancel = self.cancel.clone();
        // `QObjectMutPtr` wraps a raw pointer and is therefore not `Send`; the
        // address crosses the thread boundary and is turned back into a pointer
        // inside the closure that runs on the GUI thread.
        let owner_addr = owner.as_mut_ptr() as usize;

        set_label(self.as_mut(), keys::SYNC, "");
        self.as_mut().progress_changed();

        std::thread::spawn(move || {
            for step in steps {
                if cancel.load(Ordering::Relaxed) {
                    finish_cancelled(&thread);
                    return;
                }
                let sleep_ms = step.sleep_ms;
                thread
                    .queue(move |mut job| {
                        set_step(job.as_mut(), &step);
                        for line in &step.lines {
                            append_line(job.as_ref(), line);
                        }
                    })
                    .ok();
                if sleep_ms > 0 {
                    std::thread::sleep(Duration::from_millis(sleep_ms));
                }
            }

            if cancel.load(Ordering::Relaxed) {
                finish_cancelled(&thread);
                return;
            }
            thread
                .queue(move |mut job| {
                    if job.cancelled {
                        finish_cancelled_now(job.as_mut());
                        return;
                    }
                    if insecure {
                        finish_failed(job.as_mut());
                        return;
                    }
                    finish(job.as_mut(), total, &items_json, owner_addr);
                })
                .ok();
        });
    }
}

/// Writes one step's progress and label, then notifies both properties.
fn set_step(mut job: Pin<&mut qobject::JobBackend>, step: &JobStep) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.progress = step.progress;
        data.label_key = QString::from(step.label_key);
        data.label_arg = QString::from(step.label_arg.as_str());
    }
    job.as_mut().progress_changed();
    job.as_mut().label_key_changed();
    job.as_mut().label_arg_changed();
}

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

/// Final state after a cancel: nothing was applied to the store.
fn finish_cancelled_now(mut job: Pin<&mut qobject::JobBackend>) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.done = true;
        data.cancelled = true;
        // Cancellation is not a failure: the exit code stays 0.
        data.failed = false;
        data.exit_code = 0;
    }
    job.as_mut().done_changed();
    job.as_mut().cancelled_changed();
    set_label(job, keys::CANCELLED, "");
}

/// Final state after the script refused an insecure mirror: the error line is
/// already in the log, the plan is not applied, and `failed`/`exitCode` say so.
fn finish_failed(mut job: Pin<&mut qobject::JobBackend>) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.failed = true;
        data.exit_code = 1;
        data.done = true;
    }
    set_label(job.as_mut(), keys::FAILED, "");
    job.as_mut().failed_changed();
    job.as_mut().exit_code_changed();
    job.as_mut().done_changed();
}

fn finish_cancelled(thread: &cxx_qt::CxxQtThread<qobject::JobBackend>) {
    thread.queue(finish_cancelled_now).ok();
}

/// Final state after a completed transaction: 100 %, `done`, the closing log
/// lines and the store update.
fn finish(
    mut job: Pin<&mut qobject::JobBackend>,
    total: usize,
    items_json: &str,
    owner_addr: usize,
) {
    {
        let data = Pin::get_mut(job.as_mut().rust_mut());
        data.progress = 100;
        data.done = true;
        data.failed = false;
        data.exit_code = 0;
    }
    job.as_mut().progress_changed();
    job.as_mut().done_changed();
    set_label(job.as_mut(), keys::DONE, "");
    append_line(job.as_ref(), "");
    append_line(job.as_ref(), &format!("{total} packages processed."));

    // The job thread only ever queues onto the GUI thread, so this call lands
    // where the store and the models live.
    let owner = unsafe { QObjectMutPtr::from_raw(owner_addr as *mut QObject) };
    qobject::installed_apply_plan(owner, &QString::from(items_json));
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::PlanReason;

    fn item(name: &str, action: PlanAction, from: Option<&str>, to: Option<&str>) -> PlanItem {
        PlanItem {
            name: name.into(),
            action,
            from: from.map(str::to_string),
            to: to.map(str::to_string),
            reason: PlanReason::Requested,
            download: 1_024,
            size: 2_048,
        }
    }

    fn lines(steps: &[JobStep]) -> Vec<&str> {
        steps
            .iter()
            .flat_map(|step| step.lines.iter().map(String::as_str))
            .collect()
    }

    #[test]
    fn script_starts_with_the_index_sync_and_ends_with_the_last_package() {
        let plan = vec![item("vim", PlanAction::Install, None, Some("9.1.1700_1"))];
        let (steps, failed) = script(
            &plan,
            "https://repo-default.voidlinux.org",
            Repos::default(),
        );
        assert!(!failed);
        let all = lines(&steps);

        assert_eq!(
            all[0],
            "[*] Updating repository `https://repo-default.voidlinux.org/current/x86_64-repodata' ..."
        );
        assert_eq!(
            all[1],
            "vim-9.1.1700_1.x86_64.xbps: downloading from repo-default.voidlinux.org ..."
        );
        assert_eq!(all[2], "vim-9.1.1700_1.x86_64.xbps: download OK");
        assert_eq!(all[3], "[*] Verifying package integrity...");
        assert_eq!(all[4], "[*] Collecting package files...");
        assert_eq!(all[5], "vim-9.1.1700_1: unpacking ...");
        assert_eq!(all[6], "vim-9.1.1700_1: configuring ...");
        assert_eq!(all[7], "vim-9.1.1700_1: installed successfully.");
        assert_eq!(all.len(), 8);
    }

    #[test]
    fn enabled_repositories_add_their_repodata_lines() {
        let (steps, failed) = script(
            &[],
            "https://repo-default.voidlinux.org/",
            Repos {
                nonfree: true,
                multilib: true,
                debug: true,
            },
        );
        assert!(!failed);
        let all = lines(&steps);
        assert_eq!(all.len(), 6, "four repodata lines plus the verify pair");
        assert!(all[1].contains("current/nonfree/x86_64-repodata"));
        assert!(all[2].contains("current/multilib/x86_64-repodata"));
        assert!(all[3].contains("current/debug/x86_64-repodata"));
    }

    #[test]
    fn removals_have_no_download_and_use_the_installed_version() {
        let plan = vec![item("tzdata", PlanAction::Remove, Some("2024a_1"), None)];
        let (steps, failed) = script(&plan, "https://example.org/", Repos::default());
        assert!(!failed);
        let all = lines(&steps);
        let removing = all
            .iter()
            .position(|line| line.contains("removing"))
            .unwrap();
        assert_eq!(all[removing], "tzdata-2024a_1: removing ...");
        assert_eq!(all[removing + 1], "tzdata: removed successfully.");
        assert!(!all.iter().any(|line| line.contains("downloading")));
    }

    #[test]
    fn progress_reaches_one_hundred_and_never_overshoots() {
        let plan = vec![
            item("a", PlanAction::Install, None, Some("1")),
            item("b", PlanAction::Remove, Some("1"), None),
        ];
        let (steps, failed) = script(&plan, "https://example.org/", Repos::default());
        assert!(!failed);
        let progress: Vec<i32> = steps.iter().map(|step| step.progress).collect();
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(progress.last().copied(), Some(100));
        assert!(progress.iter().all(|value| (0..=100).contains(value)));
    }

    #[test]
    fn only_https_mirrors_are_secure() {
        assert!(secure_mirror("https://repo-default.voidlinux.org"));
        assert!(secure_mirror("https://repo-default.voidlinux.org/"));
        assert!(secure_mirror("https://repo-de.voidlinux.org/current"));
        assert!(!secure_mirror("http://ftp.dk.xemacs.org/voidlinux/"));
        assert!(!secure_mirror(""));
        assert!(!secure_mirror("https://"));
        assert!(!secure_mirror("https:///current"));
        assert!(!secure_mirror("ftp://repo-de.voidlinux.org/"));
    }

    #[test]
    fn an_insecure_mirror_fails_before_downloading() {
        let plan = vec![item("vim", PlanAction::Install, None, Some("9.1.1700_1"))];
        let (steps, failed) = script(
            &plan,
            "http://ftp.dk.xemacs.org/voidlinux/",
            Repos::default(),
        );
        assert!(failed, "a plain-HTTP mirror cannot carry a transaction");
        assert_eq!(steps.len(), 2, "the sync step plus the error");
        let all = lines(&steps);
        assert!(
            all[0].contains("http://ftp.dk.xemacs.org/voidlinux/current/x86_64-repodata"),
            "the sync step still runs: {}",
            all[0]
        );
        assert_eq!(
            all.last().copied(),
            Some(
                "ERROR: Repository 'http://ftp.dk.xemacs.org/voidlinux/current' is not secure; refusing to continue."
            )
        );
        assert!(!all.iter().any(|line| line.contains("downloading")));
        assert!(!all.iter().any(|line| line.contains("unpacking")));
        assert!(
            !all.iter()
                .any(|line| line.contains("installed successfully"))
        );
    }

    #[test]
    fn host_and_root_helpers_match_the_web_app() {
        assert_eq!(host_of("https://example.org/current"), "example.org");
        assert_eq!(host_of("http://example.org"), "example.org");
        assert_eq!(mirror_root("https://example.org"), "https://example.org/");
        assert_eq!(mirror_root("https://example.org/"), "https://example.org/");
    }

    #[test]
    fn payload_round_trips_through_json() {
        let payload = JobPayload {
            items: vec![item("vim", PlanAction::Install, None, Some("1"))],
            mirror: "https://example.org/".into(),
            repos: Repos {
                nonfree: true,
                ..Repos::default()
            },
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: JobPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.items, payload.items);
        assert_eq!(parsed.mirror, payload.mirror);
        assert_eq!(parsed.repos, payload.repos);
    }
}
