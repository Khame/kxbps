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
use std::time::{SystemTime, UNIX_EPOCH};

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QObjectMutPtr, QString};
use khaxbps_core::{
    Catalog, InstalledMap, Plan, PlanItem, QueueAction, QueueItem, Repos, Store, held_updates,
    plan_install, plan_queue, plan_remove, plan_system, repos_from_installed, update_signature,
};

use crate::job_backend::JobPayload;
use crate::plan_backend::{PlanPayload, keys as plan_keys};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

        // Declares `khaxbps_create_package_model`, which lives in the model
        // shim rather than the backend shim.
        include!("model_shim.h");

        include!("backend_shim.h");

        #[cxx_name = "khaxbps_create_package_model"]
        fn create_package_model(
            parent: QObjectMutPtr,
            catalog_json: &QString,
            installed_json: &QString,
        ) -> QObjectMutPtr;

        #[cxx_name = "khaxbps_create_queue_model"]
        fn create_queue_model(parent: QObjectMutPtr, queue_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "khaxbps_package_model_set_state"]
        fn package_model_set_state(
            model: QObjectMutPtr,
            installed_json: &QString,
            queue_json: &QString,
        );

        #[cxx_name = "khaxbps_queue_model_set_state"]
        fn queue_model_set_state(model: QObjectMutPtr, queue_json: &QString);

        #[cxx_name = "khaxbps_create_plan_backend"]
        fn create_plan_backend(parent: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "khaxbps_start_job"]
        fn start_job(owner: QObjectMutPtr, payload_json: &QString) -> QObjectMutPtr;

        #[cxx_name = "khaxbps_object_string_property"]
        fn object_string_property(object: QObjectMutPtr, name: &QString) -> QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, packages, READ, CONSTANT)]
        #[qproperty(QObjectMutPtr, queue_model, READ, CONSTANT)]
        #[qproperty(i32, installed_count, READ, NOTIFY)]
        #[qproperty(i32, queue_count, READ, NOTIFY)]
        #[qproperty(i32, updates_count, READ, NOTIFY)]
        #[qproperty(QString, error, READ, NOTIFY)]
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

        #[qinvokable]
        fn reset(self: Pin<&mut InstalledBackend>);

        // Called by `khaxbps_installed_apply_plan` on the GUI thread when a job
        // finishes; not invokable so QML cannot bypass the job.
        fn apply_plan_items(self: Pin<&mut InstalledBackend>, items_json: &QString);
    }

    impl cxx_qt::Constructor<()> for InstalledBackend {}
}

/// Rust side of [`qobject::InstalledBackend`].
pub struct InstalledBackendRust {
    catalog: Arc<Catalog>,
    store: Store,
    /// Child model created by [`cxx_qt::Initialize::initialize`]; Qt owns it.
    packages: QObjectMutPtr,
    queue_model: QObjectMutPtr,
    installed_count: i32,
    queue_count: i32,
    updates_count: i32,
    error: QString,
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
            installed_count: 0,
            queue_count: 0,
            updates_count: 0,
            error: QString::from(""),
        }
    }
}

impl cxx_qt::Initialize for qobject::InstalledBackend {
    /// Loads the catalog, then the store, then creates both child models.
    ///
    /// A store that cannot be loaded is not fatal: the seed image stays in
    /// memory and `error` carries a technical reason for QML to wrap in a
    /// translated sentence.
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
        let installed_json = json_of(&store.installed);
        let queue_json = json_of(&store.queue);
        let packages = qobject::create_package_model(parent, &model_json, &installed_json);
        let queue_model = qobject::create_queue_model(parent, &queue_json);

        let repos = repos_from_installed(&store.installed);
        let updates = catalog.updates(&store.installed, &repos).len();
        let installed_count = store.installed.len();
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
        data.installed_count = i32::try_from(installed_count).unwrap_or(i32::MAX);
        data.queue_count = i32::try_from(queue_count).unwrap_or(i32::MAX);
        data.updates_count = i32::try_from(updates).unwrap_or(i32::MAX);
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
            &self.store.queue,
        );
        self.plan_object(&payload)
    }

    fn open_system_plan(&self) -> QObjectMutPtr {
        let payload = system_payload(
            &self.catalog,
            &self.store.installed,
            &repos_from_installed(&self.store.installed),
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

    fn reset(mut self: Pin<&mut Self>) {
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store.reset();
        }
        save_store(self.as_mut());
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
            let repos = repos_from_installed(&data.store.installed);
            if data
                .catalog
                .updates(&data.store.installed, &repos)
                .is_empty()
            {
                let signature =
                    update_signature(&data.catalog, &data.store.installed, &repos).to_string();
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
pub fn queue_payload(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
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
            let held = held_in_queue(catalog, installed, repos, queue);
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
pub fn system_payload(catalog: &Catalog, installed: &InstalledMap, repos: &Repos) -> PlanPayload {
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
            let held = held_updates(catalog, installed, repos);
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
/// [`held_updates`] returns every held package with a pending update; the note
/// on a queue plan only concerns the ones the user queued.
pub fn held_in_queue(
    catalog: &Catalog,
    installed: &InstalledMap,
    repos: &Repos,
    queue: &[QueueItem],
) -> Vec<String> {
    let updating: Vec<&str> = queue
        .iter()
        .filter(|item| item.action == QueueAction::Update)
        .map(|item| item.name.as_str())
        .collect();
    held_updates(catalog, installed, repos)
        .into_iter()
        .filter(|name| updating.contains(&name.as_str()))
        .collect()
}

/// Applies a finished plan and refreshes the model and the counters.
fn refresh(mut self_: Pin<&mut qobject::InstalledBackend>) {
    let (
        installed_json,
        queue_json,
        installed_count,
        queue_count,
        updates_count,
        packages,
        queue_model,
    ) = {
        let data = &*self_;
        let repos = repos_from_installed(&data.store.installed);
        (
            json_of(&data.store.installed),
            json_of(&data.store.queue),
            i32::try_from(data.store.installed.len()).unwrap_or(i32::MAX),
            i32::try_from(data.store.queue.len()).unwrap_or(i32::MAX),
            i32::try_from(data.catalog.updates(&data.store.installed, &repos).len())
                .unwrap_or(i32::MAX),
            data.packages,
            data.queue_model,
        )
    };

    if !packages.as_ptr().is_null() {
        qobject::package_model_set_state(packages, &installed_json, &queue_json);
    }
    if !queue_model.as_ptr().is_null() {
        qobject::queue_model_set_state(queue_model, &queue_json);
    }

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
    use khaxbps_core::{PlanAction, PlanReason, SEED_AT};

    fn catalog() -> Catalog {
        Catalog::load_from_str(include_str!("../resources/catalog.json")).unwrap()
    }

    fn seed() -> (Catalog, InstalledMap) {
        let catalog = catalog();
        let store = Store::fresh(&catalog);
        (catalog, store.installed)
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
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
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
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
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
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
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
        let payload = queue_payload(
            &catalog,
            &installed,
            &repos_from_installed(&installed),
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
        let updates = catalog.updates(&installed, &repos);
        let payload = system_payload(&catalog, &installed, &repos);
        assert_eq!(payload.title_key, "planSystem");
        assert_eq!(payload.items.len(), updates.len());
        assert!(!updates.is_empty(), "the seed pins outdated versions");
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
        let payload = queue_payload(&catalog, &installed, &Repos::default(), &[]);
        assert_eq!(payload.notice_key, "emptyQueue");
        assert!(payload.items.is_empty());
    }

    #[test]
    fn held_note_only_mentions_packages_the_queue_updates() {
        let (catalog, mut installed) = seed();
        let repos = repos_from_installed(&installed);
        let updates = catalog.updates(&installed, &repos);
        assert!(updates.len() >= 2, "the seed has several outdated packages");
        let held_name = updates[0].name.clone();
        installed.get_mut(&held_name).unwrap().held = true;

        let queued = queue(&held_name, QueueAction::Update);
        assert_eq!(
            held_in_queue(&catalog, &installed, &repos, &queued),
            vec![held_name.clone()]
        );
        assert!(
            held_in_queue(&catalog, &installed, &repos, &[]).is_empty(),
            "nothing is queued, so nothing is skipped"
        );
    }

    #[test]
    fn queue_actions_parse_from_the_qml_strings() {
        assert_eq!(parse_action("install"), QueueAction::Install);
        assert_eq!(parse_action("remove"), QueueAction::Remove);
        assert_eq!(parse_action("update"), QueueAction::Update);
        assert_eq!(parse_action("nonsense"), QueueAction::Install);
    }
}
