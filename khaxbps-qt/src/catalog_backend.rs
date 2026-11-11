//! Catalog façade for QML: the query helpers over the embedded catalog.
//!
//! The package *model* is owned by [`crate::installed_backend::InstalledBackend`],
//! because it needs the store's installed and queue snapshots. This object
//! answers `search()` / `get()` and reports a catalog that failed to load — and
//! it receives the *same* snapshot from that backend (`queryBackend` in QML), so
//! a query result and a model row never disagree about what is installed.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QVariant};
use khaxbps_core::{Catalog, InstalledMap, Pkg, QueryMode, QueueAction, QueueItem};

use crate::package_model::{self, InstalledState, RowView};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/core/qlist/qlist_QVariant.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;

        include!("cxx-qt-lib/core/qmap/qmap_QString_QVariant.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, error, READ, CONSTANT)]
        type CatalogBackend = super::CatalogBackendRust;

        #[qinvokable]
        fn version(self: &CatalogBackend) -> QString;

        #[qinvokable]
        fn search(
            self: &CatalogBackend,
            query: &QString,
            mode: &QString,
            prop: &QString,
        ) -> QList_QVariant;

        #[qinvokable]
        fn get(self: &CatalogBackend, name: &QString) -> QMap_QString_QVariant;

        // Pushes the installed/queue snapshot; called by the installed backend
        // through `khaxbps_catalog_backend_set_state`.
        fn set_state(
            self: Pin<&mut CatalogBackend>,
            installed_json: &QString,
            queue_json: &QString,
        );
    }

    impl cxx_qt::Constructor<()> for CatalogBackend {}
}

/// Rust side of [`qobject::CatalogBackend`].
pub struct CatalogBackendRust {
    error: QString,
    catalog: Arc<Catalog>,
    /// The same snapshot the package model holds.
    installed: InstalledMap,
    queue: HashMap<String, QueueAction>,
    /// Derived from `installed`, shared with the model.
    state: InstalledState,
}

impl Default for CatalogBackendRust {
    fn default() -> Self {
        Self {
            error: QString::from(""),
            catalog: Arc::default(),
            installed: InstalledMap::new(),
            queue: HashMap::new(),
            state: InstalledState::new(&Catalog::default(), &InstalledMap::new()),
        }
    }
}

impl cxx_qt::Initialize for qobject::CatalogBackend {
    /// Reads the embedded catalog. A missing or malformed resource must not
    /// abort the app: it becomes the `error` string QML shows in a banner.
    fn initialize(mut self: Pin<&mut Self>) {
        let json = crate::qobject::read_resource(&QString::from(crate::CATALOG_RESOURCE_PATH));
        let (catalog, error, _) = load_catalog(&json);
        let backend = Pin::get_mut(self.as_mut().rust_mut());
        backend.error = QString::from(error.as_str());
        backend.catalog = Arc::new(catalog);
    }
}

/// Turns the catalog resource into `(catalog, error, json for the model)`.
///
/// `error` is empty on success and a short technical reason otherwise; the
/// sentence around it belongs to QML's `i18nd`. The third element is the JSON a
/// package model can parse, so a caller never has to guess why it is empty.
pub(crate) fn load_catalog(resource: &QString) -> (Catalog, String, QString) {
    if resource.is_empty() {
        return (
            Catalog::default(),
            format!("resource not found: {}", crate::CATALOG_RESOURCE_PATH),
            QString::from("[]"),
        );
    }
    match Catalog::load_from_str(&resource.to_string()) {
        Ok(catalog) => (catalog, String::new(), resource.clone()),
        Err(err) => (
            Catalog::default(),
            format!("invalid catalog JSON: {err}"),
            QString::from("[]"),
        ),
    }
}

impl qobject::CatalogBackend {
    fn version(&self) -> QString {
        QString::from(env!("CARGO_PKG_VERSION"))
    }

    fn search(
        &self,
        query: &cxx_qt_lib::QString,
        mode: &cxx_qt_lib::QString,
        prop: &cxx_qt_lib::QString,
    ) -> QList<QVariant> {
        let mode = parse_mode(&mode.to_string());
        let hits = self
            .catalog
            .search(&query.to_string(), mode, &prop.to_string());
        QList::from(
            hits.iter()
                .map(|pkg| QVariant::from(&self.map_for(pkg)))
                .collect::<Vec<_>>(),
        )
    }

    fn get(&self, name: &cxx_qt_lib::QString) -> QMap<QMapPair_QString_QVariant> {
        self.catalog
            .by_name(&name.to_string())
            .map_or_else(QMap::default, |pkg| self.map_for(pkg))
    }

    fn set_state(
        mut self: Pin<&mut Self>,
        installed_json: &cxx_qt_lib::QString,
        queue_json: &cxx_qt_lib::QString,
    ) {
        let installed: InstalledMap =
            serde_json::from_str(&installed_json.to_string()).unwrap_or_default();
        let queue: Vec<QueueItem> =
            serde_json::from_str(&queue_json.to_string()).unwrap_or_default();
        let data = Pin::get_mut(self.as_mut().rust_mut());
        data.apply_snapshot(installed, queue);
    }
}

impl CatalogBackendRust {
    /// Replaces the installed and queue snapshots with the store's own values.
    ///
    /// `InstalledState::new` is the same call the package model makes, so
    /// `isUpdate`/`updateVersion` are derived once, in one place.
    fn apply_snapshot(&mut self, installed: InstalledMap, queue: Vec<QueueItem>) {
        self.state = InstalledState::new(&self.catalog, &installed);
        self.installed = installed;
        self.queue = queue
            .into_iter()
            .map(|item| (item.name, item.action))
            .collect();
    }

    /// One row of the snapshot, exactly as the model builds it.
    fn row_view<'a>(&'a self, pkg: &'a Pkg) -> RowView<'a> {
        RowView {
            pkg,
            installed: self.installed.get(&pkg.name),
            queued: self.queue.get(&pkg.name).copied(),
            update_to: self.state.update_to(&pkg.name),
        }
    }

    /// One package as a `QVariantMap`, keyed exactly like the model's roles plus
    /// `files`.
    fn map_for(&self, pkg: &khaxbps_core::Pkg) -> QMap<QMapPair_QString_QVariant> {
        let row = self.row_view(pkg);
        let mut map = QMap::default();
        for (_, name) in package_model::ROLES {
            if let Some(value) = package_model::field_value(name, &row) {
                map.insert_clone(&QString::from(name), &value);
            }
        }
        if let Some(files) = package_model::field_value("files", &row) {
            map.insert_clone(&QString::from("files"), &files);
        }
        map
    }
}

/// The `xbps-query` mode token QML sends (`"Rs"`, `"show"`, …); anything
/// unknown falls back to `-Rs`, like the web app's `MODES[0]`.
fn parse_mode(token: &str) -> QueryMode {
    match token {
        "s" => QueryMode::S,
        "l" => QueryMode::L,
        "R" => QueryMode::R,
        "show" => QueryMode::Show,
        "x" => QueryMode::X,
        "X" => QueryMode::RevX,
        "f" => QueryMode::F,
        "o" => QueryMode::O,
        "m" => QueryMode::M,
        "O" => QueryMode::Orphans,
        "H" => QueryMode::H,
        "L" => QueryMode::RepoL,
        "p" => QueryMode::P,
        _ => QueryMode::Rs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use khaxbps_core::Store;

    fn text_of(map: &QMap<QMapPair_QString_QVariant>, key: &str) -> String {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<QString>())
            .map(|value| value.to_string())
            .unwrap_or_default()
    }

    fn flag_of(map: &QMap<QMapPair_QString_QVariant>, key: &str) -> bool {
        map.get(&QString::from(key))
            .and_then(|value| value.value::<bool>())
            .unwrap_or(false)
    }

    #[test]
    fn every_mode_token_round_trips() {
        for mode in [
            QueryMode::Rs,
            QueryMode::S,
            QueryMode::L,
            QueryMode::R,
            QueryMode::Show,
            QueryMode::X,
            QueryMode::RevX,
            QueryMode::F,
            QueryMode::O,
            QueryMode::M,
            QueryMode::Orphans,
            QueryMode::H,
            QueryMode::RepoL,
            QueryMode::P,
        ] {
            let token = serde_json::to_string(&mode).unwrap();
            let token = token.trim_matches('"');
            assert_eq!(parse_mode(token), mode, "token {token}");
        }
        assert_eq!(parse_mode("nonsense"), QueryMode::Rs);
    }

    #[test]
    fn missing_resource_is_an_error_not_a_panic() {
        let (catalog, error, json) = load_catalog(&QString::from(""));
        assert!(catalog.is_empty());
        assert!(error.starts_with("resource not found: "), "got {error}");
        assert_eq!(json.to_string(), "[]");
    }

    #[test]
    fn malformed_resource_is_an_error_not_a_panic() {
        let (catalog, error, json) = load_catalog(&QString::from("{ not json"));
        assert!(catalog.is_empty());
        assert!(error.starts_with("invalid catalog JSON: "), "got {error}");
        assert_eq!(json.to_string(), "[]");
    }

    #[test]
    fn query_maps_carry_the_installed_snapshot() {
        let (catalog, error, _) =
            load_catalog(&QString::from(include_str!("../resources/catalog.json")));
        assert_eq!(error, "");
        let store = Store::fresh(&catalog);
        let mut backend = CatalogBackendRust {
            catalog: Arc::new(catalog),
            ..CatalogBackendRust::default()
        };
        backend.apply_snapshot(
            store.installed.clone(),
            vec![QueueItem {
                name: "ark".into(),
                action: QueueAction::Install,
            }],
        );

        let vim = backend.map_for(backend.catalog.by_name("vim").unwrap());
        assert_eq!(
            text_of(&vim, "installedVersion"),
            store.installed["vim"].version,
            "an installed package reports its version"
        );
        assert!(flag_of(&vim, "explicit"));
        assert!(!flag_of(&vim, "isUpdate"), "vim is up to date");

        let linux = backend.catalog.by_name("linux").unwrap();
        let linux_map = backend.map_for(linux);
        assert!(flag_of(&linux_map, "isUpdate"), "the seed pins linux");
        assert_eq!(text_of(&linux_map, "updateVersion"), linux.version);

        let ark = backend.map_for(backend.catalog.by_name("ark").unwrap());
        assert_eq!(text_of(&ark, "installedVersion"), "");
        assert!(!flag_of(&ark, "explicit"));
        assert_eq!(text_of(&ark, "queuedAction"), "install");
    }

    #[test]
    fn embedded_catalog_loads_without_error() {
        let (catalog, error, json) =
            load_catalog(&QString::from(include_str!("../resources/catalog.json")));
        assert_eq!(error, "");
        assert_eq!(catalog.len(), 115);
        assert_eq!(json.to_string(), include_str!("../resources/catalog.json"));
    }
}
