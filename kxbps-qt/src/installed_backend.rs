//! The installed database, the transaction queue and transaction planning.
//!
//! One object owns the [`Store`] (installed map, queue, preferences, mirror) and
//! is therefore the single source of truth for everything that survives a
//! restart. It exposes the package model and the queue model, mutates the store
//! on behalf of QML, and builds the plan/job objects the dialogs show.
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
    Catalog, InstalledMap, Plan, PlanItem, QueueAction, QueueItem, Repos, Store, effective_updates,
    held_updates_in, plan_install, plan_queue, plan_remove, plan_system, repos_from_installed,
    update_signature_of,
};

use crate::job_backend::JobPayload;
use crate::plan_backend::{PlanPayload, keys as plan_keys};
use crate::xbps_snapshot::{SharedInvoke, Snapshot, SnapshotError, read_snapshot};
use kxbps_core::UpdateEntry;

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

        #[cxx_name = "kxbps_create_package_model"]
        fn create_package_model(parent: QObjectMutPtr, catalog_json: &QString) -> QObjectMutPtr;

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

        #[cxx_name = "kxbps_create_plan_backend"]
        fn create_plan_backend(parent: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "kxbps_start_job"]
        fn start_job(owner: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "kxbps_object_string_property"]
        fn object_string_property(object: QObjectMutPtr, name: &QString) -> QString;

        #[cxx_name = "kxbps_catalog_backend_set_state"]
        fn catalog_backend_set_state(
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
        #[qproperty(QObjectMutPtr, query_backend, READ, WRITE = set_query_backend, NOTIFY)]
        #[qproperty(i32, installed_count, READ, NOTIFY)]
        #[qproperty(i32, queue_count, READ, NOTIFY)]
        #[qproperty(i32, updates_count, READ, NOTIFY)]
        // Bumped on every `refresh()`, including changes the counters cannot
        // see (a hold toggle). QML uses it to re-read snapshots that have no
        // notify signal of their own (`catalogBackend.get`).
        #[qproperty(i32, state_revision, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
        // A failed read of the system (xbps missing, timed out, unparsable
        // output) as the i18n key QML wraps in a sentence, plus its `%1…%3`
        // arguments. Empty key means the last read succeeded.
        #[qproperty(QString, read_error, READ, NOTIFY)]
        #[qproperty(QStringList, read_error_args, READ, NOTIFY)]
        // Whether a snapshot is being read. The list keeps its previous rows
        // while one is in flight; QML only needs this to explain an empty list.
        #[qproperty(bool, loading, READ, NOTIFY)]
        type InstalledBackend = super::InstalledBackendRust;

        #[qinvokable]
        fn toggle_queue(self: Pin<&mut InstalledBackend>, name: &QString, action: &QString);

        #[qinvokable]
        fn remove_queued(self: Pin<&mut InstalledBackend>, name: &QString);

        #[qinvokable]
        fn clear_queue(self: Pin<&mut InstalledBackend>);

        #[qinvokable]
        fn toggle_hold(self: Pin<&mut InstalledBackend>, name: &QString);

        #[qinvokable]
        fn open_queue_plan(self: &InstalledBackend) -> QObjectMutPtr;

        #[qinvokable]
        fn open_system_plan(self: &InstalledBackend) -> QObjectMutPtr;

        #[qinvokable]
        fn open_named_plan(
            self: &InstalledBackend,
            names: &QStringList,
            title_key: &QString,
        ) -> QObjectMutPtr;

        #[qinvokable]
        fn open_remove(self: &InstalledBackend, name: &QString, cascade: bool) -> QObjectMutPtr;

        #[qinvokable]
        fn open_remove_names(
            self: &InstalledBackend,
            names: &QStringList,
            cascade: bool,
        ) -> QObjectMutPtr;

        #[qinvokable]
        fn apply_plan(self: &InstalledBackend, plan: QObjectMutPtr) -> QObjectMutPtr;

        // Re-reads the installed set, the updates and the orphan list from
        // xbps on a worker thread. Called once when the window opens and by the
        // "reload" button; the previous rows stay visible until it answers.
        #[qinvokable]
        fn reload(self: Pin<&mut InstalledBackend>);

        #[qinvokable]
        fn reset(self: Pin<&mut InstalledBackend>);

        /// Whether the installed set holds `name`. A view that needs one
        /// package's state reads the list it already shows, rather than asking
        /// xbps for the package.
        #[qinvokable]
        fn is_installed(self: &InstalledBackend, name: &QString) -> bool;

        #[qinvokable]
        fn release(self: &InstalledBackend, object: QObjectMutPtr);

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

        // Called by `kxbps_installed_apply_plan` on the GUI thread when a job
        // finishes; not invokable so QML cannot bypass the job.
        fn apply_plan_items(self: Pin<&mut InstalledBackend>, items_json: &QString);
    }

    impl cxx_qt::Constructor<()> for InstalledBackend {}
    // A read runs on a worker thread and reports back through
    // `qt_thread().queue(...)`; that needs the thread handle CXX-Qt generates.
    impl cxx_qt::Threading for InstalledBackend {}
}

/// Rust side of [`qobject::InstalledBackend`].
pub struct InstalledBackendRust {
    catalog: Arc<Catalog>,
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
    read_error: QString,
    read_error_args: QStringList,
    loading: bool,
    /// The last snapshot xbps answered; its updates and orphans are what the
    /// model and the query façade show. `None` until the first read.
    snapshot: Option<Snapshot>,
    /// How many reads were started; only the newest may write its result, so
    /// a slow first read cannot overwrite a newer one.
    generation: Arc<AtomicU64>,
    /// The process transport. Tests install a fake here, so the suite never
    /// starts xbps; production installs [`crate::xbps_bridge::production_invoke`].
    invoke: SharedInvoke,
}

impl Default for InstalledBackendRust {
    /// Only used before `initialize` runs; the models stay null until then and
    /// the store is the empty seed image.
    fn default() -> Self {
        Self {
            catalog: Arc::default(),
            store: Store::fresh(&Catalog::default()),
            packages: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            queue_model: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            query_backend: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            installed_count: 0,
            queue_count: 0,
            updates_count: 0,
            state_revision: 0,
            error: QString::from(""),
            read_error: QString::from(""),
            read_error_args: QStringList::default(),
            loading: false,
            snapshot: None,
            generation: Arc::new(AtomicU64::new(0)),
            invoke: crate::xbps_bridge::production_invoke(),
        }
    }
}

impl cxx_qt::Initialize for qobject::InstalledBackend {
    /// Loads the catalog, then the store, then creates both child models.
    ///
    /// A store that cannot be loaded is not fatal: the seed image stays in
    /// memory and `error` carries a technical reason for QML to wrap in a
    /// translated sentence. The installed set itself is not the store's: the
    /// window opens with whatever the store last saved, and the first read of
    /// xbps replaces it a moment later (see [`qobject::InstalledBackend::reload`]).
    fn initialize(mut self: Pin<&mut Self>) {
        let json = crate::qobject::read_resource(&QString::from(crate::CATALOG_RESOURCE_PATH));
        let (catalog, catalog_error, model_json) = crate::catalog_backend::load_catalog(&json);
        let (store, store_error) = match Store::load(&catalog) {
            Ok(store) => (store, String::new()),
            Err(err) => (Store::fresh(&catalog), format!("{err}")),
        };

        let parent = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::InstalledBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self)
                    .cast_mut(),
            )
        };
        let queue_json = json_of(&store.queue);
        let packages = qobject::create_package_model(parent, &model_json);
        let queue_model = qobject::create_queue_model(parent, &queue_json);

        let queue_count = store.queue.len();
        let error = if catalog_error.is_empty() {
            store_error
        } else {
            catalog_error
        };

        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.catalog = Arc::new(catalog);
        data.store = store;
        data.packages = packages;
        data.queue_model = queue_model;
        data.installed_count = 0;
        data.queue_count = i32::try_from(queue_count).unwrap_or(i32::MAX);
        data.updates_count = 0;
        data.error = QString::from(error.as_str());
    }
}

impl qobject::InstalledBackend {
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
        refresh(self.as_mut());
    }

    fn remove_queued(mut self: Pin<&mut Self>, name: &cxx_qt_lib::QString) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.remove_queued(&name.to_string());
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
    }

    fn clear_queue(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.clear_queue();
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
    }

    fn toggle_hold(mut self: Pin<&mut Self>, name: &cxx_qt_lib::QString) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.toggle_hold(&name.to_string());
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
    }

    fn open_queue_plan(&self) -> QObjectMutPtr {
        let payload = queue_payload(
            &self.catalog,
            &self.store.installed,
            &repos_from_installed(&self.store.installed),
            self.store.pending_updates(),
            &self.store.queue,
        );
        self.plan_object(&payload)
    }

    fn open_system_plan(&self) -> QObjectMutPtr {
        let payload = system_payload(
            &self.catalog,
            &self.store.installed,
            &repos_from_installed(&self.store.installed),
            self.store.pending_updates(),
        );
        self.plan_object(&payload)
    }

    fn open_named_plan(
        &self,
        names: &cxx_qt_lib::QStringList,
        title_key: &cxx_qt_lib::QString,
    ) -> QObjectMutPtr {
        let names = strings_of(names);
        let title_key = title_key.to_string();
        let title_key = if title_key.is_empty() {
            plan_keys::PLAN_INSTALL.to_string()
        } else {
            title_key
        };
        let payload = named_payload(
            &self.catalog,
            &self.store.installed,
            &repos_from_installed(&self.store.installed),
            &names,
            &title_key,
        );
        self.plan_object(&payload)
    }

    fn open_remove(&self, name: &cxx_qt_lib::QString, cascade: bool) -> QObjectMutPtr {
        self.open_remove_names(&cxx_qt_lib::QStringList::from(name), cascade)
    }

    fn open_remove_names(&self, names: &cxx_qt_lib::QStringList, cascade: bool) -> QObjectMutPtr {
        let payload = remove_payload(
            &self.catalog,
            &self.store.installed,
            &strings_of(names),
            cascade,
        );
        self.plan_object(&payload)
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
            mirror: self.store.mirror.clone(),
            repos: repos_from_installed(&self.store.installed),
        };
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| String::from("{}"));
        qobject::start_job(self.parent_ptr(), &QString::from(json.as_str()))
    }

    fn is_installed(&self, name: &cxx_qt_lib::QString) -> bool {
        self.store.installed.contains_key(&name.to_string())
    }

    fn reset(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.reset();
        }
        save_store(self.as_mut());
        refresh(self.as_mut());
        // The store is back to the seed image, which is not the system: read
        // it again so the window shows what is installed rather than a demo.
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
            let data = Pin::get_mut(self.as_mut().rust_mut());
            apply_read(&mut data.store, &mut data.snapshot, result)
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
                    items,
                    blocked: Vec::new(),
                },
                now,
            );
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
    }

    /// The backend as a `QObject*`, for the C++ child factories.
    fn parent_ptr(&self) -> QObjectMutPtr {
        unsafe {
            QObjectMutPtr::from_raw(
                <qobject::InstalledBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(self).cast_mut(),
            )
        }
    }

    /// Wraps a payload into a child `PlanBackend` and hands it to QML.
    fn plan_object(&self, payload: &PlanPayload) -> QObjectMutPtr {
        let json = serde_json::to_string(payload).unwrap_or_else(|_| String::from("{}"));
        qobject::create_plan_backend(self.parent_ptr(), &QString::from(json.as_str()))
    }
}

/// Plan payload for the transaction queue.
///
/// `updates` is the list the last read reported; it is what the held note
/// needs, because the catalog no longer decides what an update is.
pub fn queue_payload(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    updates: &[UpdateEntry],
    queue: &[QueueItem],
) -> PlanPayload {
    if queue.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE);
    }
    match plan_queue(catalog, installed, repos, queue) {
        Ok(plan) if plan.blocked.is_empty() && plan.items.is_empty() => {
            PlanPayload::notice(plan_keys::NOTICE_EMPTY_QUEUE)
        }
        Ok(plan) => {
            let names = queue
                .iter()
                .filter(|item| item.action == QueueAction::Remove)
                .map(|item| item.name.clone())
                .collect();
            let mut payload =
                PlanPayload::from_plan(plan, plan_keys::PLAN_QUEUE, Vec::new(), names);
            let held = held_in_queue(updates, installed, queue);
            if !held.is_empty() {
                payload.note_key = plan_keys::NOTE_HELD.to_string();
                payload.note_args = vec![held.join(", ")];
            }
            payload
        }
        Err(err) => PlanPayload::error(&err),
    }
}

/// Plan payload for the system upgrade.
pub fn system_payload(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    updates: &[UpdateEntry],
) -> PlanPayload {
    match plan_system(catalog, installed, repos) {
        Ok(items) if items.is_empty() => PlanPayload::notice(plan_keys::NOTICE_EMPTY_SYSTEM),
        Ok(items) => {
            let mut payload = PlanPayload::from_plan(
                Plan {
                    items,
                    blocked: Vec::new(),
                },
                plan_keys::PLAN_SYSTEM,
                Vec::new(),
                Vec::new(),
            );
            let held = held_updates_in(updates, installed);
            if !held.is_empty() {
                payload.note_key = plan_keys::NOTE_HELD.to_string();
                payload.note_args = vec![held.join(", ")];
            }
            payload
        }
        Err(err) => PlanPayload::error(&err),
    }
}

/// Plan payload for "install exactly these packages".
pub fn named_payload(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    names: &[String],
    title_key: &str,
) -> PlanPayload {
    let queue: Vec<QueueItem> = names
        .iter()
        .map(|name| QueueItem {
            name: name.clone(),
            action: QueueAction::Install,
        })
        .collect();
    match plan_install(catalog, installed, repos, &queue) {
        Ok(items) if items.is_empty() => PlanPayload::notice(plan_keys::NOTICE_EMPTY_INSTALL),
        Ok(items) => PlanPayload::from_plan(
            Plan {
                items,
                blocked: Vec::new(),
            },
            title_key,
            Vec::new(),
            names.to_vec(),
        ),
        Err(err) => PlanPayload::error(&err),
    }
}

/// Plan payload for a removal, blocked list included.
pub fn remove_payload(
    catalog: &Catalog,
    installed: &InstalledMap,
    names: &[String],
    cascade: bool,
) -> PlanPayload {
    let plan = plan_remove(catalog, installed, names, cascade);
    let title_key = if cascade {
        plan_keys::PLAN_CASCADE
    } else {
        plan_keys::PLAN_REMOVE
    };
    let title_args = vec![names.join(" ")];
    if plan.is_blocked() {
        return PlanPayload::from_plan(plan, title_key, title_args, names.to_vec());
    }
    if plan.items.is_empty() {
        return PlanPayload::notice(plan_keys::NOTICE_EMPTY_REMOVE);
    }
    PlanPayload::from_plan(plan, title_key, title_args, names.to_vec())
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
        qobject::catalog_backend_set_state(
            query,
            installed_json,
            queue_json,
            updates_json,
            orphans_json,
        );
    }
}

/// The installed set, queue, updates and orphans as the shims read them.
fn state_json(store: &Store, snapshot: Option<&Snapshot>) -> (QString, QString, QString, QString) {
    let updates = snapshot.map_or_else(Vec::new, |snapshot| snapshot.updates.clone());
    let orphans = snapshot.map_or_else(Vec::new, |snapshot| snapshot.orphans.clone());
    (
        json_of(&store.installed),
        json_of(&store.queue),
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

/// Applies a finished plan and refreshes the model and the counters.
fn refresh(mut self_: Pin<&mut qobject::InstalledBackend>) {
    let (
        installed_json,
        queue_json,
        updates_json,
        orphans_json,
        installed_count,
        queue_count,
        updates_count,
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
            data.packages,
            data.queue_model,
            data.query_backend,
            data.snapshot.as_ref().map(json_of),
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

    let (installed_changed, queue_changed, updates_changed) = {
        let data = Pin::get_mut(self_.as_mut().rust_mut());
        let changed = (
            data.installed_count != installed_count,
            data.queue_count != queue_count,
            data.updates_count != updates_count,
        );
        data.installed_count = installed_count;
        data.queue_count = queue_count;
        data.updates_count = updates_count;
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
        let message = format!("{err}");
        {
            let data = Pin::get_mut(self_.as_mut().rust_mut());
            data.error = QString::from(message.as_str());
        }
        self_.as_mut().error_changed();
    }
}

fn strings_of(list: &cxx_qt_lib::QStringList) -> Vec<String> {
    list.iter().map(QString::to_string).collect()
}

fn json_of<T: serde::Serialize>(value: &T) -> QString {
    let json = serde_json::to_string(value).unwrap_or_else(|_| String::from("null"));
    QString::from(json.as_str())
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
    use kxbps_core::{PlanAction, PlanReason, SEED_AT};

    fn catalog() -> Catalog {
        Catalog::load_from_str(include_str!("../resources/catalog.json")).unwrap()
    }

    fn seed() -> (Catalog, InstalledMap) {
        let catalog = catalog();
        let store = Store::fresh(&catalog);
        (catalog, store.installed)
    }

    /// The updates the read layer would report for the seed image: every
    /// installed package whose seed version is below the catalog's, plus the
    /// dependency pins the plan tests need. The read layer gets this list from
    /// `xbps-install -un`; the seed only knows the versions.
    fn seed_updates(catalog: &Catalog, installed: &InstalledMap) -> Vec<UpdateEntry> {
        let mut updates: Vec<UpdateEntry> = installed
            .iter()
            .filter_map(|(name, inst)| {
                let pkg = catalog.by_name(name)?;
                (pkg.version != inst.version).then(|| update_entry(name, &pkg.version))
            })
            .collect();
        updates.sort_by(|left, right| left.name.cmp(&right.name));
        updates
    }

    fn update_entry(name: &str, version: &str) -> UpdateEntry {
        UpdateEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: format!("{name}-{version}"),
            action: kxbps_core::XbpsAction::Update,
            arch: "x86_64".to_owned(),
            repository: "https://repo.example/current".to_owned(),
            installed_size: 1,
            download_size: 1,
        }
    }

    fn queue(name: &str, action: QueueAction) -> Vec<QueueItem> {
        vec![QueueItem {
            name: name.into(),
            action,
        }]
    }

    #[test]
    fn queue_plan_installs_a_package_missing_from_the_seed_image() {
        // `vim` is part of the seed image, so the interesting install is a
        // package the image does not have. `ark` needs `qt6-base`, which needs
        // `mesa` — pinned below the catalog by the seed, so the plan is the
        // dependency update followed by the install.
        let (catalog, installed) = seed();
        let updates = seed_updates(&catalog, &installed);
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &updates,
            &queue("ark", QueueAction::Install),
        );
        assert_eq!(payload.title_key, "planQueue");
        assert!(payload.error_key.is_empty());
        assert!(payload.notice_key.is_empty());
        assert_eq!(payload.items.len(), 2, "mesa is an outdated dependency");
        assert_eq!(payload.items[0].name, "mesa");
        assert_eq!(payload.items[0].action, PlanAction::Update);
        assert_eq!(payload.items[0].reason, PlanReason::Dependency);
        assert_eq!(payload.items[1].name, "ark");
        assert_eq!(payload.items[1].action, PlanAction::Install);
        assert_eq!(payload.items[1].reason, PlanReason::Requested);
    }

    #[test]
    fn queue_plan_pulls_in_an_outdated_dependency() {
        // The seed pins `git` and `curl` below the catalog, so updating `git`
        // plans the dependency update first.
        let (catalog, installed) = seed();
        let updates = seed_updates(&catalog, &installed);
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &updates,
            &queue("git", QueueAction::Update),
        );
        assert!(payload.error_key.is_empty());
        assert!(
            payload
                .items
                .iter()
                .any(|item| item.name == "curl" && item.reason == PlanReason::Dependency),
            "curl is planned as a dependency"
        );
        assert!(
            payload.items.iter().any(|item| item.name == "git"),
            "git itself is planned"
        );
        let curl = payload.items.iter().position(|item| item.name == "curl");
        let git = payload.items.iter().position(|item| item.name == "git");
        assert!(curl < git, "dependencies come before dependents");
    }

    #[test]
    fn applying_the_plan_adds_exactly_one_package() {
        let (catalog, installed) = seed();
        assert_eq!(installed.len(), 56, "seed image size");
        let updates = seed_updates(&catalog, &installed);
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &updates,
            &queue("ark", QueueAction::Install),
        );
        let mut store = Store::fresh(&catalog);
        store.queue = queue("ark", QueueAction::Install);
        store.apply_plan(
            &Plan {
                items: payload.items.clone(),
                blocked: Vec::new(),
            },
            SEED_AT,
        );
        assert_eq!(store.installed.len(), 57, "ark is the only new package");
        assert!(store.installed.contains_key("ark"));
        assert!(
            store.queue.is_empty(),
            "the applied package leaves the queue"
        );
    }

    #[test]
    fn reinstalling_a_seed_package_keeps_the_installed_count() {
        // The brief expected `vim` to be a fresh install; it is part of the seed
        // image, so the queue produces a reinstall and the count stays at 56.
        let (catalog, installed) = seed();
        let updates = seed_updates(&catalog, &installed);
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &updates,
            &queue("vim", QueueAction::Install),
        );
        assert_eq!(payload.items.len(), 1);
        assert_eq!(payload.items[0].name, "vim");
        assert_eq!(payload.items[0].action, PlanAction::Reinstall);

        let mut store = Store::fresh(&catalog);
        store.apply_plan(
            &Plan {
                items: payload.items,
                blocked: Vec::new(),
            },
            SEED_AT,
        );
        assert_eq!(store.installed.len(), 56);
    }

    #[test]
    fn system_plan_matches_the_core_update_list() {
        let (catalog, installed) = seed();
        let repos = repos_from_installed(&installed);
        let updates = seed_updates(&catalog, &installed);
        assert!(!updates.is_empty(), "the seed pins outdated versions");
        let payload = system_payload(&catalog, &installed, &repos, &updates);
        assert_eq!(payload.title_key, "planSystem");
        assert!(
            payload.items.len() >= updates.len(),
            "every reported update is planned"
        );
        for update in &updates {
            assert!(
                payload.items.iter().any(|item| item.name == update.name),
                "{} is planned",
                update.name
            );
        }
    }

    #[test]
    fn removing_glibc_reports_its_dependents() {
        let (catalog, installed) = seed();
        let payload = remove_payload(&catalog, &installed, &["glibc".to_string()], false);
        assert!(payload.items.is_empty(), "a blocked plan has no steps");
        assert!(
            payload
                .blocked
                .iter()
                .any(|blocked| blocked.name == "glibc"),
            "glibc is blocked"
        );
        let blocked = payload
            .blocked
            .iter()
            .find(|blocked| blocked.name == "glibc")
            .unwrap();
        assert!(!blocked.needed_by.is_empty(), "dependents are listed");
        assert!(blocked.needed_by.iter().any(|name| name == "bash"));
        assert_eq!(payload.names, vec!["glibc".to_string()]);
        assert_eq!(payload.title_key, "planRemove");
    }

    #[test]
    fn cascade_removes_the_dependents_instead_of_blocking() {
        let (catalog, installed) = seed();
        let payload = remove_payload(&catalog, &installed, &["glibc".to_string()], true);
        assert!(payload.blocked.is_empty());
        assert!(payload.items.iter().any(|item| item.name == "glibc"));
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
        let (catalog, installed) = seed();
        let payload = remove_payload(&catalog, &installed, &["nonexistent".to_string()], false);
        assert_eq!(payload.notice_key, "emptyRemove");
        assert!(payload.items.is_empty());
    }

    #[test]
    fn named_plan_keeps_the_callers_title_key() {
        let (catalog, installed) = seed();
        let payload = named_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &["ark".to_string()],
            "planInstall",
        );
        assert_eq!(payload.title_key, "planInstall");
        assert!(payload.items.iter().any(|item| item.name == "ark"));
        assert_eq!(payload.names, vec!["ark".to_string()]);
    }

    #[test]
    fn unknown_packages_become_an_error_key() {
        let (catalog, installed) = seed();
        let payload = named_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &["nope".to_string()],
            "planInstall",
        );
        assert_eq!(payload.error_key, "errorUnknownPackage");
        assert_eq!(payload.error_args, vec!["nope".to_string()]);
    }

    #[test]
    fn disabled_repository_becomes_a_key_with_the_repo() {
        let (catalog, installed) = seed();
        // `steam` lives in nonfree, which the seed image does not enable.
        let payload = named_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
            &["steam".to_string()],
            "planInstall",
        );
        assert_eq!(payload.error_key, "errorRepoDisabled");
        assert_eq!(
            payload.error_args,
            vec!["steam".to_string(), "nonfree".to_string()]
        );
    }

    #[test]
    fn empty_queue_is_a_notice_not_a_plan() {
        let (catalog, installed) = seed();
        let payload = queue_payload(&catalog, &installed, &Repos::default(), &[], &[]);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert!(payload.items.is_empty());
    }

    #[test]
    fn held_note_only_mentions_packages_the_queue_updates() {
        let (catalog, mut installed) = seed();
        let updates = seed_updates(&catalog, &installed);
        assert!(updates.len() >= 2, "the seed has several outdated packages");
        let held_name = updates[0].name.clone();
        installed.get_mut(&held_name).unwrap().held = true;

        let queued = queue(&held_name, QueueAction::Update);
        assert_eq!(
            held_in_queue(&updates, &installed, &queued),
            vec![held_name.clone()]
        );
        assert!(
            held_in_queue(&updates, &installed, &[]).is_empty(),
            "nothing is queued, so nothing is skipped"
        );
    }

    #[test]
    fn state_revision_advances_and_wraps() {
        assert_eq!(next_revision(0), 1);
        assert_eq!(next_revision(41), 42);
        assert_eq!(next_revision(i32::MAX), i32::MIN);
    }

    #[test]
    fn queue_actions_parse_from_the_qml_strings() {
        assert_eq!(parse_action("install"), QueueAction::Install);
        assert_eq!(parse_action("remove"), QueueAction::Remove);
        assert_eq!(parse_action("update"), QueueAction::Update);
        assert_eq!(parse_action("nonsense"), QueueAction::Install);
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
        let mut store = Store::fresh(&catalog());
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
            "the seed image is replaced, not merged"
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
        let mut store = Store::fresh(&catalog());
        let mut snapshot = None;
        assert!(!store.installed.is_empty(), "the seed starts the store");

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
        let mut store = Store::fresh(&catalog());
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
}
