// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! The installed database, the transaction queue and transaction planning.
//!
//! One object owns the [`Store`] (installed map, queue, preferences, mirror) and
//! is therefore the single source of truth for everything that survives a
//! restart. It exposes the package model and the queue model, mutates the store
//! on behalf of QML, and starts the plan/job objects the dialogs show.
//!
//! A plan is not computed: it is whatever `xbps-install -n` or `xbps-remove -n`
//! would do, read on a worker thread, so what the dialog promises is what the
//! transaction would perform.
//!
//! Persistence: every mutating invokable saves the store once, after the
//! mutation, and a failed save is retried once before it becomes `error`.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use cxx_qt::CxxQtType;
use cxx_qt::Threading;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QObjectMutPtr, QString, QStringList};
use kxbps_core::{
    AUTO_GAP_MS, CheckOutcome, CheckStatus, InstalledMap, Plan, PlanItem, QueueAction, QueueItem,
    Store, StoreError, UpdatesOrigin, check_status, drop_applied_updates, effective_updates,
    held_updates_in, installs_as_updates, is_due, is_offline, local_replaces, not_installed,
    update_signature_of,
};

use crate::job_backend::JobPayload;
use crate::network::ReachabilityFn;
use crate::plan_backend::{PlanPayload, keys as plan_keys};
use crate::xbps_plan;
use crate::xbps_runner::{READ_TIMEOUT_MS, repos_argv, run};
use crate::xbps_snapshot::{
    SharedInvoke, Snapshot, SnapshotError, command_lines, read_remote_updates_logged, read_snapshot,
};
use kxbps_core::UpdateEntry;
use kxbps_core::xbps::parse_repos;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        // Declares `kxbps_create_package_model`, which lives in the model
        // shim rather than the backend shim.
        include!("model_shim.h");

        include!("backend_shim.h");

        // A mirror job reports its end through this owner; the forwarder is
        // only data (`QObject*`) plus one call on the generated class.
        #[cxx_name = "kxbps_mirror_apply_landed"]
        fn mirror_apply_landed(backend: QObjectMutPtr);

        #[cxx_name = "kxbps_create_package_model"]
        fn create_package_model(parent: QObjectMutPtr) -> QObjectMutPtr;

        #[cxx_name = "kxbps_create_queue_model"]
        fn create_queue_model(parent: QObjectMutPtr, queue_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "kxbps_package_model_set_state"]
        fn package_model_set_state(
            model: QObjectMutPtr,
            installed_json: &QString,
            queue_json: &QString,
        );

        #[cxx_name = "kxbps_package_model_set_snapshot"]
        fn package_model_set_snapshot(
            model: QObjectMutPtr,
            snapshot_json: &QString,
            queue_json: &QString,
        );

        #[cxx_name = "kxbps_queue_model_set_state"]
        fn queue_model_set_state(model: QObjectMutPtr, queue_json: &QString);

        /// Hands the list its repository answer. Called by the installed
        /// backend when a `read_list_search` read lands, so the QML side never
        /// has to copy hits between two objects itself.
        #[cxx_name = "kxbps_package_model_set_search_results"]
        fn package_model_set_search_results(
            model: QObjectMutPtr,
            hits_json: &QString,
            error_key: &QString,
            error_args: &QStringList,
        );

        #[cxx_name = "kxbps_create_plan_backend"]
        fn create_plan_backend(parent: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "kxbps_start_job"]
        fn start_job(owner: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "kxbps_object_string_property"]
        fn object_string_property(object: QObjectMutPtr, name: &QString) -> QString;

        #[cxx_name = "kxbps_query_backend_set_state"]
        fn query_backend_set_state(
            backend: QObjectMutPtr,
            installed_json: &QString,
            queue_json: &QString,
            updates_json: &QString,
            orphans_json: &QString,
        );

        #[cxx_name = "kxbps_release_object"]
        fn release_object(object: QObjectMutPtr);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, packages, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, queue_model, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, check_log, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, query_backend, READ, WRITE = set_query_backend, NOTIFY)]
        #[qproperty(i32, installed_count, READ, NOTIFY)]
        #[qproperty(i32, queue_count, READ, NOTIFY)]
        #[qproperty(i32, updates_count, READ, NOTIFY)]
        // Bumped on every `refresh()`, including changes the counters cannot
        // see (a hold toggle). QML uses it to re-read snapshots that have no
        // notify signal of their own (`catalogBackend.get`).
        #[qproperty(i32, state_revision, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
        // The `%1…%2` arguments of the save error's sentence; `error` alone is
        // the key, never a finished English sentence.
        #[qproperty(QStringList, error_args, READ, NOTIFY)]
        // A failed read of the system (xbps missing, timed out, unparsable
        // output) as the i18n key QML wraps in a sentence, plus its `%1…%3`
        // arguments. Empty key means the last read succeeded.
        #[qproperty(QString, read_error, READ, NOTIFY)]
        #[qproperty(QStringList, read_error_args, READ, NOTIFY)]
        // Whether a snapshot is being read. The list keeps its previous rows
        // while one is in flight; QML only needs this to explain an empty list.
        #[qproperty(bool, loading, READ, NOTIFY)]
        // Whether a plan is being read from xbps right now.
        #[qproperty(bool, plan_loading, READ, NOTIFY)]
        // Whether a finished plan is dropping the queue entries it found
        // satisfied. The drop moves `queueCount`, and that signal normally
        // cancels the request a dialog is waiting for; this says the change is
        // the plan's own bookkeeping, so the answer it is about to hand over
        // must survive it.
        #[qproperty(bool, plan_pruning, READ, NOTIFY)]
        // A stable fingerprint of the updates the window would announce: the
        // FNV of the effective update set, exactly what `apply_plan` compares
        // against the dismissed marker. Empty until the first read answers.
        // `refresh()` is the only writer and only emits when it moves.
        #[qproperty(QString, pending_signature, READ, NOTIFY)]
        // Whether a fresh check is running; the Check button and the tray
        // action are disabled while it is and show a spinner.
        #[qproperty(bool, checking, READ, NOTIFY)]
        // When the last check succeeded, epoch milliseconds; `0` is never. The
        // window reads it to decide whether the show-time check is due, and the
        // status sentence words the age from it.
        #[qproperty(i64, last_check_at, READ, NOTIFY)]
        // The last check's sentence as an i18n key plus its `%1…%4` arguments;
        // empty means no sentence to add. QML words it, never this side.
        #[qproperty(QString, check_key, READ, NOTIFY)]
        #[qproperty(QStringList, check_args, READ, NOTIFY)]
        // Why the last check could not refresh the list; empty when it
        // succeeded. Kept apart from `check_key` so the strip can say what
        // happened after the sentence that announced it has gone.
        #[qproperty(QString, check_error_key, READ, NOTIFY)]
        #[qproperty(QStringList, check_error_args, READ, NOTIFY)]
        type InstalledBackend = super::InstalledBackendRust;

        #[qinvokable]
        fn toggle_queue(self: Pin<&mut InstalledBackend>, name: &QString, action: &QString);

        #[qinvokable]
        fn remove_queued(self: Pin<&mut InstalledBackend>, name: &QString);

        #[qinvokable]
        fn clear_queue(self: Pin<&mut InstalledBackend>);

        /// Marks every pending update as queued, leaving the rest of the queue
        /// alone; the marking is idempotent.
        #[qinvokable]
        fn queue_all_updates(self: Pin<&mut InstalledBackend>);

        #[qinvokable]
        /// Pins or releases one package through `pkexec xbps-pkgdb`, and
        /// returns the job so the window shows the password prompt, the log
        /// and the result like any other privileged action.
        #[qinvokable]
        fn toggle_hold(self: &InstalledBackend, name: &QString) -> QObjectMutPtr;

        /// Fetches the repository indexes and writes them to disk through
        /// `pkexec xbps-install -S`, and returns the job so the window shows the
        /// password prompt and the log.
        ///
        /// The reads fetch indexes into memory and write nothing, so a package
        /// added to a repository stays invisible to `xbps-query -Rs` until
        /// something syncs. This is that something.
        #[qinvokable]
        fn sync_indexes(self: &InstalledBackend) -> QObjectMutPtr;

        // Planning asks xbps what it would do, which is a process, so every
        // `open*` starts a worker thread and answers through `plan_ready`
        // instead of returning the object. The returned number is the request
        // id; a caller that sees a second answer for one request only shows the
        // last one, and a plan whose dry run finished after a newer request
        // started is dropped before it reaches QML.
        /// Plans the transaction queue; answers with the request id.
        #[qinvokable]
        fn open_queue_plan(self: Pin<&mut InstalledBackend>) -> u64;

        /// Plans the system upgrade (`xbps-install -n -Su`).
        #[qinvokable]
        fn open_system_plan(self: Pin<&mut InstalledBackend>) -> u64;

        /// Plans installing exactly `names`.
        #[qinvokable]
        fn open_named_plan(
            self: Pin<&mut InstalledBackend>,
            names: &QStringList,
            title_key: &QString,
        ) -> u64;

        /// Plans removing `name` (and, with `cascade`, whatever else the call
        /// asked for).
        #[qinvokable]
        fn open_remove(self: Pin<&mut InstalledBackend>, name: &QString, cascade: bool) -> u64;

        /// Plans removing `names`, the list the Blocked dialog approved.
        #[qinvokable]
        fn open_remove_names(
            self: Pin<&mut InstalledBackend>,
            names: &QStringList,
            cascade: bool,
        ) -> u64;

        /// Hands the package list the repository answer of its search field.
        ///
        /// QML calls this with what `QueryBackend.listSearchDone` carried; the
        /// model is the only place the rows live, so the copy happens here
        /// rather than in the view. A failed read passes an empty `hits_json`
        /// and the failure key, and the list then shows no rows.
        #[qinvokable]
        fn apply_search_results(
            self: &InstalledBackend,
            hits_json: &QString,
            error_key: &QString,
            error_args: &QStringList,
        );

        /// One plan finished: the payload as JSON, the request it belongs to
        /// and the title/blocked metadata the dialog needs to rebuild it.
        /// QML calls `makePlan(payloadJson)` for a plan with steps.
        // The payload carries its own title and names, so the signal only says
        // which request it answers and which payload to build.
        #[qsignal]
        fn plan_ready(self: Pin<&mut InstalledBackend>, payload_json: QString, request: u64);

        /// Builds the `PlanBackend` child of a payload QML received; the
        /// caller owns it and releases it like every other plan.
        #[qinvokable]
        fn make_plan(self: &InstalledBackend, payload_json: &QString) -> QObjectMutPtr;

        /// Whether a plan is being read right now; the dialogs show a spinner.
        #[qinvokable]
        fn plan_loading_of(self: &InstalledBackend) -> bool;

        #[qinvokable]
        fn apply_plan(self: &InstalledBackend, plan: QObjectMutPtr) -> QObjectMutPtr;

        // Re-reads the installed set, the updates and the orphan list from
        // xbps on a worker thread. Called once when the window opens and by the
        // "reload" button; the previous rows stay visible until it answers.
        #[qinvokable]
        fn reload(self: Pin<&mut InstalledBackend>);

        /// Asks the repositories for updates: `xbps-install -M -un`, on a worker
        /// thread.
        ///
        /// `force` is the manual button and the tray action: it ignores the
        /// clock and the reachability reading. `false` is the timer and the
        /// window-show hook, which do nothing inside the gap and nothing at all
        /// when Qt reports no connection.
        #[qinvokable]
        fn check_updates(self: Pin<&mut InstalledBackend>, force: bool);

        #[qinvokable]
        fn reset(self: Pin<&mut InstalledBackend>);

        /// Whether the installed set holds `name`. A view that needs one
        /// package's state reads the list it already shows, rather than asking
        /// xbps for the package.
        #[qinvokable]
        fn is_installed(self: &InstalledBackend, name: &QString) -> bool;

        /// The action queued for `name` (`install`, `remove`, `update`), or an
        /// empty string when the package is not in the queue. The details pane
        /// reads it to offer the opposite gesture.
        #[qinvokable]
        fn queued_action(self: &InstalledBackend, name: &QString) -> QString;

        #[qinvokable]
        fn release(self: &InstalledBackend, object: QObjectMutPtr);

        // Offers one announcement, if the decision table in
        // `crate::notification` allows it, and records it in the same call.
        // Returns `{kind, count, signature}` as JSON, or
        // an empty string when nothing is announced. `visible` is the window's
        // visibility: a popup over the list the user is reading is noise, so a
        // visible window only defers the announcement to a later call.
        // `startup` marks the offer a start makes, a deferred retry of it
        // included: the start switch gates that one, not a running check.
        #[qinvokable]
        fn take_announcement(
            self: Pin<&mut InstalledBackend>,
            visible: bool,
            startup: bool,
        ) -> QString;

        fn set_query_backend(self: Pin<&mut InstalledBackend>, value: QObjectMutPtr);

        // The selected mirror lives in the store, so the mirror backend reads and
        // writes it through these two forwarders instead of keeping a second copy
        // of the store. Not invokable: the Mirrors view owns the presentation.
        fn mirror_value(self: &InstalledBackend) -> QString;

        fn set_mirror_value(self: Pin<&mut InstalledBackend>, url: &QString);

        // The preference block lives in the store too. The preferences backend
        // reads it once at attach and writes each change back through these
        // three forwarders, so there is still exactly one `Store`. Not
        // invokable: QML writes through `PrefsBackend`, which notifies its own
        // properties as well.
        fn prefs_value(self: &InstalledBackend) -> QString;

        fn set_pref_bool(self: Pin<&mut InstalledBackend>, key: &QString, value: bool);

        fn set_pref_string(self: Pin<&mut InstalledBackend>, key: &QString, value: &QString);

        fn set_pref_int(self: Pin<&mut InstalledBackend>, key: &QString, value: i32);

        // Called by `kxbps_installed_apply_plan` on the GUI thread when a job
        // finishes; not invokable so QML cannot bypass the job.
        fn apply_plan_items(self: Pin<&mut InstalledBackend>, items_json: &QString);

        // Called by `kxbps_installed_hold_landed` when a hold job finishes; not
        // invokable for the same reason.
        fn hold_landed(self: Pin<&mut InstalledBackend>);

        // Called by `kxbps_installed_sync_landed` when the index sync finishes;
        // not invokable, so only a finished job can trigger the re-read.
        fn sync_landed(self: Pin<&mut InstalledBackend>);

        // The mirror backend's apply job reports through this owner, because the
        // job is only created with the owner it was handed. `watch` records the
        // mirror backend as the one waiting for the next job to end; `landed`
        // tells it the job is over; `unwatch` forgets it when that backend is
        // torn down, so its address is never called after free. None is
        // invokable: QML starts the apply through
        // `MirrorBackend.applySystemMirror`, which owns the pairing.
        fn watch_mirror_job(self: Pin<&mut InstalledBackend>, backend: QObjectMutPtr);

        fn mirror_job_landed(self: Pin<&mut InstalledBackend>);

        fn unwatch_mirror_job(self: Pin<&mut InstalledBackend>);
    }

    impl cxx_qt::Constructor<()> for InstalledBackend {}
    // A read runs on a worker thread and reports back through
    // `qt_thread().queue(...)`; that needs the thread handle CXX-Qt generates.
    impl cxx_qt::Threading for InstalledBackend {}
}

/// Rust side of [`qobject::InstalledBackend`].
pub struct InstalledBackendRust {
    store: Store,
    /// Child model created by [`cxx_qt::Initialize::initialize`]; Qt owns it.
    packages: QObjectMutPtr,
    queue_model: QObjectMutPtr,
    /// The query façade (a sibling in QML), pushed the same snapshot on every
    /// refresh; null until QML wires it.
    query_backend: QObjectMutPtr,
    installed_count: i32,
    queue_count: i32,
    updates_count: i32,
    state_revision: i32,
    error: QString,
    error_args: QStringList,
    read_error: QString,
    read_error_args: QStringList,
    loading: bool,
    plan_loading: bool,
    plan_pruning: bool,
    pending_signature: QString,
    /// The last snapshot xbps answered; its updates and orphans are what the
    /// model and the query façade show. `None` until the first read.
    snapshot: Option<Snapshot>,
    /// How many reads were started; only the newest may write its result, so
    /// a slow first read cannot overwrite a newer one.
    generation: Arc<AtomicU64>,
    /// Same guard for plans: a dry run that finishes after a newer request
    /// started is dropped, and a plan is never shown for a stale queue.
    plan_generation: Arc<AtomicU64>,
    /// The process transport. Tests install a fake here, so the suite never
    /// starts xbps; production installs [`crate::xbps_bridge::production_invoke`].
    invoke: SharedInvoke,
    /// Network reachability, as `QNetworkInformation::Reachability` reports it.
    ///
    /// Injected like [`Self::invoke`] so a test can say "Disconnected" without a
    /// D-Bus or a NetworkManager; production installs [`crate::network_reachability`].
    /// `-1` means no backend loaded, which is not offline.
    reachability: ReachabilityFn,
    /// Where the pending-update list came from, in memory only.
    ///
    /// A local read cannot see anything the last fresh check did not; the caller
    /// uses this to keep a young remote list from being overwritten by it.
    updates_origin: UpdatesOrigin,
    /// Whether a check is running; the Checks button is disabled while it is.
    checking: bool,
    /// The last successful check, epoch milliseconds; `0` is never. Mirrors
    /// `prefs.last_check_at` for the property, since QML cannot read prefs from
    /// here and the store's copy is the persisted one.
    last_check_at: i64,
    /// The sentence the last check produced: the key and its `%1…%4` arguments.
    check_key: QString,
    check_args: QStringList,
    /// The failure sentence of the last check: an i18n key, empty when the last
    /// check succeeded. Kept beside the status so the strip can show the reason
    /// after the sentence that announced it is gone.
    check_error_key: QString,
    check_error_args: QStringList,
    /// How many checks were started; only the newest may write its answer.
    check_generation: Arc<AtomicU64>,
    /// The mirror backend waiting for the next job to end, as an address: a job
    /// is created with this owner and has no other way back to its caller.
    mirror_waiter: usize,
    /// The lines the update checks printed, for the log drawer.
    ///
    /// A check is not a transaction and has no job, so its output needs a home
    /// of its own: the drawer shows this model until a transaction exists.
    check_log: QObjectMutPtr,
}

impl Default for InstalledBackendRust {
    /// Only used before `initialize` runs; the models stay null until then and
    /// the store is empty.
    fn default() -> Self {
        Self {
            store: Store::fresh(),
            packages: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            queue_model: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            query_backend: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            installed_count: 0,
            queue_count: 0,
            updates_count: 0,
            state_revision: 0,
            error: QString::from(""),
            error_args: QStringList::default(),
            read_error: QString::from(""),
            read_error_args: QStringList::default(),
            loading: false,
            plan_loading: false,
            plan_pruning: false,
            pending_signature: QString::from(""),
            snapshot: None,
            generation: Arc::new(AtomicU64::new(0)),
            plan_generation: Arc::new(AtomicU64::new(0)),
            invoke: crate::xbps_bridge::production_invoke(),
            reachability: crate::network::production_reachability(),
            updates_origin: UpdatesOrigin::Local,
            checking: false,
            last_check_at: 0,
            check_key: QString::from(""),
            check_args: QStringList::default(),
            check_error_key: QString::from(""),
            check_error_args: QStringList::default(),
            check_generation: Arc::new(AtomicU64::new(0)),
            mirror_waiter: 0,
            check_log: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
        }
    }
}

impl cxx_qt::Initialize for qobject::InstalledBackend {
    /// Loads the store, then creates both child models.
    ///
    /// A store that cannot be loaded is not fatal: an empty store stays in
    /// memory and `error`/`errorArgs` carry the key and arguments of the
    /// sentence QML shows. The installed set is not the store's: the window
    /// opens empty and the first read of xbps fills it a moment later (see
    /// [`qobject::InstalledBackend::reload`]).
    fn initialize(mut self: Pin<&mut Self>) {
        let (store, store_error) = match Store::load() {
            Ok(store) => (store, None),
            Err(err) => (Store::fresh(), Some(err)),
        };

        let parent = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::InstalledBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self)
                    .cast_mut(),
            )
        };
        let queue_json = json_of(&store.queue);
        let packages = qobject::create_package_model(parent);
        let queue_model = qobject::create_queue_model(parent, &queue_json);
        // The check log is a plain log model, the same type the job dialog
        // uses: one `line` role and a `count`, appended a line at a time.
        let check_log = crate::job_backend::qobject::create_log_model(parent);

        let queue_count = store.queue.len();
        let (error_key, error_args) = match &store_error {
            Some(err) => store_error_parts(err),
            None => (String::new(), Vec::new()),
        };

        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.store = store;
        data.packages = packages;
        data.queue_model = queue_model;
        data.check_log = check_log;
        data.installed_count = 0;
        data.queue_count = i32::try_from(queue_count).unwrap_or(i32::MAX);
        data.updates_count = 0;
        data.error = QString::from(error_key.as_str());
        data.error_args = error_args
            .iter()
            .map(|arg| QString::from(arg.as_str()))
            .collect::<QStringList>();
        data.last_check_at = data.store.prefs.last_check_at;
    }
}

impl InstalledBackendRust {
    /// Marks every pending update of the last read as queued, leaving the
    /// entries that are already in the queue alone.
    ///
    /// Idempotent: a second call over the same pending updates adds nothing, and
    /// a package that is already queued keeps its entry instead of being queued
    /// twice.
    fn mark_pending_updates(&mut self) {
        let pending: Vec<String> = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .updates
                    .iter()
                    .map(|update| update.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for name in pending {
            let queued = self.store.queue.iter().any(|item| item.name == name);
            if !queued {
                self.store.queue.push(QueueItem {
                    name,
                    action: QueueAction::Update,
                });
            }
        }
    }

    /// Runs one update check and returns the sentence it produced.
    ///
    /// `force` is the manual button: it ignores the clock and the reachability
    /// reading, because the user asked and xbps' own answer is the best one
    /// available. An automatic check respects both — it does nothing inside the
    /// gap, and nothing at all when the system is `Disconnected`.
    ///
    /// Only a successful remote read replaces the list; a failure leaves it
    /// exactly as it was, which is what makes the age in the sentence truthful.
    /// The returned status is also stored in `check_key`/`check_args`, so the
    /// QObject properties describe the same run this call reports.
    #[cfg_attr(not(test), allow(dead_code))]
    fn run_check(&mut self, force: bool, now: i64) -> CheckStatus {
        let status = self.decide_and_check(force, now);
        self.check_key = QString::from(status.key);
        self.check_args = strings_list(&status.args);
        self.last_check_at = self.store.prefs.last_check_at;
        status
    }

    /// The decision and the process, without touching the property fields.
    #[cfg_attr(not(test), allow(dead_code))]
    fn decide_and_check(&mut self, force: bool, now: i64) -> CheckStatus {
        if self.checking {
            return CheckStatus::none();
        }
        if !force && !is_due(self.store.prefs.last_check_at, now, AUTO_GAP_MS) {
            return CheckStatus::none();
        }
        let offline = is_offline((self.reachability)());
        if !force && offline {
            // Only an automatic check refuses to try; a manual one may be
            // right when Qt's backend is wrong, and xbps will say so itself.
            self.finish_check_error("checkOffline", Vec::new());
            return check_status(&CheckOutcome::Offline, &self.check_age(now));
        }
        self.checking = true;
        let invoke = self.invoke.clone();
        let (result, lines) = check_remote(&*invoke);
        self.checking = false;
        self.append_check_lines(&lines);
        let age = self.check_age(now);
        match result {
            Ok(updates) => {
                let count = updates.len();
                self.store.replace_updates(updates.clone());
                if let Some(snapshot) = self.snapshot.as_mut() {
                    snapshot.updates = updates;
                }
                self.updates_origin = UpdatesOrigin::Remote { at: now };
                self.store.set_pref_last_check_at(now);
                self.check_error_key = QString::from("");
                self.check_error_args = QStringList::default();
                let outcome = if count == 0 {
                    CheckOutcome::UpToDate
                } else {
                    CheckOutcome::Updates { count }
                };
                check_status(&outcome, &age)
            }
            Err(SnapshotError::Repositories {
                failed,
                total,
                detail,
                ..
            }) => {
                self.finish_check_error("checkRepositoriesFailed", Vec::new());
                check_status(
                    &CheckOutcome::RepositoriesFailed {
                        failed,
                        total,
                        detail,
                    },
                    &age,
                )
            }
            Err(err) => {
                self.finish_check_error(err.key(), err.args());
                check_status(&CheckOutcome::ReadFailed { key: err.key() }, &age)
            }
        }
    }

    /// Appends the lines one check printed to the drawer's model.
    ///
    /// The model does not exist in the tests that drive the pure decision
    /// path, so a null pointer is a no-op rather than a crash.
    fn append_check_lines(&self, lines: &[String]) {
        let log = self.check_log;
        if log.as_mut_ptr().is_null() {
            return;
        }
        for line in lines {
            crate::job_backend::qobject::log_model_append(log, &QString::from(line.as_str()));
        }
    }

    /// Records a failed check: the list stays, the sentence is kept.
    #[cfg_attr(not(test), allow(dead_code))]
    fn finish_check_error(&mut self, key: &str, args: Vec<String>) {
        self.check_error_key = QString::from(key);
        self.check_error_args = strings_list(&args);
    }

    /// How old the last successful check is, worded for the sentence.
    ///
    /// The age is spelled in words rather than seconds so the caller can
    /// pluralise it in QML's translation; an unreadable clock and a stamp in
    /// the future both read as "not checked yet", because neither can be
    /// trusted to produce a sane age.
    fn check_age(&self, now: i64) -> String {
        let last = self.store.prefs.last_check_at;
        if last <= 0 || last > now {
            return String::from("0");
        }
        (now - last).to_string()
    }
}

impl qobject::InstalledBackend {
    /// Offers one announcement and records it in the same call.
    ///
    /// See [`take_announcement_with`] for the rules. One save for the whole
    /// announcement, and none at all when nothing was announced: a deferred
    /// popup must not write a file.
    fn take_announcement(
        mut self: Pin<&mut Self>,
        visible: bool,
        startup: bool,
    ) -> cxx_qt_lib::QString {
        let now = now_ms();
        let (json, changed) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let before = data.store.prefs.clone();
            let json = take_announcement_with(&mut data.store, now, visible, startup);
            (json, data.store.prefs != before)
        };
        if changed {
            save_store(self.as_mut());
        }
        cxx_qt_lib::QString::from(json.as_str())
    }

    fn toggle_queue(
        mut self: Pin<&mut Self>,
        name: &cxx_qt_lib::QString,
        action: &cxx_qt_lib::QString,
    ) {
        let action = parse_action(&action.to_string());
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.toggle_queue(&name.to_string(), action);
        }
        save_store(self.as_mut());
        refresh_store(self.as_mut());
    }

    fn remove_queued(mut self: Pin<&mut Self>, name: &cxx_qt_lib::QString) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.remove_queued(&name.to_string());
        }
        save_store(self.as_mut());
        refresh_store(self.as_mut());
    }

    fn clear_queue(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.clear_queue();
        }
        save_store(self.as_mut());
        refresh_store(self.as_mut());
    }

    /// Marks every pending update as queued, leaving the rest of the queue
    /// alone; the marking is idempotent.
    fn queue_all_updates(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.mark_pending_updates();
        }
        save_store(self.as_mut());
        refresh_store(self.as_mut());
    }

    fn toggle_hold(&self, name: &cxx_qt_lib::QString) -> QObjectMutPtr {
        let name = name.to_string();
        // The target state comes from the snapshot, which `xbps-query -H`
        // filled: the store never invents a hold of its own.
        let Some(installed) = self.store.installed.get(&name) else {
            return unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) };
        };
        let json = crate::job_backend::hold_payload_json(&name, !installed.held);
        qobject::start_job(self.parent_ptr(), &QString::from(json.as_str()))
    }

    /// Called by `kxbps_installed_hold_landed` on the GUI thread when a hold
    /// job finishes, so the pill, the filter and the details pane read the
    /// state xbps now has rather than the one the click assumed.
    fn hold_landed(mut self: Pin<&mut Self>) {
        self.as_mut().reload();
    }

    /// Fetches the repository indexes and writes them to disk, through polkit.
    ///
    /// The reads fetch indexes into memory and write nothing, so a package
    /// added to a repository stays invisible to `xbps-query -Rs` until
    /// something syncs. This is that something, and it is the only action in
    /// the window that needs a password for a read.
    fn sync_indexes(&self) -> QObjectMutPtr {
        let json = crate::job_backend::sync_payload_json();
        qobject::start_job(self.parent_ptr(), &QString::from(json.as_str()))
    }

    /// Called by `kxbps_installed_sync_landed` on the GUI thread when the sync
    /// finishes: the disk index moved, so the update list is recomputed against
    /// what xbps just wrote.
    fn sync_landed(mut self: Pin<&mut Self>) {
        self.as_mut().check_updates(true);
    }

    /// Plans the transaction queue on a worker thread.
    fn open_queue_plan(mut self: Pin<&mut Self>) -> u64 {
        let task = {
            let data = &*self;
            PlanTask::queue(&data.store)
        };
        self.as_mut().start_plan(task)
    }

    /// Plans the system upgrade on a worker thread.
    fn open_system_plan(mut self: Pin<&mut Self>) -> u64 {
        self.as_mut().start_plan(PlanTask::system())
    }

    /// Plans installing exactly `names` on a worker thread.
    fn open_named_plan(
        mut self: Pin<&mut Self>,
        names: &cxx_qt_lib::QStringList,
        title_key: &cxx_qt_lib::QString,
    ) -> u64 {
        let names = strings_of(names);
        let title_key = title_key.to_string();
        let title_key = if title_key.is_empty() {
            plan_keys::PLAN_INSTALL.to_string()
        } else {
            title_key
        };
        self.as_mut().start_plan(PlanTask::named(names, title_key))
    }

    /// Plans removing `name` on a worker thread.
    fn open_remove(mut self: Pin<&mut Self>, name: &cxx_qt_lib::QString, cascade: bool) -> u64 {
        self.as_mut()
            .open_remove_names(&cxx_qt_lib::QStringList::from(name), cascade)
    }

    /// Plans removing `names` on a worker thread.
    fn open_remove_names(
        mut self: Pin<&mut Self>,
        names: &cxx_qt_lib::QStringList,
        cascade: bool,
    ) -> u64 {
        self.as_mut()
            .start_plan(PlanTask::remove(strings_of(names), cascade))
    }

    /// Hands the package list the repository answer of its search field.
    fn apply_search_results(
        &self,
        hits_json: &cxx_qt_lib::QString,
        error_key: &cxx_qt_lib::QString,
        error_args: &cxx_qt_lib::QStringList,
    ) {
        let packages = self.packages;
        if packages.as_ptr().is_null() {
            return;
        }
        qobject::package_model_set_search_results(packages, hits_json, error_key, error_args);
    }

    /// Starts the dry runs of one plan and answers with its request id.
    ///
    /// The GUI thread only takes a copy of what the worker needs and spawns the
    /// thread; the answer arrives through `qt_thread().queue(...)`, the only
    /// way a background thread may touch this object.
    fn start_plan(mut self: Pin<&mut Self>, task: PlanTask) -> u64 {
        let request = self.plan_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = Arc::clone(&self.plan_generation);
        let invoke = Arc::clone(&self.invoke);
        let installed = self.store.installed.clone();
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.plan_loading = true;
        }
        self.as_mut().plan_loading_changed();

        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let payload = task.run(&installed, &*invoke);
            let meta = PlanMeta { request };
            thread
                .queue(move |backend| backend.finish_plan(request, &current, meta, payload))
                .ok();
        });
        request
    }

    /// Emits a finished plan, unless a newer request owns the dialog.
    fn finish_plan(
        mut self: Pin<&mut Self>,
        request: u64,
        current: &Arc<AtomicU64>,
        meta: PlanMeta,
        payload: PlanPayload,
    ) {
        if current.load(Ordering::SeqCst) != request {
            // A newer plan owns the dialog; this answer describes a queue the
            // user has already moved past.
            return;
        }
        let was_loading = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let was_loading = data.plan_loading;
            data.plan_loading = false;
            was_loading
        };
        if was_loading {
            self.as_mut().plan_loading_changed();
        }
        // A name the dry run called already installed has nothing left to do;
        // it leaves the queue here instead of staying as a row that looks
        // queued and changes nothing. Only the queue moves, so the store-only
        // refresh is enough.
        if !payload.satisfied.is_empty() {
            let dropped = {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                let before = data.store.queue.len();
                data.store
                    .queue
                    .retain(|item| !payload.satisfied.contains(&item.name));
                let dropped = before != data.store.queue.len();
                // `queueCountChanged` reaches QML from the refresh below, and
                // the window treats it as an edit that cancels the request a
                // dialog is waiting for. This drop is the plan's own move, so
                // the answer it is about to hand over must survive it.
                data.plan_pruning = dropped;
                dropped
            };
            if dropped {
                self.as_mut().plan_pruning_changed();
                save_store(self.as_mut());
                refresh_store(self.as_mut());
                {
                    let data = Pin::get_mut(self.as_mut().rust_mut());
                    data.plan_pruning = false;
                }
                self.as_mut().plan_pruning_changed();
            }
        }
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| String::from("{}"));
        self.as_mut()
            .plan_ready(QString::from(json.as_str()), meta.request);
    }

    /// Builds the plan object QML was told about.
    fn make_plan(&self, payload_json: &cxx_qt_lib::QString) -> QObjectMutPtr {
        qobject::create_plan_backend(self.parent_ptr(), payload_json)
    }

    /// Whether a plan is in flight.
    fn plan_loading_of(&self) -> bool {
        self.plan_loading
    }

    fn apply_plan(&self, plan: QObjectMutPtr) -> QObjectMutPtr {
        let items_json = qobject::object_string_property(plan, &QString::from("itemsJson"));
        let items: Vec<PlanItem> =
            serde_json::from_str(&items_json.to_string()).unwrap_or_default();
        if items.is_empty() {
            return unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) };
        }
        let payload = JobPayload {
            items,
            ..JobPayload::default()
        };
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| String::from("{}"));
        qobject::start_job(self.parent_ptr(), &QString::from(json.as_str()))
    }

    fn is_installed(&self, name: &cxx_qt_lib::QString) -> bool {
        self.store.installed.contains_key(&name.to_string())
    }

    /// The queue token for one package, empty when it is not queued.
    fn queued_action(&self, name: &cxx_qt_lib::QString) -> cxx_qt_lib::QString {
        queued_action_in(&self.store.queue, &name.to_string())
            .map(cxx_qt_lib::QString::from)
            .unwrap_or_default()
    }

    fn reset(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.reset();
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
        // Reset drops the cached system state, which is not the system: read
        // it again so the window shows what is installed rather than nothing.
        self.as_mut().reload();
    }

    /// Schedules a plan or job object for deletion.
    ///
    /// Every `open*` and `applyPlan` returns a child of this backend. A
    /// C++-parented object gets no JavaScript ownership, so QML holding it in a
    /// property is not enough to free it: the dialog must call this when it
    /// closes, after dropping its own reference.
    fn release(&self, object: QObjectMutPtr) {
        // Only the throwaway plan/job children may go; releasing one of the
        // long-lived models would take the app down with it.
        let target = object.as_ptr();
        if target.is_null()
            || target == self.packages.as_ptr()
            || target == self.queue_model.as_ptr()
            || target == self.query_backend.as_ptr()
        {
            return;
        }
        qobject::release_object(object);
    }

    /// The selected mirror, as `mirror.json` holds it.
    fn mirror_value(&self) -> cxx_qt_lib::QString {
        QString::from(self.store.mirror.as_str())
    }

    /// Records which mirror backend is waiting for the next job to end.
    fn watch_mirror_job(mut self: Pin<&mut Self>, backend: QObjectMutPtr) {
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.mirror_waiter = backend.as_ptr() as usize;
    }

    /// Forgets the waiting mirror backend, because it is being torn down.
    ///
    /// The slot is a raw address of a `MirrorBackend` the owner does not own, so
    /// the owner must be told when that object goes away; the mirror backend
    /// calls this from `set_owner(nullptr)` with the owner it is detaching from.
    /// After this the slot is `0`, and [`Self::mirror_job_landed`] has nothing
    /// to call, so the address is only ever read while its object is alive.
    fn unwatch_mirror_job(mut self: Pin<&mut Self>) {
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.mirror_waiter = 0;
    }

    /// Tells the waiting mirror backend that its apply job is over.
    ///
    /// A mirror apply is the only job this forwarding names, and it is a single
    /// job: the slot is cleared *before* the call, so a landing that re-enters
    /// this forwarder (or a second one) sees `0` and cannot call twice.
    fn mirror_job_landed(mut self: Pin<&mut Self>) {
        let waiter = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            std::mem::replace(&mut data.mirror_waiter, 0)
        };
        if waiter == 0 {
            return;
        }
        let backend = unsafe { QObjectMutPtr::from_raw(waiter as *mut cxx_qt::QObject) };
        qobject::mirror_apply_landed(backend);
    }

    /// Persists a mirror picked in the Mirrors view.
    ///
    /// The store is the only owner of the selection, so the write and its save
    /// happen here; an empty URL is not a mirror and is ignored.
    fn set_mirror_value(mut self: Pin<&mut Self>, url: &cxx_qt_lib::QString) {
        let url = url.to_string();
        if url.is_empty() {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.set_mirror(&url);
        }
        save_store(self.as_mut());
    }

    /// The whole preference block, as `prefs.json` holds it.
    fn prefs_value(&self) -> cxx_qt_lib::QString {
        json_of(&self.store.prefs)
    }

    /// Persists one bool preference named by its QML key.
    ///
    /// An unknown key, and a value the store already holds, change nothing:
    /// no file is written for a no-op, so a binding cannot save on every
    /// evaluation.
    fn set_pref_bool(mut self: Pin<&mut Self>, key: &cxx_qt_lib::QString, value: bool) {
        let Some(write) = crate::prefs_backend::bool_write(&key.to_string(), value) else {
            return;
        };
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            crate::prefs_backend::apply_to_store_if_changed(&write, &mut data.store)
        };
        if changed {
            save_store(self.as_mut());
        }
    }

    /// Persists one string preference named by its QML key; same rules as
    /// [`Self::set_pref_bool`].
    fn set_pref_string(
        mut self: Pin<&mut Self>,
        key: &cxx_qt_lib::QString,
        value: &cxx_qt_lib::QString,
    ) {
        let Some(write) = crate::prefs_backend::string_write(&key.to_string(), &value.to_string())
        else {
            return;
        };
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            crate::prefs_backend::apply_to_store_if_changed(&write, &mut data.store)
        };
        if changed {
            save_store(self.as_mut());
        }
    }

    /// Persists one integer preference named by its QML key; same rules as
    /// [`Self::set_pref_bool`]. A negative value, or a key that is not an
    /// integer preference, changes nothing.
    fn set_pref_int(mut self: Pin<&mut Self>, key: &cxx_qt_lib::QString, value: i32) {
        let Some(write) = crate::prefs_backend::int_write(&key.to_string(), value) else {
            return;
        };
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            crate::prefs_backend::apply_to_store_if_changed(&write, &mut data.store)
        };
        if changed {
            save_store(self.as_mut());
        }
    }

    /// Wires the query façade and pushes the current snapshot into it.
    fn set_query_backend(mut self: Pin<&mut Self>, value: QObjectMutPtr) {
        let (installed_json, queue_json, updates_json, orphans_json) = {
            let data = &*self;
            state_json(&data.store, data.snapshot.as_ref())
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.query_backend = value;
        }
        push_query_state(
            value,
            &installed_json,
            &queue_json,
            &updates_json,
            &orphans_json,
        );
        self.as_mut().query_backend_changed();
    }

    /// Reads the installed set, the updates and the orphan list from xbps.
    ///
    /// The GUI thread only bumps the generation and starts the thread; the
    /// process runs on the worker, and its result arrives through
    /// `qt_thread().queue(...)`, which is the only way a background thread may
    /// touch this object. The previous rows stay visible until it answers, and
    /// a read that finishes after a newer one was started is dropped.
    fn reload(mut self: Pin<&mut Self>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.loading = true;
        }
        self.as_mut().loading_changed();

        let thread = self.qt_thread();
        let current = self.generation.clone();
        let invoke = self.invoke.clone();
        std::thread::spawn(move || {
            let result = read_snapshot(&*invoke);
            // The closure runs on the GUI thread; `thread.queue` fails only
            // once the backend is gone, and then nobody wants the result.
            thread
                .queue(move |backend| backend.finish_read(generation, &current, result))
                .ok();
        });
    }

    /// Asks the repositories for updates, on a worker thread.
    ///
    /// The GUI thread only decides whether to start and marks `checking`; the
    /// process and the reachability read happen on the worker, and the answer
    /// arrives through `qt_thread().queue(...)` — the only way a background
    /// thread may touch this object. A check that finishes after a newer one
    /// started is dropped, so a slow repository can never overwrite the answer
    /// the user is waiting for.
    fn check_updates(mut self: Pin<&mut Self>, force: bool) {
        let now = now_ms();
        let already = self.checking;
        if already {
            return;
        }
        if !force && !is_due(self.store.prefs.last_check_at, now, AUTO_GAP_MS) {
            return;
        }
        let offline = is_offline((self.reachability)());
        if !force && offline {
            // Nothing to run: the sentence is built here, on the GUI thread,
            // and reported like any other finished check.
            let status = self.check_age(now);
            let status = check_status(&CheckOutcome::Offline, &status);
            self.finish_check_offline(&status);
            return;
        }

        let generation = self.check_generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.checking = true;
            data.check_key = QString::from("");
            data.check_args = QStringList::default();
        }
        self.as_mut().checking_changed();
        self.as_mut().check_key_changed();
        self.as_mut().check_args_changed();

        let thread = self.qt_thread();
        let current = self.check_generation.clone();
        let invoke = self.invoke.clone();
        std::thread::spawn(move || {
            let (result, lines) = check_remote(&*invoke);
            thread
                .queue(move |backend| {
                    backend.finish_check(generation, &current, now, result, lines)
                })
                .ok();
        });
    }

    /// Applies a finished check, unless a newer one has started since.
    fn finish_check(
        mut self: Pin<&mut Self>,
        generation: u64,
        current: &Arc<AtomicU64>,
        now: i64,
        result: Result<Vec<UpdateEntry>, SnapshotError>,
        lines: Vec<String>,
    ) {
        if current.load(Ordering::SeqCst) != generation {
            return;
        }
        // The lines belong to the answer that was accepted: a check a newer one
        // superseded never reaches this point, so the drawer never shows the
        // output of a question nobody is waiting for.
        self.append_check_lines(&lines);
        let age = self.check_age_at(now);
        let status = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.checking = false;
            match result {
                Ok(updates) => {
                    let count = updates.len();
                    data.store.replace_updates(updates.clone());
                    if let Some(snapshot) = data.snapshot.as_mut() {
                        snapshot.updates = updates;
                    }
                    data.updates_origin = UpdatesOrigin::Remote { at: now };
                    data.store.set_pref_last_check_at(now);
                    data.last_check_at = now;
                    data.check_error_key = QString::from("");
                    data.check_error_args = QStringList::default();
                    let outcome = if count == 0 {
                        CheckOutcome::UpToDate
                    } else {
                        CheckOutcome::Updates { count }
                    };
                    check_status(&outcome, &age)
                }
                Err(SnapshotError::Repositories {
                    failed,
                    total,
                    detail,
                    ..
                }) => {
                    data.check_error_key = QString::from("checkRepositoriesFailed");
                    data.check_error_args = QStringList::default();
                    check_status(
                        &CheckOutcome::RepositoriesFailed {
                            failed,
                            total,
                            detail,
                        },
                        &age,
                    )
                }
                Err(err) => {
                    data.check_error_key = QString::from(err.key());
                    data.check_error_args = strings_list(&err.args());
                    check_status(&CheckOutcome::ReadFailed { key: err.key() }, &age)
                }
            }
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.check_key = QString::from(status.key);
            data.check_args = strings_list(&status.args);
            data.checking = false;
        }
        // The store moved (the list, maybe the stamp), so it is saved and the
        // models receive it; a failed check changed nothing and `refresh` only
        // republishes what the window already shows.
        save_store(self.as_mut());
        refresh(self.as_mut());
        self.as_mut().checking_changed();
        self.as_mut().check_key_changed();
        self.as_mut().check_args_changed();
        self.as_mut().check_error_key_changed();
        self.as_mut().check_error_args_changed();
        self.as_mut().last_check_at_changed();
    }

    /// Reports the offline state without starting a process.
    fn finish_check_offline(mut self: Pin<&mut Self>, status: &CheckStatus) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.check_error_key = QString::from("checkOffline");
            data.check_error_args = QStringList::default();
            data.check_key = QString::from(status.key);
            data.check_args = strings_list(&status.args);
        }
        self.as_mut().check_key_changed();
        self.as_mut().check_args_changed();
        self.as_mut().check_error_key_changed();
        self.as_mut().check_error_args_changed();
    }

    /// How old the last successful check is, as the number of milliseconds the
    /// sentence pluralises; `0` when no check ever succeeded.
    fn check_age_at(&self, now: i64) -> String {
        let last = self.store.prefs.last_check_at;
        if last <= 0 || last > now {
            return String::from("0");
        }
        (now - last).to_string()
    }

    /// Applies a finished read, unless a newer one has started since.
    fn finish_read(
        mut self: Pin<&mut Self>,
        generation: u64,
        current: &Arc<AtomicU64>,
        result: Result<Snapshot, SnapshotError>,
    ) {
        if current.load(Ordering::SeqCst) != generation {
            // A newer read owns the window; this answer is stale.
            return;
        }
        let (was_loading, error_key, error_args) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let was_loading = data.loading;
            data.loading = false;
            match &result {
                Ok(_) => (was_loading, String::new(), Vec::new()),
                Err(err) => (was_loading, err.key().to_owned(), err.args()),
            }
        };
        if was_loading {
            self.as_mut().loading_changed();
        }

        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.read_error = QString::from(error_key.as_str());
            data.read_error_args = error_args
                .iter()
                .map(|arg| QString::from(arg.as_str()))
                .collect::<QStringList>();
        }
        self.as_mut().read_error_changed();
        self.as_mut().read_error_args_changed();

        let applied = {
            let now = now_ms();
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // A fresh remote list outranks this read: the local index cannot
            // see anything the check did not already see, so replacing it here
            // would make the repositories look older than they are.
            let keep_remote = !local_replaces(data.updates_origin, now, AUTO_GAP_MS);
            let result = match result {
                Ok(mut snapshot) if keep_remote => {
                    snapshot.updates = data.store.pending_updates().to_vec();
                    Ok(snapshot)
                }
                other => other,
            };
            let applied = apply_read(&mut data.store, &mut data.snapshot, result);
            if applied && !keep_remote {
                data.updates_origin = UpdatesOrigin::Local;
            }
            applied
        };
        if applied {
            // The installed set changed, and the next start shows it before a
            // new read answers. A read that failed before anything was read
            // leaves the file as it is.
            save_store(self.as_mut());
        }
        refresh(self.as_mut());
    }

    fn apply_plan_items(mut self: Pin<&mut Self>, items_json: &cxx_qt_lib::QString) {
        let items: Vec<PlanItem> =
            serde_json::from_str(&items_json.to_string()).unwrap_or_default();
        if items.is_empty() {
            return;
        }
        let now = now_ms();
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.apply_plan(
                &Plan {
                    items: items.clone(),
                    blocked: Vec::new(),
                },
                now,
            );
            // The check that produced the list predates the transaction: it
            // described packages that are installed now. A local read taken
            // after it is newer information, so it must be allowed to replace
            // the list instead of being dropped as older than the check.
            data.updates_origin = UpdatesOrigin::Local;
            // The read is asynchronous; until it lands the snapshot is what
            // the window shows, so the applied entries leave it now rather
            // than surviving as rows that offer the same update again.
            if let Some(snapshot) = data.snapshot.as_mut() {
                drop_applied_updates(&mut snapshot.updates, &items);
            }
            // `runJob` clears the dismissed-update preference once nothing is
            // left to announce, so a later batch of updates is not swallowed.
            // The list is the one the last read reported, minus the packages
            // the user held since.
            let pending = effective_updates(data.store.pending_updates(), &data.store.installed);
            if pending.is_empty() {
                let signature = update_signature_of(&pending).to_string();
                data.store.set_pref_dismissed_version(&signature);
            }
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
        // The plan says what the transaction meant to do; only xbps says what
        // it did. Without a fresh read the updated package stays on the
        // updates list with its old version until the user refreshes by hand.
        self.as_mut().reload();
    }

    /// The backend as a `QObject*`, for the C++ child factories.
    fn parent_ptr(&self) -> QObjectMutPtr {
        unsafe {
            QObjectMutPtr::from_raw(
                <qobject::InstalledBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(self).cast_mut(),
            )
        }
    }
}

/// Reads the repository count and then the fresh update list.
///
/// Runs on the worker thread: two processes, the second the network one. The
/// count is only for the sentence, so a listing this version cannot read costs
/// the number and not the check.
fn check_remote(invoke: &crate::xbps_snapshot::Invoke) -> CheckAnswer {
    // The check is two commands: the repository count, then the update list.
    // Both print to the log, because both are xbps runs the user asked for.
    let mut lines = Vec::new();
    let repos = repos_argv();
    let total = match run(&repos, READ_TIMEOUT_MS, invoke) {
        Ok(output) => {
            lines.extend(command_lines(&repos, &output));
            if output.success() {
                parse_repos(&output.stdout)
                    .map(|entries| entries.len())
                    .unwrap_or(0)
            } else {
                0
            }
        }
        Err(_) => 0,
    };
    let updates = read_remote_updates_logged(invoke, total);
    lines.extend(updates.lines);
    (updates.result, lines)
}

/// What one update check answers: the list or the failure, plus the lines the
/// drawer shows.
type CheckAnswer = (Result<Vec<UpdateEntry>, SnapshotError>, Vec<String>);

/// What one `open*` asks the worker to plan, and how it becomes a payload.
///
/// The variants carry exactly what the dry runs need; the installed set, the
/// installed set and the enabled repositories are copied at spawn time, so a plan
/// always describes the system the queue was built against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanTask {
    /// Installs and updates first, then removals, as the queue asks.
    Queue(Vec<QueueItem>),
    /// `xbps-install -n -Su`.
    System,
    /// `xbps-install -n <names>` under the caller's dialog title.
    Named {
        names: Vec<String>,
        title_key: String,
    },
    /// `xbps-remove -n <names>`; `cascade` only changes the dialog title.
    Remove { names: Vec<String>, cascade: bool },
}

impl PlanTask {
    /// The task of `openQueuePlan`.
    pub fn queue(store: &Store) -> Self {
        Self::Queue(store.queue.clone())
    }

    /// The task of `openSystemPlan`.
    pub fn system() -> Self {
        Self::System
    }

    /// The task of `openNamedPlan`.
    pub fn named(names: Vec<String>, title_key: String) -> Self {
        Self::Named { names, title_key }
    }

    /// The task of `openRemoveNames`.
    pub fn remove(names: Vec<String>, cascade: bool) -> Self {
        Self::Remove { names, cascade }
    }

    /// Runs the task's dry runs and returns the payload the dialog shows.
    ///
    /// Runs on the worker thread; every process call goes through `invoke`, so
    /// this is testable without a GUI and the suite never starts xbps.
    pub fn run(
        &self,
        installed: &InstalledMap,
        invoke: &crate::xbps_snapshot::Invoke,
    ) -> PlanPayload {
        match self {
            Self::System => xbps_plan::system_payload(installed, invoke),
            Self::Named { names, title_key } => named_payload(installed, names, title_key, invoke),
            Self::Remove { names, cascade } => {
                xbps_plan::remove_payload(installed, names, *cascade, invoke)
            }
            Self::Queue(queue) => queue_payload(installed, queue, invoke),
        }
    }
}

/// What a finished plan reports back: the request it answers.
///
/// The title, its arguments and the user's names travel inside the payload, so
/// QML builds the dialog from one object and nothing can disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanMeta {
    pub request: u64,
}

/// Plan payload for the transaction queue.
///
/// Installs and updates come from one `xbps-install -n`, removals from one
/// `xbps-remove -n`. A blocked removal wins and discards the steps: the dialog
/// must not offer Apply for a transaction xbps refuses.
pub fn queue_payload(
    installed: &InstalledMap,
    queue: &[QueueItem],
    invoke: &crate::xbps_snapshot::Invoke,
) -> PlanPayload {
    if queue.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    let missing = not_installed(queue, installed);
    if !missing.is_empty() {
        return PlanPayload::from_error(&xbps_plan::PlanError::NotInstalled(missing));
    }
    // `xbps-install` upgrades a package the machine already holds, so an
    // install of one is the same command as an update; the plan and the log
    // must say "update". The rows queue the right action themselves, and this
    // relabels what an older build may have left behind.
    let queue = installs_as_updates(queue, installed);
    if queue.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    let payload = xbps_plan::queue_payload(installed, &queue, invoke);
    let mut payload = payload;
    // The held note concerns the packages the queue actually updates; xbps
    // neither holds nor reports them, so it is added here.
    let updates = installed
        .keys()
        .filter(|name| queue.iter().any(|item| &item.name == *name))
        .count();
    if updates > 0 && payload.note_key.is_empty() {
        let held = held_in_queue(
            &installed
                .iter()
                .filter(|(_, inst)| inst.held)
                .map(|(name, inst)| UpdateEntry {
                    name: name.clone(),
                    version: inst.version.clone(),
                    pkgver: format!("{name}-{}", inst.version),
                    action: kxbps_core::XbpsAction::Update,
                    arch: String::new(),
                    repository: String::new(),
                    installed_size: 0,
                    download_size: 0,
                })
                .collect::<Vec<_>>(),
            installed,
            &queue,
        );
        if !held.is_empty() {
            payload.note_key = plan_keys::NOTE_HELD.to_string();
            payload.note_args = vec![held.join(", ")];
        }
    }
    payload
}

/// Plan payload for "install exactly these packages".
pub fn named_payload(
    installed: &InstalledMap,
    names: &[String],
    title_key: &str,
    invoke: &crate::xbps_snapshot::Invoke,
) -> PlanPayload {
    xbps_plan::named_payload(installed, names, title_key, invoke)
}

/// Held packages that the queue actually wants to update.
///
/// [`held_updates_in`] returns every held package with a reported update; the
/// note on a queue plan only concerns the ones the user queued.
pub fn held_in_queue(
    updates: &[UpdateEntry],
    installed: &InstalledMap,
    queue: &[QueueItem],
) -> Vec<String> {
    let updating: Vec<&str> = queue
        .iter()
        .filter(|item| item.action == QueueAction::Update)
        .map(|item| item.name.as_str())
        .collect();
    held_updates_in(updates, installed)
        .into_iter()
        .filter(|name| updating.contains(&name.as_str()))
        .collect()
}

/// Pushes a snapshot into the query façade; a missing backend is not an error.
fn push_query_state(
    query: QObjectMutPtr,
    installed_json: &QString,
    queue_json: &QString,
    updates_json: &QString,
    orphans_json: &QString,
) {
    if !query.as_ptr().is_null() {
        qobject::query_backend_set_state(
            query,
            installed_json,
            queue_json,
            updates_json,
            orphans_json,
        );
    }
}

/// The queue as the model reads it: the action plus the version and the bytes
/// the read knows about. A queued install of a package the machine does not
/// hold has neither, and the row shows a dash instead of a guess.
fn queue_rows(store: &Store, snapshot: Option<&Snapshot>) -> Vec<crate::queue_model::QueueRow> {
    store
        .queue
        .iter()
        .map(|item| {
            let installed = store.installed.get(&item.name);
            let version = match item.action {
                QueueAction::Update => snapshot
                    .and_then(|snapshot| snapshot.update_of(&item.name))
                    .map(str::to_owned)
                    .or_else(|| installed.map(|inst| inst.version.clone())),
                _ => installed.map(|inst| inst.version.clone()),
            }
            .unwrap_or_default();
            let bytes = match item.action {
                QueueAction::Remove => {
                    snapshot.and_then(|snapshot| size_in(&snapshot.installed_sizes, &item.name))
                }
                _ => {
                    snapshot.and_then(|snapshot| size_in(&snapshot.repo_download_sizes, &item.name))
                }
            };
            crate::queue_model::QueueRow {
                name: item.name.clone(),
                action: item.action,
                version,
                size: bytes
                    .map(crate::package_model::format_size)
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn size_in(entries: &[kxbps_core::SizeEntry], name: &str) -> Option<u64> {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.bytes)
}

/// The installed set, queue, updates and orphans as the shims read them.
fn state_json(store: &Store, snapshot: Option<&Snapshot>) -> (QString, QString, QString, QString) {
    let updates = snapshot.map_or_else(Vec::new, |snapshot| snapshot.updates.clone());
    let orphans = snapshot.map_or_else(Vec::new, |snapshot| snapshot.orphans.clone());
    (
        json_of(&store.installed),
        json_of(&queue_rows(store, snapshot)),
        json_of(&updates),
        json_of(&orphans),
    )
}

/// Puts a finished read into the store and remembers the snapshot itself.
///
/// A successful read keeps the install dates the store already knew and the
/// queue; the snapshot keeps the updates and the orphans, which nothing else
/// reports. A failed read empties the installed set, because rows from an
/// earlier read no longer describe the system and a stale list reads as a
/// healthy one; the caller shows the error key instead.
///
/// Returns whether the store holds a new installed set and should be saved: a
/// successful read always does, a failed one only when it had something to
/// empty.
pub fn apply_read(
    store: &mut Store,
    snapshot: &mut Option<Snapshot>,
    result: Result<Snapshot, SnapshotError>,
) -> bool {
    match result {
        Ok(read) => {
            store.apply_snapshot(
                &read.installed,
                &read.explicit,
                &read.held,
                read.updates.clone(),
            );
            *snapshot = Some(read);
            true
        }
        Err(_) => {
            if store.installed.is_empty() && snapshot.is_none() {
                return false;
            }
            store.apply_snapshot(&[], &[], &[], Vec::new());
            *snapshot = None;
            true
        }
    }
}

/// The pending-update fingerprint the notification decision compares against
/// the dismissed marker.
///
/// The same expression `apply_plan` writes into `prefs.dismissed` when nothing
/// is left, so the two can never disagree about which set the user saw: the
/// effective updates (xbps' list minus the packages held in this window) hashed
/// with FNV-1a. An empty set hashes to the same non-empty string every time;
/// "nothing read yet" is an empty *store*, which this function reports as the
/// empty string, so QML can tell "no signature" from "a signature of nothing".
fn pending_signature_of(store: &Store) -> String {
    let pending = effective_updates(store.pending_updates(), &store.installed);
    if pending.is_empty() && store.installed.is_empty() {
        return String::new();
    }
    update_signature_of(&pending).to_string()
}

/// Pushes everything a finished read produced into the models and counters.
///
/// This is the read path: a new snapshot was taken, so the model gets all of
/// it — the records with their descriptions, the sizes, the updates and the
/// orphans — and the list is rebuilt from xbps rather than from the store.
fn refresh(mut self_: Pin<&mut qobject::InstalledBackend>) {
    refresh_models(self_.as_mut(), true)
}

/// Pushes a store-only change (a queue toggle, a hold) into the models.
///
/// No snapshot is shipped: nothing about xbps moved, so the model keeps the
/// records, the descriptions and the derived update/orphan state it already
/// has and only re-reads the installed map and the queue. Shipping the whole
/// snapshot here would re-serialise and re-parse 1340 records through serde on
/// every click for nothing.
///
/// `apply_plan_items` deliberately stays on [`refresh`]: a finished job may
/// have changed what is installed, and `store_state` rebuilds records from the
/// map without descriptions — the read path is the one that carries them.
fn refresh_store(mut self_: Pin<&mut qobject::InstalledBackend>) {
    refresh_models(self_.as_mut(), false)
}

/// The shared body of [`refresh`] and [`refresh_store`].
///
/// `with_snapshot` decides whether the model receives the full snapshot push
/// or the store-only one; every other step (counters, query façade, revision)
/// is the same for both.
fn refresh_models(mut self_: Pin<&mut qobject::InstalledBackend>, with_snapshot: bool) {
    let (
        installed_json,
        queue_json,
        updates_json,
        orphans_json,
        installed_count,
        queue_count,
        updates_count,
        pending_signature,
        packages,
        queue_model,
        query,
        snapshot_json,
    ) = {
        let data = &*self_;
        let (installed_json, queue_json, updates_json, orphans_json) =
            state_json(&data.store, data.snapshot.as_ref());
        (
            installed_json,
            queue_json,
            updates_json,
            orphans_json,
            i32::try_from(data.store.installed.len()).unwrap_or(i32::MAX),
            i32::try_from(data.store.queue.len()).unwrap_or(i32::MAX),
            i32::try_from(data.store.pending_updates().len()).unwrap_or(i32::MAX),
            pending_signature_of(&data.store),
            data.packages,
            data.queue_model,
            data.query_backend,
            if with_snapshot {
                data.snapshot.as_ref().map(json_of)
            } else {
                None
            },
        )
    };

    if !packages.as_ptr().is_null() {
        match &snapshot_json {
            // The model derives `explicit`/`held` from the snapshot, so the
            // list and the counters cannot disagree.
            Some(snapshot_json) => {
                qobject::package_model_set_snapshot(packages, snapshot_json, &queue_json)
            }
            // No read yet: only the store moved (queue, hold), so the installed
            // set it holds is all the model needs to update.
            None => qobject::package_model_set_state(packages, &installed_json, &queue_json),
        }
    }
    if !queue_model.as_ptr().is_null() {
        qobject::queue_model_set_state(queue_model, &queue_json);
    }
    push_query_state(
        query,
        &installed_json,
        &queue_json,
        &updates_json,
        &orphans_json,
    );

    let (installed_changed, queue_changed, updates_changed, signature_changed) = {
        let data = Pin::get_mut(self_.as_mut().rust_mut());
        let changed = (
            data.installed_count != installed_count,
            data.queue_count != queue_count,
            data.updates_count != updates_count,
            data.pending_signature.to_string() != pending_signature,
        );
        data.installed_count = installed_count;
        data.queue_count = queue_count;
        data.updates_count = updates_count;
        data.pending_signature = QString::from(pending_signature.as_str());
        changed
    };
    if installed_changed {
        self_.as_mut().installed_count_changed();
    }
    if queue_changed {
        self_.as_mut().queue_count_changed();
    }
    if updates_changed {
        self_.as_mut().updates_count_changed();
    }
    // Before the revision: QML's start-only hook listens to this one, and the
    // dialog-facing revision must not see a half-updated object.
    if signature_changed {
        self_.as_mut().pending_signature_changed();
    }

    // Unconditional: a hold toggle changes neither counter, but it does change
    // what `get()` returns and what `-H` must list.
    {
        let data = Pin::get_mut(self_.as_mut().rust_mut());
        data.state_revision = next_revision(data.state_revision);
    }
    self_.as_mut().state_revision_changed();
}

/// Next value of the state revision counter; wraps instead of overflowing.
fn next_revision(current: i32) -> i32 {
    current.wrapping_add(1)
}

/// Offers one announcement and records it, for [`qobject::InstalledBackend::take_announcement`].
///
/// The decision is [`crate::notification::announcement`]; this only turns it
/// into the JSON QML renders and writes the bookkeeping the next call reads
/// back. `None` writes nothing, so a deferred announcement (a visible window, a
/// switch turned off) is offered again later instead of being lost.
///
/// The start switch gates the start offer, a deferred retry of it included, and
/// the desktop switch gates the popups as a whole; a check the app runs while it
/// is open is not gated by the start switch.
///
/// The payload carries keys, not sentences: QML picks the plural form, because
/// the plural rule belongs to the translation.
fn take_announcement_with(store: &mut Store, now: i64, visible: bool, startup: bool) -> String {
    let signature = pending_signature_of(store);
    let count = effective_updates(store.pending_updates(), &store.installed).len();
    let state = crate::notification::AnnounceState {
        signature: signature.clone(),
        dismissed: store.prefs.dismissed.clone(),
        reminded: store.prefs.reminded.clone(),
        notified_at: store.prefs.notified_at,
        now,
        count,
        desktop_notifications: store.prefs.desktop,
        window_visible: visible,
        announce_on_start: store.prefs.notify_on_start,
        startup,
    };
    let Some(kind) = crate::notification::announcement(&state) else {
        return String::new();
    };
    match kind {
        crate::notification::Announcement::First => {
            store.set_pref_dismissed_version(&signature);
            store.set_pref_reminded_version("");
            store.set_pref_notified_at(now);
        }
        crate::notification::Announcement::Reminder => {
            store.set_pref_reminded_version(&signature);
        }
    }
    let payload = crate::notification::payload_for(kind, count);
    // Only the kind and the count: the sentences are QML's, so the extractor
    // finds them and every language gets them.
    serde_json::json!({
        "kind": match payload.kind {
            crate::notification::Announcement::First => "first",
            crate::notification::Announcement::Reminder => "reminder",
        },
        "count": payload.count,
        "signature": signature,
    })
    .to_string()
}

/// Writes the store, retrying once before reporting a technical error.
fn save_store(mut self_: Pin<&mut qobject::InstalledBackend>) {
    let result = {
        let data = Pin::get_mut(self_.as_mut().rust_mut());
        match data.store.save() {
            Ok(()) => Ok(()),
            Err(_) => data.store.save(),
        }
    };
    if let Err(err) = result {
        let (key, args) = store_error_parts(&err);
        set_store_error(self_.as_mut(), &key, &args);
    }
}

/// Splits a [`StoreError`] into the key QML maps and the arguments of its
/// sentence.
///
/// The `Display` text (a path glued to an OS or parser reason) is not
/// translatable, so the window never sees it: this pair is what `error` and
/// `errorArgs` carry. `Display` stays available for the log.
fn store_error_parts(error: &StoreError) -> (String, Vec<String>) {
    (error.key().to_owned(), error.args())
}

/// Writes the save error into `error`/`errorArgs` and notifies QML.
fn set_store_error(mut self_: Pin<&mut qobject::InstalledBackend>, key: &str, args: &[String]) {
    {
        let data = Pin::get_mut(self_.as_mut().rust_mut());
        data.error = QString::from(key);
        data.error_args = args
            .iter()
            .map(|arg| QString::from(arg.as_str()))
            .collect::<QStringList>();
    }
    self_.as_mut().error_changed();
    self_.as_mut().error_args_changed();
}

/// The Qt list form of a sentence's arguments, for the properties QML reads.
fn strings_list(args: &[String]) -> QStringList {
    args.iter()
        .map(|arg| QString::from(arg.as_str()))
        .collect::<QStringList>()
}

fn strings_of(list: &cxx_qt_lib::QStringList) -> Vec<String> {
    list.iter().map(QString::to_string).collect()
}

fn json_of<T: serde::Serialize>(value: &T) -> QString {
    let json = serde_json::to_string(value).unwrap_or_else(|_| String::from("null"));
    QString::from(json.as_str())
}

/// The action the queue holds for `name`, as the token QML reads.
fn queued_action_in(queue: &[QueueItem], name: &str) -> Option<&'static str> {
    queue
        .iter()
        .find(|item| item.name == name)
        .map(|item| action_token(item.action))
}

/// The QML token for a queued action, the inverse of [`parse_action`].
fn action_token(action: QueueAction) -> &'static str {
    match action {
        QueueAction::Remove => "remove",
        QueueAction::Update => "update",
        QueueAction::Install => "install",
    }
}

fn parse_action(token: &str) -> QueueAction {
    match token {
        "remove" => QueueAction::Remove,
        "update" => QueueAction::Update,
        _ => QueueAction::Install,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_model::VisibleRow;
    use crate::xbps_plan::PlanError;
    use kxbps_core::{Inst, PlanAction, PlanReason, XbpsAction};
    use std::sync::{Arc, Mutex};

    /// A store error becomes the key QML maps plus the arguments its sentence
    /// takes, never the `Display` text: a failed save must not surface as an
    /// untranslated English sentence in a Polish build.
    #[test]
    fn a_save_error_reaches_qml_as_a_key_and_its_arguments() {
        let error = StoreError::Io {
            path: std::path::PathBuf::from("/home/u/.local/share/kxbps/queue.json"),
            source: std::io::Error::other("Read-only file system"),
        };
        let (key, args) = store_error_parts(&error);
        assert_eq!(key, "errorStoreWrite");
        assert_eq!(
            args,
            vec![
                String::from("/home/u/.local/share/kxbps/queue.json"),
                String::from("Read-only file system"),
            ]
        );
    }

    #[test]
    fn a_missing_data_directory_reaches_qml_without_arguments() {
        let (key, args) = store_error_parts(&StoreError::NoDataDir);
        assert_eq!(key, "errorStoreNoDataDir");
        assert!(args.is_empty());
    }

    /// A small installed set: the packages the plan tests talk about.
    fn installed_set() -> InstalledMap {
        [
            ("acpid", "2.0.34_4"),
            ("ark", "25.07.0_1"),
            ("bash", "5.2_1"),
            ("glibc", "2.41_1"),
            ("mesa", "25.1_1"),
            ("vim", "9.2.1031_1"),
        ]
        .into_iter()
        .map(|(name, version)| {
            (
                name.to_owned(),
                Inst {
                    version: version.to_owned(),
                    explicit: true,
                    held: false,
                    at: 7,
                },
            )
        })
        .collect()
    }

    fn queue(name: &str, action: QueueAction) -> Vec<QueueItem> {
        vec![QueueItem {
            name: name.into(),
            action,
        }]
    }

    fn update_entry(name: &str, version: &str) -> UpdateEntry {
        UpdateEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: kxbps_core::xbps::pkgver(name, version),
            action: XbpsAction::Update,
            arch: "x86_64".to_owned(),
            repository: "https://repo.example/current".to_owned(),
            installed_size: 1,
            download_size: 1,
        }
    }

    /// The argv lists a fake transport was asked to run.
    type Seen = Arc<Mutex<Vec<Vec<String>>>>;

    /// A transport that answers every request with one canned reply and
    /// remembers the argv it was asked for.
    fn one_answer(reply: &str) -> (SharedInvoke, Seen) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let reply = reply.to_owned();
        let invoke: SharedInvoke = Arc::new(move |request: &str, _: u32| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            recorder.lock().map(|mut seen| seen.push(argv)).ok();
            reply.clone()
        });
        (invoke, seen)
    }

    fn ok_reply(stdout: &str) -> String {
        serde_json::json!({
            "exitCode": 0,
            "stdout": stdout,
            "stderr": "",
            "timedOut": false,
        })
        .to_string()
    }

    fn exit_reply(exit_code: i32, stdout: &str, stderr: &str) -> String {
        serde_json::json!({
            "exitCode": exit_code,
            "stdout": stdout,
            "stderr": stderr,
            "timedOut": false,
        })
        .to_string()
    }

    #[test]
    fn the_queue_plan_is_the_dependency_install_xbps_reported() {
        // `xbps-install -n ark` answers with the outdated dependency first,
        // then the package the user asked for; the dialog shows exactly that.
        let installed = installed_set();
        let (invoke, seen) = one_answer(&ok_reply(
            "mesa-25.2_1 update x86_64 https://r/current 4194304 1468006\n\
             ark-25.08.0_1 install x86_64 https://r/current 2097152 524288\n",
        ));
        let payload = queue_payload(&installed, &queue("ark", QueueAction::Install), &*invoke);
        assert_eq!(payload.title_key, "planQueue");
        assert!(payload.error_key.is_empty());
        assert!(payload.notice_key.is_empty());
        assert_eq!(payload.items.len(), 2, "xbps reported two steps");
        assert_eq!(payload.items[0].action, PlanAction::Update);
        assert_eq!(payload.items[0].reason, PlanReason::Dependency);
        assert_eq!(payload.items[1].name, "ark");
        assert_eq!(payload.items[1].reason, PlanReason::Requested);
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-install", "-M", "-n", "ark"]
        );
    }

    #[test]
    fn a_queue_with_nothing_to_do_is_a_notice() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(""));
        let payload = queue_payload(&installed, &queue("ark", QueueAction::Install), &*invoke);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert!(payload.items.is_empty());
    }

    /// xbps names the queued packages it will not touch because the machine
    /// already has them. The payload carries those names so the backend can
    /// drop them from the queue; otherwise the row stays queued for ever and
    /// Apply keeps saying there is nothing to do.
    #[test]
    fn a_queue_of_installed_packages_names_what_xbps_left_alone() {
        let installed = installed_set();
        let (invoke, _) = one_answer(
            &serde_json::json!({
                "exitCode": 0,
                "stdout": "",
                "stderr": "ERROR: Package `ark' already installed.\n\
                           ERROR: Package `vim' already installed.",
                "timedOut": false,
            })
            .to_string(),
        );
        let payload = queue_payload(&installed, &queue("ark", QueueAction::Update), &*invoke);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert_eq!(payload.satisfied, ["ark", "vim"]);
    }

    /// The queue's Apply runs the dry run without a dialog, so a payload with
    /// no steps must always carry the reason. QML shows a key it does not get
    /// as nothing at all, and Apply would then look like a dead button.
    #[test]
    fn a_payload_without_steps_always_names_why() {
        let installed = installed_set();
        let cases = [
            // xbps answered, and the transaction is already done.
            (ok_reply(""), "emptyQueue"),
            // xbps refused the name.
            (
                exit_reply(2, "", "ERROR: Package 'nope' not found in repository pool."),
                "errorUnknownPackage",
            ),
        ];
        for (reply, expected) in cases {
            let (invoke, _) = one_answer(&reply);
            let payload = queue_payload(&installed, &queue("nope", QueueAction::Install), &*invoke);
            assert!(payload.items.is_empty(), "{expected} carries no steps");
            let key = if payload.notice_key.is_empty() {
                &payload.error_key
            } else {
                &payload.notice_key
            };
            assert_eq!(key, expected, "a payload with no steps names why");
        }
    }

    #[test]
    fn an_install_of_an_installed_package_is_a_notice_when_nothing_is_newer() {
        let installed = installed_set();
        let (invoke, seen) = one_answer(&ok_reply(""));
        let payload = queue_payload(&installed, &queue("vim", QueueAction::Install), &*invoke);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert!(payload.items.is_empty());
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-install", "-M", "-n", "vim"],
            "the dry run is asked, so the plan says there is nothing to do"
        );
    }

    #[test]
    fn an_installed_install_is_planned_as_an_update() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "vim-9.2.1032_1 update x86_64 https://r/current 4194304 1468006\n",
        ));
        let payload = queue_payload(&installed, &queue("vim", QueueAction::Install), &*invoke);
        assert_eq!(payload.items.len(), 1);
        assert_eq!(payload.items[0].name, "vim");
        assert_eq!(
            payload.items[0].action,
            PlanAction::Update,
            "an installed package is upgraded, never installed again"
        );
    }

    #[test]
    fn an_update_of_a_package_that_is_not_installed_never_runs_xbps() {
        let installed = installed_set();
        let (invoke, seen) = one_answer(&ok_reply(""));
        let payload = queue_payload(&installed, &queue("ghost", QueueAction::Update), &*invoke);
        assert_eq!(payload.error_key, "errorNotInstalled");
        assert!(seen.lock().map(|seen| seen.is_empty()).unwrap());
    }

    /// A read snapshot over `installed_set()`, the shape `refresh` ships.
    fn store_snapshot(installed: &InstalledMap) -> Snapshot {
        use kxbps_core::InstallState;
        let records: Vec<kxbps_core::InstalledPackage> = installed
            .iter()
            .map(|(name, inst)| kxbps_core::InstalledPackage {
                name: name.clone(),
                version: inst.version.clone(),
                pkgver: format!("{name}-{}", inst.version),
                state: InstallState::Installed,
                description: format!("The {name} package"),
            })
            .collect();
        Snapshot {
            installed: records,
            updates: vec![update_entry("mesa", "25.2_1")],
            explicit: installed.keys().cloned().collect(),
            held: Vec::new(),
            orphans: vec![String::from("zramen")],
            installed_sizes: Vec::new(),
            repo_installed_sizes: Vec::new(),
            repo_download_sizes: Vec::new(),
        }
    }

    /// A store holding `installed_set()` with one queued package.
    fn store_with(installed: &InstalledMap, queue: Vec<QueueItem>) -> Store {
        let mut store = Store::fresh();
        store.installed = installed.clone();
        store.queue = queue;
        store
    }

    /// A model that has just taken `refresh`'s snapshot push.
    fn snapshot_model(
        store: &Store,
        snapshot: &Snapshot,
    ) -> crate::package_model::PackageModelRust {
        let mut model = crate::package_model::PackageModelRust::default();
        let installed = kxbps_core::installed_map(
            &snapshot.installed,
            &snapshot.explicit,
            &snapshot.held,
            &|_| 0,
        );
        let (_, rows, _) = model.snapshot(
            snapshot.installed.clone(),
            installed,
            store.queue.clone(),
            snapshot.updates.clone(),
            snapshot.orphans.clone(),
        );
        model.assign_rows(rows);
        model
    }

    /// The store-only push, into a model that already took a snapshot push:
    /// the production order, where `refresh_store` follows a read.
    fn store_push(store: &Store, snapshot: &Snapshot) -> (Vec<u32>, Vec<VisibleRow>) {
        let mut model = snapshot_model(store, snapshot);
        let (installed_json, queue_json, _, _) = state_json(store, None);
        let installed: InstalledMap =
            serde_json::from_str(&installed_json.to_string()).expect("installed decodes");
        let queue: Vec<QueueItem> =
            serde_json::from_str(&queue_json.to_string()).expect("queue decodes");
        let (_, rows, _) = model.store_state(installed, queue);
        model.assign_rows(rows.clone());
        (rows, model.visible_rows())
    }

    /// The same click through the old path: a full snapshot push.
    fn full_push(store: &Store, snapshot: &Snapshot) -> (Vec<u32>, Vec<VisibleRow>) {
        let model = snapshot_model(store, snapshot);
        (model.row_indices(), model.visible_rows())
    }

    #[test]
    fn a_queue_toggle_reads_the_same_through_the_store_path() {
        // A queue click now pushes the store, not the snapshot; the list must
        // show exactly what the snapshot path showed.
        let installed = installed_set();
        let snapshot = store_snapshot(&installed);
        let mut store = store_with(&installed, Vec::new());
        store.toggle_queue("ark", QueueAction::Install);

        assert_eq!(
            store_push(&store, &snapshot),
            full_push(&store, &snapshot),
            "a queue toggle looks the same either way"
        );
    }

    #[test]
    fn a_hold_toggle_reads_the_same_through_the_store_path() {
        let installed = installed_set();
        let snapshot = store_snapshot(&installed);
        let mut store = store_with(&installed, Vec::new());
        store.toggle_hold("mesa");

        assert_eq!(
            store_push(&store, &snapshot),
            full_push(&store, &snapshot),
            "a hold toggle looks the same either way"
        );
    }

    #[test]
    fn a_store_only_push_keeps_the_updates_and_the_orphans() {
        // The store path has no snapshot to ship, so the model keeps the
        // updates and orphans the last read printed.
        let installed = installed_set();
        let snapshot = store_snapshot(&installed);
        let store = store_with(&installed, Vec::new());
        let mut model = snapshot_model(&store, &snapshot);

        let (installed_json, queue_json, _, _) = state_json(&store, None);
        let map: InstalledMap = serde_json::from_str(&installed_json.to_string()).unwrap();
        let queue: Vec<QueueItem> = serde_json::from_str(&queue_json.to_string()).unwrap();
        model.store_state(map, queue);

        model.set_filter("updates");
        assert!(
            model.visible_names().contains(&"mesa"),
            "the pending update survives a store-only refresh"
        );
        assert!(
            model.is_orphan_for_test("zramen"),
            "the orphan list survives a store-only refresh"
        );
    }

    #[test]
    fn applying_the_plan_adds_exactly_one_package() {
        // The plan installs `ark`, so the system starts without it.
        let mut installed = installed_set();
        installed.remove("ark");
        assert_eq!(installed.len(), 5, "the fixture set without ark");
        let (invoke, _) = one_answer(&ok_reply(
            "ark-25.08.0_1 install x86_64 https://r/current 2097152 524288\n",
        ));
        let payload = queue_payload(&installed, &queue("ark", QueueAction::Install), &*invoke);
        let mut store = Store::fresh();
        store.installed = installed.clone();
        store.queue = queue("ark", QueueAction::Install);
        store.apply_plan(
            &Plan {
                items: payload.items.clone(),
                blocked: Vec::new(),
            },
            7,
        );
        assert_eq!(store.installed.len(), 6, "no package but ark changes");
        assert_eq!(
            store.installed["ark"].version, "25.08.0_1",
            "the installed record carries the planned version"
        );
        assert!(store.installed.contains_key("ark"));
        assert!(
            store.queue.is_empty(),
            "the applied package leaves the queue"
        );
    }

    #[test]
    fn reinstalling_an_installed_package_keeps_the_installed_count() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "vim-9.2.1031_1 reinstall x86_64 https://r/current 4194304 1468006\n",
        ));
        let payload = queue_payload(&installed, &queue("vim", QueueAction::Install), &*invoke);
        assert_eq!(payload.items.len(), 1);
        assert_eq!(payload.items[0].action, PlanAction::Reinstall);

        let mut store = Store::fresh();
        store.installed = installed.clone();
        store.apply_plan(
            &Plan {
                items: payload.items,
                blocked: Vec::new(),
            },
            7,
        );
        assert_eq!(store.installed.len(), 6, "a reinstall changes no package");
    }

    #[test]
    fn the_system_plan_is_the_update_all_dry_run() {
        let installed = installed_set();
        let (invoke, seen) = one_answer(&ok_reply(
            "vim-9.2.2000_1 update x86_64 https://r/current 4194304 1468006\n",
        ));
        let payload = crate::xbps_plan::system_payload(&installed, &*invoke);
        assert_eq!(payload.title_key, "planSystem");
        assert_eq!(payload.items.len(), 1);
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-install", "-M", "-n", "-Su"]
        );
    }

    #[test]
    fn an_up_to_date_system_is_a_notice() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(""));
        let payload = crate::xbps_plan::system_payload(&installed, &*invoke);
        assert_eq!(payload.notice_key, "emptySystem");
    }

    #[test]
    fn removing_a_needed_package_reports_the_dry_run_refusal() {
        // `xbps-remove -n acpid` exits 19 and names what breaks; that refusal
        // is the only source of `blocked`.
        let installed = installed_set();
        let (invoke, seen) = one_answer(&exit_reply(
            19,
            "",
            "acpid-2.0.34_4 in transaction breaks installed pkg 'base-system-0.114_2'\n\
             ERROR: Transaction aborted due to unresolved dependencies.\n",
        ));
        let names = vec!["acpid".to_string()];
        let payload = crate::xbps_plan::remove_payload(&installed, &names, false, &*invoke);
        assert!(payload.items.is_empty(), "a blocked plan has no steps");
        assert_eq!(payload.blocked.len(), 1);
        assert_eq!(payload.blocked[0].name, "acpid");
        assert_eq!(
            payload.blocked[0].needed_by,
            vec!["base-system".to_string()]
        );
        assert_eq!(payload.names, names);
        assert_eq!(payload.title_key, "planRemove");
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-remove", "-n", "acpid"]
        );
    }

    #[test]
    fn a_cascade_removal_is_a_plain_removal_plan_with_its_own_title() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "glibc-2.41_1 remove x86_64 https://r/current 0 848\n\
             bash-5.2_1 remove x86_64 https://r/current 8100000 0\n",
        ));
        let names = vec!["glibc".to_string(), "bash".to_string()];
        let payload = crate::xbps_plan::remove_payload(&installed, &names, true, &*invoke);
        assert!(payload.blocked.is_empty());
        assert!(
            payload
                .items
                .iter()
                .all(|item| item.action == PlanAction::Remove),
            "a cascade only removes"
        );
        assert_eq!(payload.title_key, "planCascade");
    }

    #[test]
    fn removing_an_uninstalled_package_is_a_notice() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&exit_reply(
            2,
            "",
            "Package `nonexistent' is not currently installed.",
        ));
        let payload = crate::xbps_plan::remove_payload(
            &installed,
            &["nonexistent".to_string()],
            false,
            &*invoke,
        );
        assert_eq!(payload.notice_key, "emptyRemove");
        assert!(payload.items.is_empty());
    }

    #[test]
    fn named_plan_keeps_the_callers_title_key() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "ark-25.08.0_1 install x86_64 https://r/current 2097152 524288\n",
        ));
        let payload = named_payload(&installed, &["ark".to_string()], "planInstall", &*invoke);
        assert_eq!(payload.title_key, "planInstall");
        assert!(payload.items.iter().any(|item| item.name == "ark"));
    }

    #[test]
    fn unknown_packages_become_an_error_key() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&exit_reply(
            2,
            "",
            "ERROR: Package 'nope' not found in repository pool.",
        ));
        let payload = named_payload(&installed, &["nope".to_string()], "planInstall", &*invoke);
        assert_eq!(payload.error_key, "errorUnknownPackage");
        assert_eq!(payload.error_args, vec!["nope".to_string()]);
    }

    #[test]
    fn empty_queue_is_a_notice_not_a_plan() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(""));
        let payload = queue_payload(&installed, &[], &*invoke);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert!(payload.items.is_empty());
    }

    #[test]
    fn held_note_only_mentions_packages_the_queue_updates() {
        let mut installed = installed_set();
        let held_name = "vim".to_string();
        installed.get_mut(&held_name).unwrap().held = true;

        let queued = queue(&held_name, QueueAction::Update);
        assert_eq!(
            held_in_queue(
                &[update_entry(&held_name, "9.2.2000_1")],
                &installed,
                &queued
            ),
            vec![held_name.clone()]
        );
        assert!(
            held_in_queue(&[update_entry(&held_name, "9.2.2000_1")], &installed, &[]).is_empty(),
            "nothing is queued, so nothing is skipped"
        );
    }

    #[test]
    fn every_plan_task_chooses_its_dry_runs() {
        use crate::xbps_plan::{install_argv, remove_argv, system_argv};
        assert_eq!(
            crate::xbps_plan::queue_requests(&queue("vim", QueueAction::Install))
                .0
                .names,
            ["vim"]
        );
        assert_eq!(install_argv(&["vim".to_string()])[1..3], ["-M", "-n"]);
        assert_eq!(system_argv(), ["xbps-install", "-M", "-n", "-Su"]);
        assert_eq!(
            remove_argv(&["vim".to_string()]),
            ["xbps-remove", "-n", "vim"]
        );
    }

    #[test]
    fn a_plan_error_reaches_the_dialog_as_its_key() {
        let payload = PlanPayload::from_error(&PlanError::UnknownPackage("nope".into()));
        assert_eq!(payload.error_key, "errorUnknownPackage");
        assert!(payload.items.is_empty());
    }

    /// The dialog title is chosen by the caller, not by the dry-run mode: the
    /// cascade and the named install both arrive through `PlanTask::run`.
    #[test]
    fn plan_task_run_keeps_the_cascade_title() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "glibc-2.41_1 remove x86_64 https://r/current 0 848\n",
        ));
        let payload = PlanTask::remove(vec!["glibc".to_string()], true).run(&installed, &*invoke);
        assert_eq!(payload.title_key, "planCascade");
        assert_eq!(payload.title_args, vec!["glibc".to_string()]);
    }

    #[test]
    fn plan_task_run_keeps_the_callers_install_title() {
        let installed = installed_set();
        let (invoke, _) = one_answer(&ok_reply(
            "ark-25.08.0_1 install x86_64 https://r/current 2097152 524288\n",
        ));
        let payload = PlanTask::named(vec!["ark".to_string()], "planInstallQuery".to_string())
            .run(&installed, &*invoke);
        assert_eq!(payload.title_key, "planInstallQuery");
    }

    /// A queue can name the same package twice without the entries being
    /// adjacent; the dry run asks xbps once per name.
    #[test]
    fn plan_task_run_dedups_names_and_keeps_their_order() {
        let installed = installed_set();
        let (invoke, seen) = one_answer(&ok_reply(
            "ark-25.08.0_1 install x86_64 https://r/current 2097152 524288\n\
             vim-9.2.2000_1 install x86_64 https://r/current 4194304 1468006\n",
        ));
        PlanTask::named(
            vec!["ark".to_string(), "vim".to_string(), "ark".to_string()],
            "planInstall".to_string(),
        )
        .run(&installed, &*invoke);
        assert_eq!(
            seen.lock().map(|seen| seen[0].clone()).unwrap(),
            ["xbps-install", "-M", "-n", "ark", "vim"]
        );
    }

    #[test]
    fn the_signature_of_a_store_with_nothing_read_yet_is_empty() {
        // The notifier must not announce before xbps answered: an empty
        // signature is how QML tells that apart from "a set of zero".
        assert_eq!(pending_signature_of(&Store::fresh()), "");
    }

    #[test]
    fn the_signature_moves_with_the_update_set() {
        let (store, _) = read_store();
        let before = pending_signature_of(&store);
        assert!(!before.is_empty(), "one update was reported");

        let mut next = Store::fresh();
        next.apply_snapshot(
            &[kxbps_core::InstalledPackage {
                name: "vim".to_owned(),
                version: "9.2.1031_1".to_owned(),
                pkgver: "vim-9.2.1031_1".to_owned(),
                state: kxbps_core::InstallState::Installed,
                description: String::new(),
            }],
            &["vim".to_owned()],
            &[],
            vec![update_entry("vim", "9.2.3000_1")],
        );
        assert_ne!(
            pending_signature_of(&next),
            before,
            "a new version is a new set"
        );
    }

    #[test]
    fn a_held_update_leaves_the_signature() {
        // The user held the only updatable package; the window offers nothing
        // for update, so nothing may be announced. The held set still has its
        // own fingerprint, which is what `apply_plan` compares the marker to.
        let (mut store, _) = read_store();
        let before = pending_signature_of(&store);
        store.toggle_hold("vim");
        let after = pending_signature_of(&store);
        assert_ne!(after, before, "the effective set changed");
        assert!(
            effective_updates(store.pending_updates(), &store.installed).is_empty(),
            "nothing is offered for update"
        );
        assert_eq!(
            after,
            kxbps_core::update_signature_of(&[]).to_string(),
            "an empty effective set hashes to the marker `apply_plan` writes"
        );
    }

    #[test]
    fn the_signature_matches_the_marker_apply_plan_writes() {
        // The announced set and the dismissed marker must be the same function
        // of the same store, or the popup returns after the user dismissed it.
        let (store, _) = read_store();
        let effective = effective_updates(store.pending_updates(), &store.installed);
        assert_eq!(
            pending_signature_of(&store),
            kxbps_core::update_signature_of(&effective).to_string()
        );
    }

    /// A healthy answer to the five read commands, as the shim would return it.
    fn reading(
        installed: &str,
        updates: &str,
        manual: &str,
        held: &str,
        orphans: &str,
    ) -> Snapshot {
        let outputs: Vec<(String, String)> = [
            ("-l", installed),
            ("-un", updates),
            ("-m", manual),
            ("-H", held),
            ("-O", orphans),
            // The three bulk size takes are decoration: an empty answer is a
            // valid "no sizes", and these tests are about the package lists.
            ("-p", ""),
            ("-R", ""),
        ]
        .into_iter()
        .map(|(mode, stdout)| {
            let reply = format!(
                r#"{{"exitCode":0,"stdout":{},"stderr":"","timedOut":false}}"#,
                serde_json::json!(stdout)
            );
            (mode.to_owned(), reply)
        })
        .collect();
        let invoke = move |request: &str, _: u32| {
            let argv: Vec<String> = serde_json::from_str(request).expect("request decodes");
            let mode = argv.get(1).map(String::as_str).unwrap_or_default();
            outputs
                .iter()
                .find(|(flag, _)| flag == mode)
                .map(|(_, reply)| reply.clone())
                .unwrap_or_else(|| panic!("unexpected command: {argv:?}"))
        };
        read_snapshot(&invoke).expect("a healthy system reads")
    }

    /// A store whose installed set and updates came from a read.
    fn read_store() -> (Store, Option<Snapshot>) {
        let mut store = Store::fresh();
        let mut snapshot = None;
        let read = reading(
            "ii vim-9.2.1031_1 Editor\nii ark-25.08.0_1 Archives\nii zramen-0.2.0_1 Zram\n",
            "vim-9.2.2000_1 update x86_64 https://r/current 10 20\n",
            "vim-9.2.1031_1\n",
            "",
            "zramen-0.2.0_1\n",
        );
        apply_read(&mut store, &mut snapshot, Ok(read));
        (store, snapshot)
    }

    /// The announcement offer for one store at `now`, through the same
    /// function the bridge method calls; the window is hidden, and it is the
    /// offer a start makes.
    fn offer(store: &mut Store, now: i64) -> String {
        take_announcement_with(store, now, false, true)
    }

    /// The same offer with the window on screen.
    fn offer_visible(store: &mut Store, now: i64) -> String {
        take_announcement_with(store, now, true, true)
    }

    /// The offer a check the app runs while it is open makes; the window is
    /// hidden, and the start switch does not gate it.
    fn offer_check(store: &mut Store, now: i64) -> String {
        take_announcement_with(store, now, false, false)
    }

    #[test]
    fn a_set_is_announced_reminded_once_and_then_stays_silent() {
        // The whole cycle the notifier promises: first popup, one reminder
        // three hours later, nothing after that. The bookkeeping is written by
        // the same call that offers the popup.
        const HOUR: i64 = 60 * 60 * 1000;
        let now = 1_700_000_000_000;
        let (mut store, _) = read_store();
        let signature = pending_signature_of(&store);
        assert!(!signature.is_empty(), "the fixture has one update");

        let first = offer(&mut store, now);
        assert!(first.contains("\"kind\":\"first\""), "{first}");
        assert!(first.contains(&format!("\"signature\":\"{signature}\"")));
        assert_eq!(store.prefs.dismissed, signature);
        assert_eq!(store.prefs.notified_at, now);
        assert!(store.prefs.reminded.is_empty(), "nothing reminded yet");

        assert_eq!(offer(&mut store, now + HOUR), "", "too early to remind");

        let reminder = offer(&mut store, now + 3 * HOUR);
        assert!(reminder.contains("\"kind\":\"reminder\""), "{reminder}");
        assert_eq!(store.prefs.reminded, signature);

        assert_eq!(
            offer(&mut store, now + 9 * HOUR),
            "",
            "one reminder per set, then never again"
        );
    }

    #[test]
    fn a_visible_window_defers_the_announcement_without_bookkeeping() {
        let now = 1_700_000_000_000;
        let (mut store, _) = read_store();
        assert_eq!(offer_visible(&mut store, now), "");
        assert!(
            store.prefs.dismissed.is_empty() && store.prefs.notified_at == 0,
            "a deferred announcement writes nothing"
        );
        assert!(offer(&mut store, now).contains("first"));
    }

    #[test]
    fn a_disabled_switch_offers_nothing_and_writes_nothing() {
        let now = 1_700_000_000_000;
        let (mut store, _) = read_store();
        store.set_pref_desktop_notifications(false);
        assert_eq!(offer(&mut store, now), "");
        assert!(store.prefs.dismissed.is_empty());
        assert_eq!(store.prefs.notified_at, 0);
    }

    #[test]
    fn a_start_with_the_start_switch_off_offers_nothing_then_a_check_announces() {
        let now = 1_700_000_000_000;
        let (mut store, _) = read_store();
        store.set_pref_notify_on_start(false);
        // The start offers nothing and writes no bookkeeping, so the set is
        // still offered by a check the app runs while it is open.
        assert_eq!(offer(&mut store, now), "");
        assert!(store.prefs.dismissed.is_empty());
        assert_eq!(store.prefs.notified_at, 0);
        assert!(offer_check(&mut store, now).contains("\"kind\":\"first\""));
    }

    #[test]
    fn a_new_set_reminds_again_after_its_own_three_hours() {
        const HOUR: i64 = 60 * 60 * 1000;
        let now = 1_700_000_000_000;
        let (mut store, _) = read_store();
        let before = pending_signature_of(&store);
        offer(&mut store, now);
        offer(&mut store, now + 3 * HOUR);
        // A newer version of the package: a different set. It is announced
        // once the day is up, and it gets its own reminder afterwards.
        let read = reading(
            "ii vim-9.2.1031_1 Editor\nii ark-25.08.0_1 Archives\nii zramen-0.2.0_1 Zram\n",
            "vim-9.2.3000_1 update x86_64 https://r/current 10 20\n",
            "vim-9.2.1031_1\n",
            "",
            "zramen-0.2.0_1\n",
        );
        let mut snapshot = None;
        apply_read(&mut store, &mut snapshot, Ok(read));
        assert_ne!(pending_signature_of(&store), before, "the set moved");

        let next = now + 30 * HOUR;
        let first = offer(&mut store, next);
        assert!(first.contains("first"), "{first}");
        assert!(store.prefs.reminded.is_empty(), "the reminder is reset");
        assert!(offer(&mut store, next + 3 * HOUR).contains("reminder"));
    }

    #[test]
    fn a_snapshot_replaces_the_store_and_keeps_the_reported_updates() {
        let (store, snapshot) = read_store();

        assert_eq!(
            {
                let mut names: Vec<&String> = store.installed.keys().collect();
                names.sort();
                names
            },
            ["ark", "vim", "zramen"],
            "the previous cache is replaced, not merged"
        );
        assert!(store.installed["vim"].explicit, "`-m` listed vim");
        assert!(!store.installed["zramen"].explicit);
        assert_eq!(store.installed["zramen"].at, 0);
        assert_eq!(
            store.pending_updates().len(),
            1,
            "the updates come from the dry run"
        );
        let snapshot = snapshot.expect("a successful read is remembered");
        assert_eq!(snapshot.orphans, vec!["zramen"]);
    }

    #[test]
    fn a_failed_read_empties_the_list_and_names_the_error() {
        let mut store = Store::fresh();
        store.installed = installed_set();
        let mut snapshot = None;
        assert!(!store.installed.is_empty(), "the cache starts filled");

        let reply = |_: &str, _: u32| {
            String::from(
                r#"{"exitCode":-1,"stdout":"","stderr":"No such file or directory","timedOut":false}"#,
            )
        };
        let error = read_snapshot(&reply).expect_err("xbps is missing");
        let applied = apply_read(&mut store, &mut snapshot, Err(error.clone()));

        assert!(applied, "there was something to empty");
        assert!(store.installed.is_empty(), "no stale row survives");
        assert!(store.pending_updates().is_empty());
        assert!(snapshot.is_none());
        assert_eq!(error.key(), "errorXbpsStart", "QML shows this key");
        assert_eq!(
            error.args(),
            vec!["xbps-query", "No such file or directory"]
        );
    }

    #[test]
    fn a_failed_first_read_does_not_ask_for_a_save() {
        let mut store = Store::fresh();
        store.apply_snapshot(&[], &[], &[], Vec::new());
        let mut snapshot = None;
        let error = read_snapshot(&|_, _| String::from("not json")).expect_err("broken shim");
        assert!(
            !apply_read(&mut store, &mut snapshot, Err(error)),
            "nothing changed, so nothing is written"
        );
    }

    #[test]
    fn a_slow_read_leaves_the_previous_rows_in_place() {
        // The worker answer has not arrived yet: the store still holds what the
        // last read produced, so `refresh()` keeps showing those rows. This is
        // the state a `loading` window renders, with no flash of an empty list.
        let (store, snapshot) = read_store();
        assert_eq!(store.installed.len(), 3);
        assert_eq!(
            snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.installed.len()),
            3,
            "the rows are the previous answer until a new one arrives"
        );
    }

    #[test]
    fn a_new_read_replaces_the_previous_snapshot_whole() {
        let (mut store, mut snapshot) = read_store();
        let next = reading("ii glibc-2.42_1 C library\n", "", "glibc-2.42_1\n", "", "");
        apply_read(&mut store, &mut snapshot, Ok(next));

        assert_eq!(store.installed.len(), 1, "vim and ark are gone");
        assert!(store.installed.contains_key("glibc"));
        assert!(
            store.pending_updates().is_empty(),
            "the system is up to date now"
        );
        let snapshot = snapshot.expect("the new read is remembered");
        assert!(snapshot.updates.is_empty());
        assert!(
            snapshot.orphans.is_empty(),
            "the old orphan list is replaced"
        );
    }

    #[test]
    fn queue_all_updates_marks_every_pending_update() {
        let installed = installed_set();
        let mut snapshot = store_snapshot(&installed);
        snapshot.updates = vec![
            update_entry("mesa", "25.2_1"),
            update_entry("vim", "9.2.3000_1"),
        ];
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(snapshot),
            ..Default::default()
        };

        backend.mark_pending_updates();
        let queued = &backend.store.queue;
        assert_eq!(queued.len(), 2, "both pending updates are queued");
        assert_eq!(queued[0].name, "mesa");
        assert_eq!(queued[0].action, QueueAction::Update);
        assert_eq!(queued[1].name, "vim");
        assert_eq!(queued[1].action, QueueAction::Update);

        backend.mark_pending_updates();
        assert_eq!(
            backend.store.queue.len(),
            2,
            "a second call over the same updates adds nothing"
        );
    }

    #[test]
    fn queue_all_updates_keeps_the_entries_already_queued() {
        let installed = installed_set();
        let snapshot = store_snapshot(&installed);
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, queue("acpid", QueueAction::Remove)),
            snapshot: Some(snapshot),
            ..Default::default()
        };

        backend.mark_pending_updates();
        let queued = &backend.store.queue;
        assert_eq!(queued.len(), 2, "the kept removal and the new update");
        assert_eq!(queued[0].name, "acpid");
        assert_eq!(queued[0].action, QueueAction::Remove);
        assert_eq!(queued[1].name, "mesa");
        assert_eq!(queued[1].action, QueueAction::Update);
    }

    // ---- the fresh check (task 27) ----

    /// A transport that answers the fresh check and refuses every other read.
    ///
    /// The check is the only process these tests are allowed to ask for; a
    /// `reload()` running beside it would make the count meaningless, so any
    /// other argv panics.
    fn check_transport(remote: String) -> (SharedInvoke, Arc<Mutex<usize>>, Arc<Mutex<String>>) {
        let calls = Arc::new(Mutex::new(0usize));
        let last = Arc::new(Mutex::new(String::new()));
        let seen_calls = calls.clone();
        let seen_last = last.clone();
        let invoke: SharedInvoke = Arc::new(move |request: &str, _timeout: u32| {
            if request.contains("\"-L\"") {
                // The repository count the failure sentence names.
                return ok_reply(
                    "  10 https://a/current (RSA signed)\n   5 https://b/current (RSA signed)\n",
                );
            }
            assert!(
                request.contains("\"-M\""),
                "the check is the only other process this test allows, got {request}"
            );
            *seen_calls.lock().expect("lock") += 1;
            *seen_last.lock().expect("lock") = request.to_owned();
            remote.clone()
        });
        (invoke, calls, last)
    }

    #[test]
    fn a_failed_check_keeps_the_list_and_reports_the_error() {
        // The reply is the measured shape: one repository failed, exit 0.
        let remote = Box::leak(
            concat!(
                r#"{"exitCode":0,"stdout":"vim-9.2.2000_1 update x86_64 https://r/current 10 20\n","stderr":"#,
                r#""ERROR: failed to open repository: https://a/current: Input/output error\n","timedOut":false}"#
            )
            .to_owned()
            .into_boxed_str(),
        );
        let (invoke, _, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let before = update_entry("vim", "9.2.1500_1");
        let mut store = store_with(&installed, Vec::new());
        store.replace_updates(vec![before.clone()]);
        let mut backend = InstalledBackendRust {
            store,
            snapshot: Some(Snapshot {
                updates: vec![before.clone()],
                ..store_snapshot(&installed)
            }),
            invoke,
            ..Default::default()
        };

        let status = backend.run_check(true, 1_700_000_000_000);

        assert_eq!(
            backend.store.pending_updates(),
            [before],
            "a failed check must not touch the list"
        );
        assert_eq!(status.key, "checkRepositoriesFailed");
        assert_ne!(status.key, "checkUpToDate");
        assert_eq!(
            backend.store.prefs.last_check_at, 0,
            "a failed check is not a successful one"
        );
        assert_eq!(
            backend.check_error_key.to_string(),
            "checkRepositoriesFailed",
            "the failure sentence is kept for the strip"
        );
        assert!(!backend.checking);
    }

    #[test]
    fn a_successful_check_replaces_the_list_and_stamps_the_time() {
        let remote = r#"{"exitCode":0,"stdout":"vim-9.2.2000_1 update x86_64 https://r/current 10 20\n","stderr":"","timedOut":false}"#;
        let (invoke, calls, seen) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            ..Default::default()
        };

        let status = backend.run_check(true, 1_700_000_000_000);

        assert_eq!(*calls.lock().expect("lock"), 1);
        let request = seen.lock().expect("lock").clone();
        assert_eq!(request, r#"["xbps-install","-M","-un"]"#);
        assert_eq!(backend.store.pending_updates().len(), 1);
        assert_eq!(backend.store.pending_updates()[0].version, "9.2.2000_1");
        assert_eq!(backend.store.prefs.last_check_at, 1_700_000_000_000);
        assert_eq!(status.key, "checkUpdatesFound");
        assert_eq!(
            backend.check_error_key.to_string(),
            "",
            "a good check clears the error"
        );
    }

    #[test]
    fn a_check_that_found_nothing_is_the_only_way_to_say_up_to_date() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, _, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            ..Default::default()
        };

        let status = backend.run_check(true, 1_700_000_000_000);

        assert_eq!(status.key, "checkUpToDate");
        assert!(backend.store.pending_updates().is_empty());
        assert_eq!(backend.check_error_key.to_string(), "");
    }

    #[test]
    fn a_manual_check_ignores_the_clock() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, calls, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut store = store_with(&installed, Vec::new());
        store.set_pref_last_check_at(1_699_999_999_000);
        let mut backend = InstalledBackendRust {
            store,
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            ..Default::default()
        };

        backend.run_check(true, 1_699_999_999_500);

        assert_eq!(
            *calls.lock().expect("lock"),
            1,
            "a manual check runs though the last one is a moment old"
        );
    }

    #[test]
    fn an_automatic_check_inside_the_gap_does_nothing() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, calls, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut store = store_with(&installed, Vec::new());
        store.set_pref_last_check_at(1_699_999_999_000);
        let mut backend = InstalledBackendRust {
            store,
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            ..Default::default()
        };

        let status = backend.run_check(false, 1_699_999_999_500);

        assert_eq!(*calls.lock().expect("lock"), 0, "not due, so no process");
        assert_eq!(status.key, "", "a skipped check says nothing");
    }

    #[test]
    fn an_automatic_check_offline_does_not_start_a_process() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, calls, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            reachability: Arc::new(|| 1),
            ..Default::default()
        };

        let status = backend.run_check(false, 1_700_000_000_000);

        assert_eq!(*calls.lock().expect("lock"), 0, "offline: no xbps run");
        assert_eq!(status.key, "checkOffline");
        assert_eq!(backend.check_error_key.to_string(), "checkOffline");
    }

    #[test]
    fn a_manual_check_offline_still_tries_the_network() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, calls, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            reachability: Arc::new(|| 1),
            ..Default::default()
        };

        let status = backend.run_check(true, 1_700_000_000_000);

        assert_eq!(
            *calls.lock().expect("lock"),
            1,
            "the user asked, so xbps runs and its own answer decides"
        );
        assert_eq!(status.key, "checkUpToDate");
    }

    #[test]
    fn an_unknown_reachability_lets_the_check_run() {
        let remote = r#"{"exitCode":0,"stdout":"","stderr":"","timedOut":false}"#;
        let (invoke, calls, _) = check_transport(remote.to_owned());
        let installed = installed_set();
        let mut backend = InstalledBackendRust {
            store: store_with(&installed, Vec::new()),
            snapshot: Some(store_snapshot(&installed)),
            invoke,
            reachability: Arc::new(|| -1),
            ..Default::default()
        };

        backend.run_check(false, 1_700_000_000_000);

        assert_eq!(
            *calls.lock().expect("lock"),
            1,
            "no backend is not the same as offline"
        );
    }

    /// The merge `finish_read` performs, without a QObject: a local list
    /// replaces the current one only when no fresh remote list outranks it.
    fn apply_local_read(
        backend: &mut InstalledBackendRust,
        updates: Vec<UpdateEntry>,
        now: i64,
    ) -> bool {
        if !local_replaces(backend.updates_origin, now, AUTO_GAP_MS) {
            return false;
        }
        backend.store.replace_updates(updates.clone());
        if let Some(snapshot) = backend.snapshot.as_mut() {
            snapshot.updates = updates;
        }
        backend.updates_origin = UpdatesOrigin::Local;
        true
    }

    #[test]
    fn a_local_read_does_not_replace_a_young_remote_list() {
        let installed = installed_set();
        let remote = update_entry("vim", "9.2.2000_1");
        let local = update_entry("vim", "9.2.1500_1");
        let mut store = store_with(&installed, Vec::new());
        store.replace_updates(vec![remote.clone()]);
        let mut backend = InstalledBackendRust {
            store,
            snapshot: Some(Snapshot {
                updates: vec![remote.clone()],
                ..store_snapshot(&installed)
            }),
            updates_origin: UpdatesOrigin::Remote {
                at: 1_700_000_000_000,
            },
            ..Default::default()
        };

        assert!(!apply_local_read(
            &mut backend,
            vec![local],
            1_700_000_060_000
        ));

        assert_eq!(
            backend.store.pending_updates(),
            [remote],
            "the fresh remote list survives a local read"
        );
    }

    #[test]
    fn a_local_read_replaces_a_stale_remote_list() {
        let installed = installed_set();
        let remote = update_entry("vim", "9.2.2000_1");
        let local = update_entry("vim", "9.2.1500_1");
        let mut store = store_with(&installed, Vec::new());
        store.replace_updates(vec![remote.clone()]);
        let mut backend = InstalledBackendRust {
            store,
            snapshot: Some(Snapshot {
                updates: vec![remote],
                ..store_snapshot(&installed)
            }),
            updates_origin: UpdatesOrigin::Remote {
                at: 1_700_000_000_000,
            },
            ..Default::default()
        };

        assert!(apply_local_read(
            &mut backend,
            vec![local.clone()],
            1_700_000_900_000
        ));

        assert_eq!(
            backend.store.pending_updates(),
            std::slice::from_ref(&local)
        );
        assert_eq!(
            backend.updates_origin,
            UpdatesOrigin::Local,
            "the list is local now, so the next local read may replace it"
        );
    }
}
