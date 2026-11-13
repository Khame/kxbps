//! The mirror view's backend: the model, the selection and the probe batch.
//!
//! Probing is the one part of the app that talks to the network, so it is also
//! the one part that must not run on the GUI thread. `probe_all` marks every row
//! `running`, then hands the list to a `std::thread`; that thread owns one
//! `QNetworkAccessManager` for the whole batch and walks the mirrors one at a
//! time (34 parallel requests would be rude and would measure queueing, not the
//! mirror). Every result is delivered back with `qt_thread().queue(...)`, which
//! is the only way CXX-Qt lets a background thread touch a QObject, and each one
//! updates a single row.
//!
//! Two rules come from the core, not from here: [`check_probe_url`] refuses a
//! plain-HTTP mirror *before* any request is made, and the request budget is
//! [`PROBE_TIMEOUT_MS`]. A refused mirror therefore never reaches the network
//! and lands in the `fail` state with the `plainHttp` key.
//!
//! The selected mirror is persisted by [`crate::installed_backend`], the single
//! owner of the [`Store`](kxbps_core::Store); this backend reads it at startup
//! and writes through the store on every change, so a restart remembers it.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use cxx_qt::casting::Upcast;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QObjectMutPtr, QString, QStringList};
use kxbps_core::mirror_config::{SYSTEM_CONFIG_PATH, check_base_url};
use kxbps_core::{DEFAULT_MIRROR, MIRRORS, PROBE_TIMEOUT_MS, ProbeError, check_probe_url, measure};

use crate::mirror_model::{STATE_FAIL, STATE_OK, STATE_RUNNING};
use crate::mirror_system::{SystemState, production_invoke, read_system_state};

/// The one config file xbps' `current` repository comes from.
///
/// `mirror_config` owns the parser; this constant only names the file the
/// backend reads so a test can point it at a temporary one.
pub const SYSTEM_CONFIG: &str = SYSTEM_CONFIG_PATH;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        // Model factory, per-row updates and the `HEAD` transport.
        include!("mirror_shim.h");

        #[cxx_name = "kxbps_create_mirror_model"]
        fn create_mirror_model(parent: QObjectMutPtr) -> QObjectMutPtr;

        #[cxx_name = "kxbps_mirror_model_set_all_running"]
        fn mirror_model_set_all_running(model: QObjectMutPtr);

        #[cxx_name = "kxbps_mirror_model_finish_running"]
        fn mirror_model_finish_running(model: QObjectMutPtr);

        #[cxx_name = "kxbps_mirror_model_set_result"]
        fn mirror_model_set_result(
            model: QObjectMutPtr,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        #[cxx_name = "kxbps_probe_batch_begin"]
        fn probe_batch_begin() -> usize;

        #[cxx_name = "kxbps_probe_batch_end"]
        fn probe_batch_end(handle: usize);

        #[cxx_name = "kxbps_probe_head"]
        fn probe_head(handle: usize, url: &QString, timeout_ms: u32) -> i32;

        // True once the application is shutting down; the batch polls it between
        // mirrors and inside a request, so closing the window stops the sweep.
        #[cxx_name = "kxbps_probe_shutdown_requested"]
        fn probe_shutdown_requested() -> bool;

        // The store lives in the installed backend; the mirror is read and
        // written through it so there is only ever one `Store`.
        include!("backend_shim.h");

        #[cxx_name = "kxbps_installed_mirror"]
        fn installed_mirror(owner: QObjectMutPtr) -> QString;

        #[cxx_name = "kxbps_installed_set_mirror"]
        fn installed_set_mirror(owner: QObjectMutPtr, url: &QString);

        // The one entry point a mirror job is created through; `parent` owns it
        // until QML releases it. A distinct shim name, not `kxbps_start_job`:
        // two bridges declaring the same C++ function would emit the same
        // `cxxbridge1$…` thunk twice and the linker refuses the second.
        #[cxx_name = "kxbps_start_mirror_job"]
        fn start_mirror_job(owner: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        // Tells the store owner which mirror backend is waiting for the next
        // apply job to end; the owner forwards the end back through
        // `kxbps_mirror_apply_landed`.
        #[cxx_name = "kxbps_installed_watch_mirror_job"]
        fn installed_watch_mirror_job(owner: QObjectMutPtr, backend: QObjectMutPtr);

        // Detaching from an owner also forgets this backend as the owner's
        // waiting mirror job, so a job that lands after the page is gone has no
        // address to call. Null-safe on the C++ side.
        #[cxx_name = "kxbps_installed_unwatch_mirror_job"]
        fn installed_unwatch_mirror_job(owner: QObjectMutPtr);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, model, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, owner, READ, WRITE = set_owner, NOTIFY)]
        #[qproperty(QString, current_mirror, READ, WRITE = set_current_mirror, NOTIFY)]
        #[qproperty(bool, probing, READ, NOTIFY)]
        #[qproperty(i32, finished_count, READ, NOTIFY)]
        /// The key of the last refused selection; empty when the last one was
        /// accepted. QML maps it with `i18nd()` and shows it next to the list.
        #[qproperty(QString, current_mirror_error, READ, NOTIFY)]
        /// The base URL `00-repository-main.conf` names; empty when unreadable.
        #[qproperty(QString, system_mirror, READ, NOTIFY)]
        /// `errorMirrorConfig*` key when the file is unreadable; empty otherwise.
        #[qproperty(QString, system_mirror_error, READ, NOTIFY)]
        /// The arguments of `systemMirrorError`, in `%1…` order.
        #[qproperty(QStringList, system_mirror_error_args, READ, NOTIFY)]
        /// `xbps-query -L` plus the `.conf` files, as a JSON array of
        /// `{url, count, signed, file}`.
        #[qproperty(QString, system_repos, READ, NOTIFY)]
        /// The selected mirror is not the system one.
        #[qproperty(bool, pending, READ, NOTIFY)]
        /// A system read is in flight.
        #[qproperty(bool, system_loading, READ, NOTIFY)]
        /// An apply job is running: the slot refuses a second one, and the
        /// button must say so before the round trip.
        #[qproperty(bool, apply_busy, READ, NOTIFY)]
        /// Whether the privileged helper and its polkit action are installed.
        /// Read once at startup: installing them needs a restart to take
        /// effect, and a file check on every button click would only race the
        /// package manager.
        #[qproperty(bool, helper_ready, READ, CONSTANT)]
        /// The `errorMirrorHelper*` key when `helperReady` is false; empty
        /// otherwise. QML turns it into the sentence next to the button.
        #[qproperty(QString, helper_error_key, READ, CONSTANT)]
        /// The repository helper and its policy are both installed. The two
        /// helpers ship together, but the check names the one the page runs.
        #[qproperty(bool, repo_helper_ready, READ, CONSTANT)]
        #[qproperty(QString, repo_helper_error_key, READ, CONSTANT)]
        type MirrorBackend = super::MirrorBackendRust;

        #[qinvokable]
        fn probe_all(self: Pin<&mut MirrorBackend>);

        #[qinvokable]
        fn probe_one(self: Pin<&mut MirrorBackend>, url: &QString);

        /// Re-reads `00-repository-main.conf` and `xbps-query -L` without root.
        #[qinvokable]
        fn refresh_system(self: Pin<&mut MirrorBackend>);

        /// Starts the privileged apply of the selected mirror and returns the
        /// job; the caller (QML) shows it and `release()`s it when done.
        ///
        /// Returns a null pointer when there is nothing to apply, when the URL
        /// is one the helper would refuse, or when an apply is already running.
        #[qinvokable]
        fn apply_system_mirror(self: Pin<&mut MirrorBackend>) -> QObjectMutPtr;

        /// Adds one repository through the privileged helper. Answers with the
        /// job, or null when the value is empty or another privileged job runs.
        #[qinvokable]
        fn add_repository(self: Pin<&mut MirrorBackend>, url: &QString) -> QObjectMutPtr;

        /// Removes one repository file through the privileged helper.
        #[qinvokable]
        fn remove_repository(self: Pin<&mut MirrorBackend>, file: &QString) -> QObjectMutPtr;

        /// The apply this backend started has ended, whatever its outcome.
        ///
        /// Called by the store owner on the GUI thread; not invokable, so QML
        /// cannot claim a job is over while it runs. Re-reads the system half,
        /// so `systemMirror` and `pending` show what the helper left behind.
        fn finish_apply(self: Pin<&mut MirrorBackend>);

        /// Emitted once per batch, after the last row has its result.
        #[qsignal]
        fn probe_finished(self: Pin<&mut MirrorBackend>);

        fn set_owner(self: Pin<&mut MirrorBackend>, value: QObjectMutPtr);

        fn set_current_mirror(self: Pin<&mut MirrorBackend>, value: QString);

        // Called from the queued closures below; not invokable, because QML must
        // not be able to write a result the transport did not produce. The
        // result crosses the bridge as its three wire values, since a CXX-Qt
        // method signature can only name CXX types.
        fn apply_probe_result(
            self: Pin<&mut MirrorBackend>,
            row: i32,
            batch: u64,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        fn apply_single_result(
            self: Pin<&mut MirrorBackend>,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        // Ends a batch that stopped early (shutdown, teardown or a newer batch):
        // re-enables probing and returns the unanswered rows to `idle`.
        fn finish_cancelled(self: Pin<&mut MirrorBackend>, batch: u64);
    }

    impl cxx_qt::Constructor<()> for MirrorBackend {}
    impl cxx_qt::Threading for MirrorBackend {}
}

/// One probe's wire result: the three values the model and the counters need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// `ok` or `fail`.
    pub state_key: &'static str,
    /// Milliseconds for an `ok` outcome, `0` otherwise.
    pub latency_ms: u32,
    /// Core probe key for a `fail` outcome, `""` otherwise.
    pub error_key: &'static str,
}

impl ProbeOutcome {
    /// Maps the transport's return code onto a state; `None` when the request
    /// was abandoned because the application is shutting down.
    ///
    /// `0` — the mirror answered, `1` — the timer won, `2` — the request never
    /// reached an answer (DNS, TLS, refused), `3` — cancelled, so there is no
    /// result to show.
    pub fn classify(code: i32, latency_ms: u32) -> Option<Self> {
        match code {
            0 => Some(Self {
                state_key: STATE_OK,
                latency_ms,
                error_key: "",
            }),
            1 => Some(Self {
                state_key: STATE_FAIL,
                latency_ms: 0,
                error_key: ProbeError::Timeout.key(),
            }),
            3 => None,
            _ => Some(Self {
                state_key: STATE_FAIL,
                latency_ms: 0,
                error_key: ProbeError::NoResponse.key(),
            }),
        }
    }

    /// The outcome of a mirror that never gets a request: the core rule says a
    /// plain-HTTP URL is refused before the network is touched.
    pub fn refused(error: ProbeError) -> Self {
        Self {
            state_key: STATE_FAIL,
            latency_ms: 0,
            error_key: error.key(),
        }
    }
}

/// The apply slot's busy flag, as a value so both transitions are testable
/// without a QObject.
///
/// A `pkexec` apply is one job at a time: the slot refuses a second one, so the
/// flag must move on the first job only, and a refused creation must not turn
/// it on. Every method returns whether the flag moved, which is exactly when the
/// property signal is emitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyState {
    busy: bool,
}

impl ApplyState {
    /// A job was created (`created`) or refused (`!created`).
    pub fn start(&mut self, created: bool) -> bool {
        if !created || self.busy {
            return false;
        }
        self.busy = true;
        true
    }

    /// The job ended, whatever its outcome.
    pub fn finish(&mut self) -> bool {
        let was_busy = self.busy;
        self.busy = false;
        was_busy
    }

    pub const fn is_busy(&self) -> bool {
        self.busy
    }
}

/// Rust side of [`qobject::MirrorBackend`].
pub struct MirrorBackendRust {
    /// Child model created in `initialize`; Qt owns it.
    model: QObjectMutPtr,
    /// The installed backend, i.e. the owner of the store.
    owner: QObjectMutPtr,
    current_mirror: QString,
    probing: bool,
    finished_count: i32,
    /// Bumped by every `probe_all`. A result from an older batch is dropped, so
    /// a second click cannot interleave two batches into one row set.
    batch: Arc<AtomicU64>,
    /// Stops the running batch when this backend is detached from its store
    /// owner, i.e. when the page that owns it is being torn down. A separate
    /// flag from the application-wide shutdown, which other probes share.
    cancel: Arc<AtomicBool>,
    /// The system half: what `/etc/xbps.d` and `xbps-query -L` say.
    system: SystemState,
    /// QML copies of [`SystemState`], because a `#[qproperty]` reads a struct
    /// field: `system` stays the single source, these are refreshed from it.
    system_mirror: QString,
    system_mirror_error: QString,
    system_mirror_error_args: QStringList,
    system_repos: QString,
    /// The saved selection is not what xbps uses; see [`SystemState::pending`].
    pending: bool,
    current_mirror_error: QString,
    system_loading: bool,
    /// The apply slot: one job at a time.
    apply_state: ApplyState,
    /// `apply_state.is_busy()`, because a `#[qproperty]` reads a plain field.
    apply_busy: bool,
    /// The helper and its polkit action are both on disk; see
    /// [`helper_state`]. Read once, because the answer cannot change while
    /// this process runs without a reinstall.
    helper_ready: bool,
    /// The key for the reason `helper_ready` is false; empty when it is true.
    helper_error_key: QString,
    /// The repository helper and the same policy; see [`helper_state`].
    repo_helper_ready: bool,
    /// The key for the reason `repo_helper_ready` is false.
    repo_helper_error_key: QString,
    /// How many system reads were started; only the newest may write.
    system_generation: Arc<AtomicU64>,
    /// The transport the system read uses; tests install a fake.
    invoke: crate::xbps_snapshot::SharedInvoke,
    /// The config file the system read parses; tests point it elsewhere.
    config_path: std::path::PathBuf,
}

impl Default for MirrorBackendRust {
    /// Only used before `initialize` runs; the model stays null until then.
    fn default() -> Self {
        let helper = helper_state(Path::new(HELPER_PATH), Path::new(POLICY_PATH));
        let repo_helper = helper_state(Path::new(REPO_HELPER_PATH), Path::new(POLICY_PATH));
        Self {
            model: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            owner: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            current_mirror: QString::from(""),
            probing: false,
            finished_count: 0,
            batch: Arc::new(AtomicU64::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            system: SystemState::default(),
            system_mirror: QString::from(""),
            system_mirror_error: QString::from(""),
            system_mirror_error_args: QStringList::default(),
            system_repos: QString::from("[]"),
            pending: false,
            current_mirror_error: QString::from(""),
            system_loading: false,
            apply_state: ApplyState::default(),
            apply_busy: false,
            helper_ready: helper.ready(),
            helper_error_key: QString::from(helper.key()),
            repo_helper_ready: repo_helper.ready(),
            repo_helper_error_key: QString::from(repo_helper.key()),
            system_generation: Arc::new(AtomicU64::new(0)),
            invoke: production_invoke(),
            config_path: std::path::PathBuf::from(SYSTEM_CONFIG),
        }
    }
}

/// Copies the system state and the selection into the QML-visible fields.
///
/// [`MirrorBackendRust::system`] and `current_mirror` stay the single source of
/// truth; these fields exist only because a `#[qproperty]` reads a struct
/// field. Every writer of either input calls this before it emits the change
/// signals, so QML never sees a value the store does not have.
fn adopt_system(data: &mut MirrorBackendRust) {
    data.system_mirror = QString::from(data.system.system_mirror.as_str());
    data.system_mirror_error = QString::from(data.system.error_key);
    data.system_mirror_error_args = data
        .system
        .error_args
        .iter()
        .map(|arg| QString::from(arg.as_str()))
        .collect::<QStringList>();
    data.system_repos = QString::from(data.system.repos_json.as_str());
    data.pending = data.system.pending(&data.current_mirror.to_string());
}

impl cxx_qt::Initialize for qobject::MirrorBackend {
    /// Creates the model as a child of this backend, fully populated before QML
    /// can bind to it.
    fn initialize(mut self: Pin<&mut Self>) {
        let parent = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::MirrorBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        let model = qobject::create_mirror_model(parent);
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.model = model;
    }
}

impl qobject::MirrorBackend {
    /// Probes every mirror on a worker thread.
    ///
    /// The GUI thread only marks the rows and starts the thread; results arrive
    /// through `qt_thread().queue(...)` one row at a time, so a QML timer keeps
    /// firing and the view keeps scrolling while the batch runs.
    ///
    /// The sweep stops early — leaving the remaining rows untouched — when a
    /// newer batch takes over, when the backend has been destroyed, or when the
    /// application is shutting down. The last case matters: a full sweep takes
    /// ~20 s, and a window closed mid-batch must not keep a worker and its
    /// network manager alive behind a dead event loop.
    fn probe_all(mut self: Pin<&mut Self>) {
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        let batch = self.batch.fetch_add(1, Ordering::SeqCst) + 1;
        // A batch started after a re-attach must be allowed to run.
        self.cancel.store(false, Ordering::Relaxed);

        qobject::mirror_model_set_all_running(model);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.probing = true;
            data.finished_count = 0;
        }
        self.as_mut().probing_changed();
        self.as_mut().finished_count_changed();

        let thread = self.qt_thread();
        let generation = self.batch.clone();
        let cancel = self.cancel.clone();
        let targets = row_targets();

        std::thread::spawn(move || {
            // One manager for the batch, created and destroyed on this thread.
            let handle = qobject::probe_batch_begin();
            let mut stopped = false;
            for (row, url) in targets {
                if generation.load(Ordering::SeqCst) != batch
                    || thread.is_destroyed()
                    || cancel.load(Ordering::Relaxed)
                    || qobject::probe_shutdown_requested()
                {
                    // A newer batch owns the rows, this backend was detached
                    // from its store, or the app is going away: stop asking the
                    // network.
                    stopped = true;
                    break;
                }
                let Some(outcome) = probe_url(handle, url) else {
                    // Shutdown cancelled the request in flight.
                    stopped = true;
                    break;
                };
                thread
                    .queue(move |backend| {
                        backend.apply_probe_result(
                            row,
                            batch,
                            &QString::from(outcome.state_key),
                            outcome.latency_ms,
                            &QString::from(outcome.error_key),
                        );
                    })
                    .ok();
            }
            qobject::probe_batch_end(handle);
            if stopped {
                // Clears `probing` (the button must not stay disabled) and puts
                // the unanswered rows back to `idle`; a no-op when a newer batch
                // already owns the rows.
                thread
                    .queue(move |backend| backend.finish_cancelled(batch))
                    .ok();
            }
        });
    }

    /// Probes one mirror, for a row's own "refresh" action.
    ///
    /// It never touches the batch counters: `finishedCount` describes a batch,
    /// and a single probe that ran next to one must not move it.
    fn probe_one(self: Pin<&mut Self>, url: &cxx_qt_lib::QString) {
        let url = url.to_string();
        let Some(row) = row_of(&url) else {
            // Not a listed mirror: there is no row to report into.
            return;
        };
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        qobject::mirror_model_set_result(
            model,
            row,
            &QString::from(STATE_RUNNING),
            0,
            &QString::from(""),
        );

        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let handle = qobject::probe_batch_begin();
            let outcome = probe_url(handle, &url);
            qobject::probe_batch_end(handle);
            // A cancelled request reports no result: put the row back to `idle`
            // instead of leaving it `running` forever.
            let outcome = outcome.unwrap_or(ProbeOutcome {
                state_key: crate::mirror_model::STATE_IDLE,
                latency_ms: 0,
                error_key: "",
            });
            thread
                .queue(move |backend| {
                    backend.apply_single_result(
                        row,
                        &QString::from(outcome.state_key),
                        outcome.latency_ms,
                        &QString::from(outcome.error_key),
                    );
                })
                .ok();
        });
    }

    /// Points the backend at the store owner and adopts its saved mirror.
    ///
    /// Detaching (a null owner) is the teardown path of the page that owns this
    /// backend: the running sweep stops instead of holding a socket open for a
    /// view that is going away, the old owner forgets this backend as its
    /// waiting mirror job (it holds the address, so the landing must not call a
    /// backend that no longer exists), and the last selection is kept for
    /// whoever binds next.
    fn set_owner(mut self: Pin<&mut Self>, value: QObjectMutPtr) {
        if value.as_ptr().is_null() {
            let old_owner = {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                data.cancel.store(true, Ordering::Relaxed);
                std::mem::replace(&mut data.owner, value)
            };
            // The *old* owner is the one that recorded this backend; tell it to
            // forget, before anything else can queue a landing on it.
            if !old_owner.as_ptr().is_null() {
                qobject::installed_unwatch_mirror_job(old_owner);
            }
            self.as_mut().owner_changed();
            return;
        }
        let saved = qobject::installed_mirror(value);
        let saved_text = saved.to_string();
        let current = if saved_text.is_empty() {
            // A store without a mirror means the default; write it back so the
            // next start does not have to guess again.
            qobject::installed_set_mirror(value, &QString::from(DEFAULT_MIRROR));
            QString::from(DEFAULT_MIRROR)
        } else {
            saved
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.owner = value;
            data.current_mirror = current;
            data.cancel.store(false, Ordering::Relaxed);
            adopt_system(data);
        }
        self.as_mut().owner_changed();
        self.as_mut().current_mirror_changed();
        self.as_mut().pending_changed();
    }

    /// Selects a mirror and persists it through the store.
    fn set_current_mirror(mut self: Pin<&mut Self>, value: QString) {
        let url = value.to_string();
        if url.is_empty() || self.current_mirror.to_string() == url {
            return;
        }
        if !is_selectable_mirror(&url) {
            // The helper answers a URL like this with code 2, and the store must
            // not remember a selection xbps could never use.
            {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                data.current_mirror_error = QString::from("errorMirrorUrlRefused");
            }
            self.as_mut().current_mirror_error_changed();
            return;
        }
        let owner = self.owner;
        if !owner.as_ptr().is_null() {
            qobject::installed_set_mirror(owner, &value);
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.current_mirror = value;
            data.current_mirror_error = QString::from("");
            adopt_system(data);
        }
        self.as_mut().current_mirror_changed();
        self.as_mut().current_mirror_error_changed();
        self.as_mut().pending_changed();
    }

    /// Re-reads the system half on a worker thread.
    ///
    /// No root: `/etc/xbps.d` is world-readable and `xbps-query -L` only reads
    /// the local database, so one read costs nothing and can run every time the
    /// page opens or a job ends. Only the newest read may write, so a slow one
    /// cannot overwrite a fresher answer.
    fn refresh_system(mut self: Pin<&mut Self>) {
        let generation = self.system_generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.system_loading = true;
        }
        self.as_mut().system_loading_changed();

        let thread = self.qt_thread();
        let current = Arc::clone(&self.system_generation);
        let invoke = Arc::clone(&self.invoke);
        let config_path = self.config_path.clone();
        std::thread::spawn(move || {
            let state = read_system_state(&config_path, &*invoke);
            thread
                .queue(move |backend| backend.finish_system(generation, &current, state))
                .ok();
        });
    }

    /// Writes one system read, unless a newer one owns the state.
    fn finish_system(
        mut self: Pin<&mut Self>,
        generation: u64,
        current: &Arc<AtomicU64>,
        state: SystemState,
    ) {
        if current.load(Ordering::SeqCst) != generation {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.system = state;
            data.system_loading = false;
            adopt_system(data);
        }
        self.as_mut().system_mirror_changed();
        self.as_mut().system_mirror_error_changed();
        self.as_mut().system_mirror_error_args_changed();
        self.as_mut().system_repos_changed();
        self.as_mut().system_loading_changed();
        self.as_mut().pending_changed();
    }

    /// Starts the privileged apply of the selected mirror.
    ///
    /// The JobBackend it returns is a child of the store owner and QML owns the
    /// dialog: the caller shows it with `JobDialog.showJob` and `release()`s it
    /// when the dialog closes. A null pointer means there was nothing to apply
    /// or the URL is one the helper would refuse with code 2.
    ///
    /// The single-transaction slot is the busy guard: a second Apply during a
    /// job comes back as `jobBusy` from the slot, and the mirror backend tells
    /// the store owner about the job it started so `finishApply` can clear its
    /// state and re-read the system half when the job ends.
    fn apply_system_mirror(mut self: Pin<&mut Self>) -> QObjectMutPtr {
        let selected = self.current_mirror.to_string();
        if selected.is_empty()
            || !is_selectable_mirror(&selected)
            || !self.system.pending(&selected)
        {
            return unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) };
        }
        let owner = self.owner;
        if owner.as_ptr().is_null() {
            return unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) };
        }
        let payload = crate::job_backend::mirror_payload_json(&selected);
        let job = qobject::start_mirror_job(owner, &QString::from(payload.as_str()));
        let moved = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let moved = data.apply_state.start(!job.as_ptr().is_null());
            data.apply_busy = data.apply_state.is_busy();
            moved
        };
        if moved {
            self.as_mut().apply_busy_changed();
        }
        if job.as_ptr().is_null() {
            return job;
        }
        // A job is created with its owner and has no other way back to this
        // backend: the owner forwards the end to `finishApply`. The pointer is
        // the QObject CXX-Qt generated for this Rust struct.
        let backend = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::MirrorBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        qobject::installed_watch_mirror_job(owner, backend);
        job
    }

    fn add_repository(mut self: Pin<&mut Self>, url: &cxx_qt_lib::QString) -> QObjectMutPtr {
        let value = url.to_string();
        self.as_mut().start_repo_job("add", &value)
    }

    fn remove_repository(mut self: Pin<&mut Self>, file: &cxx_qt_lib::QString) -> QObjectMutPtr {
        let value = file.to_string();
        self.as_mut().start_repo_job("remove", &value)
    }

    /// One privileged repository edit, through the same job slot as a mirror
    /// apply: the helper writes `/etc/xbps.d`, so two of them must not run at
    /// once, and the job's landing re-reads the system half either way.
    fn start_repo_job(mut self: Pin<&mut Self>, verb: &str, value: &str) -> QObjectMutPtr {
        let owner = self.owner;
        if value.is_empty() || owner.as_ptr().is_null() {
            return unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) };
        }
        let payload = crate::job_backend::repo_payload_json(verb, value);
        let job = qobject::start_mirror_job(owner, &QString::from(payload.as_str()));
        let moved = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let moved = data.apply_state.start(!job.as_ptr().is_null());
            data.apply_busy = data.apply_state.is_busy();
            moved
        };
        if moved {
            self.as_mut().apply_busy_changed();
        }
        if job.as_ptr().is_null() {
            return job;
        }
        let backend = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::MirrorBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        qobject::installed_watch_mirror_job(owner, backend);
        job
    }

    /// Clears the apply state and re-reads the system half.
    ///
    /// Called on the GUI thread when the job this backend started has ended, by
    /// the store owner's forwarder, whatever the outcome. Whatever the helper
    /// left in `/etc/xbps.d` is what `systemMirror` and `pending` must show, so
    /// the read runs here rather than in QML. The busy flag moves here too, so
    /// a failed apply does not leave the button disabled.
    fn finish_apply(mut self: Pin<&mut Self>) {
        let moved = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let moved = data.apply_state.finish();
            data.apply_busy = data.apply_state.is_busy();
            moved
        };
        if moved {
            self.as_mut().apply_busy_changed();
        }
        self.as_mut().refresh_system();
    }

    /// Applies one result of the current batch, then advances the counter.
    fn apply_probe_result(
        mut self: Pin<&mut Self>,
        row: i32,
        batch: u64,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        if self.batch.load(Ordering::SeqCst) != batch {
            return;
        }
        self.write_row(row, state_key, latency_ms, error_key);

        let total = i32::try_from(MIRRORS.len()).unwrap_or(i32::MAX);
        let done = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.finished_count = data.finished_count.saturating_add(1);
            let done = data.finished_count >= total;
            if done {
                data.probing = false;
            }
            done
        };
        self.as_mut().finished_count_changed();
        if done {
            self.as_mut().probing_changed();
            self.as_mut().probe_finished();
        }
    }

    /// Applies the result of a single-mirror probe; counters are untouched.
    fn apply_single_result(
        self: Pin<&mut Self>,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        self.write_row(row, state_key, latency_ms, error_key);
    }

    /// Ends a batch that stopped before its last row (shutdown, teardown, or a
    /// newer batch).
    ///
    /// The generation guard keeps ownership honest: only the batch that is still
    /// current resets `probing` and returns its unanswered rows to `idle`, so a
    /// superseded batch cannot re-enable a button the new batch disabled.
    fn finish_cancelled(mut self: Pin<&mut Self>, batch: u64) {
        if self.batch.load(Ordering::SeqCst) != batch {
            return;
        }
        let model = self.model;
        if !model.as_ptr().is_null() {
            qobject::mirror_model_finish_running(model);
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            if !data.probing {
                return;
            }
            data.probing = false;
        }
        self.as_mut().probing_changed();
        self.as_mut().probe_finished();
    }

    /// Writes one row into the model, if the model exists.
    fn write_row(
        &self,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        let model = self.model;
        if model.as_ptr().is_null() {
            return;
        }
        qobject::mirror_model_set_result(model, row, state_key, latency_ms, error_key);
    }
}

/// Row index of a mirror URL, `None` when the URL is not in the list.
fn row_of(url: &str) -> Option<i32> {
    MIRRORS
        .iter()
        .position(|mirror| mirror.url == url)
        .and_then(|row| i32::try_from(row).ok())
}

/// Whether `url` may be selected, and later applied, as the system mirror.
///
/// Two shapes are accepted: one of the catalog entries in [`MIRRORS`], or any
/// base URL the helper would accept — absolute `https://`, a host, one trailing
/// `/`, no `/current` in it. Plain HTTP, a missing scheme and a URL without a
/// trailing slash are refused here too, because the helper answers them with
/// code 2 and the UI must not offer what the helper refuses.
pub fn is_selectable_mirror(url: &str) -> bool {
    MIRRORS.iter().any(|mirror| mirror.url == url) || check_base_url(url).is_ok()
}

/// Every row with the URL it probes; row `i` is `MIRRORS[i]` by construction.
fn row_targets() -> Vec<(i32, &'static str)> {
    MIRRORS
        .iter()
        .enumerate()
        .filter_map(|(row, mirror)| Some((i32::try_from(row).ok()?, mirror.url)))
        .collect()
}

/// Probes one URL: the core rule first, then the transport, then [`measure`].
///
/// Runs on the worker thread. The returned outcome carries keys only; `None`
/// means the application began shutting down and the request was abandoned, so
/// there is nothing to report for that row.
fn probe_url(handle: usize, url: &str) -> Option<ProbeOutcome> {
    if let Err(error) = check_probe_url(url) {
        // Never requested: the row says why instead.
        return Some(ProbeOutcome::refused(error));
    }
    let start = Instant::now();
    let code = qobject::probe_head(
        handle,
        &QString::from(url),
        u32::try_from(PROBE_TIMEOUT_MS).unwrap_or(u32::MAX),
    );
    ProbeOutcome::classify(code, measure(start))
}

/// The installed helper and the polkit action that authorizes it.
///
/// `packaging/install.sh` writes both; the policy's `exec.path` names the same
/// helper path. The check lives here, not in QML, because a missing helper is a
/// fact about the system and the button has to say why it is disabled before
/// `pkexec` fails with a code nobody can act on.
pub const HELPER_PATH: &str = "/usr/libexec/kxbps-mirror-apply";

/// The repository helper polkit runs for the Repositories page.
pub const REPO_HELPER_PATH: &str = "/usr/libexec/kxbps-repo-apply";
pub const POLICY_PATH: &str = "/usr/share/polkit-1/actions/io.github.khame.kxbps.policy";

/// What the privileged half of "Apply to system" is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperState {
    /// Helper and policy are both installed; an apply may ask for a password.
    Ready,
    /// No helper at [`HELPER_PATH`].
    MissingHelper,
    /// The helper exists but cannot be executed.
    NotExecutable,
    /// The helper exists, but polkit has no action to authorize it.
    MissingPolicy,
}

impl HelperState {
    /// True only when an apply can be attempted.
    pub fn ready(self) -> bool {
        self == Self::Ready
    }

    /// The translation key for the reason; empty when ready.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::MissingHelper => "errorMirrorHelperMissing",
            Self::NotExecutable => "errorMirrorHelperNotExecutable",
            Self::MissingPolicy => "errorMirrorPolicyMissing",
        }
    }
}

/// Checks the two files an apply needs. Any unreadable helper path counts as
/// missing: a permission error on `/usr/libexec` is the same dead end for the
/// user as an absent file.
pub fn helper_state(helper: &Path, policy: &Path) -> HelperState {
    let metadata = match std::fs::metadata(helper) {
        Ok(metadata) => metadata,
        Err(_) => return HelperState::MissingHelper,
    };
    if !metadata.is_file() {
        return HelperState::MissingHelper;
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return HelperState::NotExecutable;
    }
    if !policy.is_file() {
        return HelperState::MissingPolicy;
    }
    HelperState::Ready
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kxbps-helper-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write(path: &Path, executable: bool) {
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").expect("write");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(if executable { 0o755 } else { 0o644 });
        std::fs::set_permissions(path, permissions).expect("chmod");
    }

    #[test]
    fn a_ready_helper_needs_both_files_and_the_exec_bit() {
        let dir = temp_dir("ready");
        let helper = dir.join("kxbps-mirror-apply");
        let policy = dir.join("kxbps.policy");
        write(&helper, true);
        write(&policy, false);
        assert_eq!(helper_state(&helper, &policy), HelperState::Ready);
        assert!(helper_state(&helper, &policy).ready());
    }

    #[test]
    fn a_missing_helper_is_named_as_such() {
        let dir = temp_dir("missing");
        let policy = dir.join("kxbps.policy");
        write(&policy, false);
        assert_eq!(
            helper_state(&dir.join("absent"), &policy),
            HelperState::MissingHelper
        );
    }

    #[test]
    fn a_helper_without_the_exec_bit_is_not_ready() {
        let dir = temp_dir("noexec");
        let helper = dir.join("kxbps-mirror-apply");
        let policy = dir.join("kxbps.policy");
        write(&helper, false);
        write(&policy, false);
        assert_eq!(helper_state(&helper, &policy), HelperState::NotExecutable);
    }

    #[test]
    fn a_missing_policy_is_named_as_such() {
        let dir = temp_dir("nopolicy");
        let helper = dir.join("kxbps-mirror-apply");
        write(&helper, true);
        assert_eq!(
            helper_state(&helper, &dir.join("absent.policy")),
            HelperState::MissingPolicy
        );
    }

    #[test]
    fn only_ready_has_no_message() {
        assert_eq!(HelperState::Ready.key(), "");
        for state in [
            HelperState::MissingHelper,
            HelperState::NotExecutable,
            HelperState::MissingPolicy,
        ] {
            assert!(!state.key().is_empty());
            assert!(!state.ready());
        }
    }

    #[test]
    fn an_answer_is_ok_with_its_latency() {
        let outcome = ProbeOutcome::classify(0, 178).expect("an answer is a result");
        assert_eq!(outcome.state_key, "ok");
        assert_eq!(outcome.latency_ms, 178);
        assert_eq!(outcome.error_key, "");
    }

    #[test]
    fn the_timer_and_a_dead_host_get_distinct_keys() {
        let key = |code| ProbeOutcome::classify(code, 6500).unwrap().error_key;
        assert_eq!(key(1), "timeout");
        assert_eq!(key(2), "noResponse");
        assert_eq!(key(-1), "noResponse");
        for code in [1, 2] {
            let outcome = ProbeOutcome::classify(code, 999).unwrap();
            assert_eq!(outcome.state_key, "fail");
            assert_eq!(outcome.latency_ms, 0, "a failure has no latency");
        }
    }

    #[test]
    fn a_cancelled_request_reports_no_result() {
        // Code 3 means the shutdown flag aborted the request: there is nothing
        // to show, so the row keeps whatever it had and the batch stops.
        assert_eq!(ProbeOutcome::classify(3, 1200), None);
    }

    #[test]
    fn a_plain_http_mirror_is_refused_before_any_request() {
        let outcome = ProbeOutcome::refused(ProbeError::PlainHttp);
        assert_eq!(outcome.state_key, "fail");
        assert_eq!(outcome.error_key, "plainHttp");
        assert_eq!(outcome.latency_ms, 0);
        // The rule, not the transport, is what produces it.
        assert_eq!(
            check_probe_url("http://ftp.dk.xemacs.org/voidlinux/"),
            Err(ProbeError::PlainHttp)
        );
    }

    #[test]
    fn the_batch_covers_every_row_of_the_model() {
        let targets = row_targets();
        assert_eq!(targets.len(), 34);
        for (row, url) in &targets {
            assert_eq!(MIRRORS[usize::try_from(*row).unwrap()].url, *url);
        }
        let plain_http: Vec<&str> = targets
            .iter()
            .filter(|(_, url)| check_probe_url(url).is_err())
            .map(|(_, url)| *url)
            .collect();
        assert_eq!(
            plain_http,
            [
                "http://ftp.dk.xemacs.org/voidlinux/",
                "http://ftp.debian.ru/mirrors/voidlinux/"
            ],
            "the two plain-HTTP entries stay in the list and are never requested"
        );
    }

    #[test]
    fn a_url_maps_back_to_its_row() {
        assert_eq!(row_of(DEFAULT_MIRROR), Some(0));
        assert_eq!(row_of("http://ftp.debian.ru/mirrors/voidlinux/"), Some(22));
        assert_eq!(row_of("https://not-a-mirror.example/"), None);
    }

    #[test]
    fn a_refused_apply_never_makes_the_backend_busy() {
        let mut apply = ApplyState::default();
        assert!(!apply.start(false), "no job, no busy");
        assert!(!apply.is_busy());
    }

    #[test]
    fn a_handed_out_job_makes_the_backend_busy_once() {
        let mut apply = ApplyState::default();
        assert!(apply.start(true), "the first job moves the flag");
        assert!(apply.is_busy());
        assert!(
            !apply.start(true),
            "a second job while busy does not move the flag"
        );
        assert!(
            !apply.start(false),
            "a refused job while busy does not move the flag"
        );
        assert!(apply.is_busy());
    }

    #[test]
    fn finishing_an_apply_clears_the_flag_exactly_once() {
        let mut apply = ApplyState::default();
        assert!(apply.start(true));
        assert!(apply.finish(), "the end of the job moves the flag");
        assert!(!apply.is_busy());
        assert!(!apply.finish(), "finishing while idle is a no-op");
        assert!(!apply.is_busy());
    }

    #[test]
    fn a_finished_apply_can_start_again() {
        let mut apply = ApplyState::default();
        assert!(apply.start(true));
        assert!(apply.finish());
        assert!(apply.start(true), "the next apply is accepted");
        assert!(apply.is_busy());
    }
}
