//! Catalog façade for QML: the package model plus the query helpers.

use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QObjectMutPtr, QString, QVariant};
use khaxbps_core::{Catalog, QueryMode};

use crate::package_model::{self, RowView};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/core/qlist/qlist_QVariant.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;

        include!("cxx-qt-lib/core/qmap/qmap_QString_QVariant.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("model_shim.h");

        #[cxx_name = "khaxbps_read_resource"]
        fn read_resource(path: &QString) -> QString;

        #[cxx_name = "khaxbps_create_package_model"]
        fn create_package_model(
            parent: QObjectMutPtr,
            catalog_json: &QString,
            installed_json: &QString,
        ) -> QObjectMutPtr;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QObjectMutPtr, packages, READ, CONSTANT)]
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
    }

    impl cxx_qt::Constructor<()> for CatalogBackend {}
}

/// Rust side of [`qobject::CatalogBackend`].
pub struct CatalogBackendRust {
    /// The child model, created in [`cxx_qt::Initialize::initialize`] with this
    /// object as its parent. Rust never dereferences the pointer: Qt owns the
    /// model and destroys it together with the backend.
    packages: QObjectMutPtr,
    error: QString,
    /// Parsed catalog for `search` / `get`.
    ///
    /// The model parses the same JSON for its rows; sharing the value across the
    /// C++ object boundary is not possible without unsafe casts, and one extra
    /// parse of a 44 kB resource at startup is cheaper than that.
    catalog: Arc<Catalog>,
}

impl Default for CatalogBackendRust {
    /// Only used before `initialize` runs; `packages` stays null until then.
    fn default() -> Self {
        Self {
            packages: unsafe { QObjectMutPtr::from_raw(std::ptr::null_mut()) },
            error: QString::from(""),
            catalog: Arc::default(),
        }
    }
}

impl cxx_qt::Initialize for qobject::CatalogBackend {
    /// Reads the embedded catalog and creates the package model as a child.
    ///
    /// A missing or malformed resource must not abort the app: it becomes the
    /// `error` string QML shows in a banner, and the model starts empty.
    fn initialize(mut self: Pin<&mut Self>) {
        let json = qobject::read_resource(&QString::from(crate::CATALOG_RESOURCE_PATH));
        let (catalog, error, model_json) = load_catalog(&json);

        let parent = unsafe {
            QObjectMutPtr::from_raw(
                <qobject::CatalogBackend as Upcast<cxx_qt::QObject>>::upcast_ptr(&*self).cast_mut(),
            )
        };
        // Task 7 replaces this empty snapshot with the store's installed map.
        let packages = qobject::create_package_model(parent, &model_json, &QString::from("{}"));

        let backend = Pin::get_mut(self.as_mut().rust_mut());
        backend.packages = packages;
        backend.error = QString::from(error.as_str());
        backend.catalog = Arc::new(catalog);
    }
}

/// Turns the catalog resource into `(catalog, error, json for the model)`.
///
/// `error` is empty on success and a short technical reason otherwise; the
/// sentence around it belongs to QML's `i18nd`. The model always receives valid
/// JSON, so it never has to guess why it is empty.
fn load_catalog(resource: &QString) -> (Catalog, String, QString) {
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
}

impl CatalogBackendRust {
    /// One package as a `QVariantMap`, keyed exactly like the model's roles plus
    /// `files`. Task 6 has no installed snapshot, so the installed-derived keys
    /// are empty/false here; Task 7 fills them.
    fn map_for(&self, pkg: &khaxbps_core::Pkg) -> QMap<QMapPair_QString_QVariant> {
        let row = RowView {
            pkg,
            installed: None,
            queued: None,
            update_to: None,
        };
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
    fn embedded_catalog_loads_without_error() {
        let (catalog, error, json) =
            load_catalog(&QString::from(include_str!("../resources/catalog.json")));
        assert_eq!(error, "");
        assert_eq!(catalog.len(), 115);
        assert_eq!(json.to_string(), include_str!("../resources/catalog.json"));
    }
}
