//! Read-only `QAbstractItemModel` over the embedded package catalog.
//!
//! The catalog is parsed once at construction and never copied: the model keeps
//! an `Arc<Catalog>` plus a `rows` vector of indices into it. Changing the
//! filter, the search text or the category only recomputes `rows` and emits a
//! full model reset, so no package struct is cloned when the user clicks a chip.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
use khaxbps_core::{
    Catalog, Category, Inst, InstalledMap, Pkg, QueueAction, RepoId, byte_decimals, format_bytes,
    repos_from_installed,
};
use qobject::{QHash_i32_QByteArray, QObjectMutPtr};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qbytearray.h");
        type QByteArray = cxx_qt_lib::QByteArray;

        include!("cxx-qt-lib/core/qhash/qhash_i32_QByteArray.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qobjectmutptr.h");
        type QObjectMutPtr = cxx_qt_lib::QObjectMutPtr;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        // Base class of the model; the header comes from `model_shim.h`.
        include!("model_shim.h");
        type QAbstractItemModel;

        // Protected `QAbstractItemModel` members, forwarded by the shim.
        #[cxx_name = "khaxbps_model_begin_reset"]
        fn model_begin_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "khaxbps_model_end_reset"]
        fn model_end_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "khaxbps_model_create_index"]
        fn model_create_index(model: &QAbstractItemModel, row: i32, column: i32) -> QModelIndex;
    }

    // `auto_cxx_name` makes the properties and the overridden model methods
    // visible to QML under their camelCase Qt names (`rowCount`, `roleNames`).
    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractItemModel]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(QString, filter, READ, WRITE = set_filter, NOTIFY)]
        #[qproperty(QString, search, READ, WRITE = set_search, NOTIFY)]
        #[qproperty(QString, category, READ, WRITE = set_category, NOTIFY)]
        #[qproperty(i32, count, READ, NOTIFY)]
        type PackageModel = super::PackageModelRust;

        #[qinvokable]
        #[cxx_override]
        fn row_count(self: &PackageModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn column_count(self: &PackageModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &PackageModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        fn role_names(self: &PackageModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        fn index(self: &PackageModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &PackageModel, index: &QModelIndex) -> QModelIndex;

        fn set_filter(self: Pin<&mut PackageModel>, value: QString);
        fn set_search(self: Pin<&mut PackageModel>, value: QString);
        fn set_category(self: Pin<&mut PackageModel>, value: QString);
    }

    // QML never creates the model: `CatalogBackend` owns it and hands it out as
    // a `QObject*`. The constructor takes the catalog JSON and the installed
    // snapshot so a fresh model is fully populated before it is visible.
    impl
        cxx_qt::Constructor<
            (QString, QString, QObjectMutPtr),
            BaseArguments = (QObjectMutPtr,),
            NewArguments = (QString, QString),
        > for PackageModel
    {
    }
}

/// Role ids handed to `data()`; QML reads the names from [`ROLES`].
///
/// Every id starts at `Qt::UserRole` (256). Ids 0-255 belong to Qt
/// (`DisplayRole` 0, `EditRole` 2, `CheckStateRole` 10, `SizeHintRole` 13 …):
/// reusing them would feed a package name into a stock view's display role and
/// break any proxy model that reads the built-in roles.
pub mod role {
    /// `Qt::UserRole`.
    const USER_ROLE: i32 = 256;

    pub const NAME: i32 = USER_ROLE;
    pub const VERSION: i32 = USER_ROLE + 1;
    pub const REPO: i32 = USER_ROLE + 2;
    pub const CATEGORY: i32 = USER_ROLE + 3;
    pub const SIZE: i32 = USER_ROLE + 4;
    pub const DOWNLOAD: i32 = USER_ROLE + 5;
    pub const SHORT: i32 = USER_ROLE + 6;
    pub const HOMEPAGE: i32 = USER_ROLE + 7;
    pub const LICENSE: i32 = USER_ROLE + 8;
    pub const DEPENDS: i32 = USER_ROLE + 9;
    pub const INSTALLED_VERSION: i32 = USER_ROLE + 10;
    pub const EXPLICIT: i32 = USER_ROLE + 11;
    pub const HELD: i32 = USER_ROLE + 12;
    pub const QUEUED_ACTION: i32 = USER_ROLE + 13;
    pub const IS_UPDATE: i32 = USER_ROLE + 14;
    pub const UPDATE_VERSION: i32 = USER_ROLE + 15;
    pub const SIZE_BYTES: i32 = USER_ROLE + 16;
    pub const DOWNLOAD_BYTES: i32 = USER_ROLE + 17;
}

/// Role ids and the names QML sees, in declaration order.
///
/// `data()` and the `QVariantMap`s built by `CatalogBackend` share this table, so
/// a delegate reading `model.installedVersion` and a query result reading
/// `result.installedVersion` always agree.
pub const ROLES: [(i32, &str); 18] = [
    (role::NAME, "name"),
    (role::VERSION, "version"),
    (role::REPO, "repo"),
    (role::CATEGORY, "category"),
    (role::SIZE, "size"),
    (role::DOWNLOAD, "download"),
    (role::SHORT, "short"),
    (role::HOMEPAGE, "homepage"),
    (role::LICENSE, "license"),
    (role::DEPENDS, "depends"),
    (role::INSTALLED_VERSION, "installedVersion"),
    (role::EXPLICIT, "explicit"),
    (role::HELD, "held"),
    (role::QUEUED_ACTION, "queuedAction"),
    (role::IS_UPDATE, "isUpdate"),
    (role::UPDATE_VERSION, "updateVersion"),
    (role::SIZE_BYTES, "sizeBytes"),
    (role::DOWNLOAD_BYTES, "downloadBytes"),
];

/// Everything one row needs beyond the package itself.
pub struct RowView<'a> {
    pub pkg: &'a Pkg,
    pub installed: Option<&'a Inst>,
    pub queued: Option<QueueAction>,
    pub update_to: Option<&'a str>,
}

/// Value of `role` for one row; an invalid `QVariant` for unknown roles.
pub fn role_value(role: i32, row: &RowView<'_>) -> QVariant {
    let pkg = row.pkg;
    match role {
        role::NAME => text(&pkg.name),
        role::VERSION => text(&pkg.version),
        role::REPO => text(repo_key(pkg.repo)),
        role::CATEGORY => text(category_key(pkg.category)),
        role::SIZE => text(&format_size(pkg.size)),
        role::DOWNLOAD => text(&format_size(pkg.download)),
        role::SHORT => text(&pkg.short),
        role::HOMEPAGE => text(&pkg.homepage),
        role::LICENSE => text(&pkg.license),
        role::DEPENDS => text(&pkg.depends.join(", ")),
        role::INSTALLED_VERSION => text(row.installed.map_or("", |inst| inst.version.as_str())),
        role::EXPLICIT => boolean(row.installed.is_some_and(|inst| inst.explicit)),
        role::HELD => boolean(row.installed.is_some_and(|inst| inst.held)),
        role::QUEUED_ACTION => text(row.queued.map_or("", queue_key)),
        role::IS_UPDATE => boolean(row.update_to.is_some()),
        role::UPDATE_VERSION => text(row.update_to.unwrap_or("")),
        role::SIZE_BYTES => integer(pkg.size),
        role::DOWNLOAD_BYTES => integer(pkg.download),
        _ => QVariant::default(),
    }
}

/// Value for a map key: the role table plus `files`, which only the query view
/// needs and which has no role.
pub fn field_value(name: &str, row: &RowView<'_>) -> Option<QVariant> {
    if name == "files" {
        return Some(text(&row.pkg.files.join("\n")));
    }
    ROLES
        .iter()
        .find(|(_, key)| *key == name)
        .map(|(id, _)| role_value(*id, row))
}

/// Rust side of [`qobject::PackageModel`].
pub struct PackageModelRust {
    catalog: Arc<Catalog>,
    /// Catalog indices ordered by package name; the visible order of every filter.
    order: Vec<u32>,
    /// Indices of the packages currently visible, in `order` sequence.
    rows: Vec<u32>,
    installed: InstalledMap,
    queue: HashMap<String, QueueAction>,
    /// name -> catalog version, for packages that have an update.
    update_to: HashMap<String, String>,
    orphans: HashSet<String>,
    filter: QString,
    search: QString,
    category: QString,
    count: i32,
}

impl PackageModelRust {
    /// Parses the catalog and the installed snapshot.
    ///
    /// `CatalogBackend` validates the catalog before handing it over, so a parse
    /// failure here can only mean the two halves disagree; the model then starts
    /// empty instead of panicking.
    fn new(catalog_json: &str, installed_json: &str) -> Self {
        let catalog = Catalog::load_from_str(catalog_json).unwrap_or_default();
        let installed: InstalledMap = serde_json::from_str(installed_json).unwrap_or_default();
        Self::with_data(Arc::new(catalog), installed)
    }

    fn with_data(catalog: Arc<Catalog>, installed: InstalledMap) -> Self {
        let mut order: Vec<u32> = (0..catalog.len() as u32).collect();
        order.sort_by(|left, right| {
            catalog.packages()[*left as usize]
                .name
                .cmp(&catalog.packages()[*right as usize].name)
        });
        let mut model = Self {
            catalog,
            order,
            rows: Vec::new(),
            installed,
            queue: HashMap::new(),
            update_to: HashMap::new(),
            orphans: HashSet::new(),
            filter: QString::from("all"),
            search: QString::from(""),
            category: QString::from("all"),
            count: 0,
        };
        model.refresh_derived();
        model.rebuild();
        model
    }

    fn package_at(&self, row: i32) -> Option<&Pkg> {
        let index = usize::try_from(row).ok()?;
        let catalog_index = *self.rows.get(index)?;
        self.catalog.packages().get(catalog_index as usize)
    }

    fn row_view<'a>(&'a self, pkg: &'a Pkg) -> RowView<'a> {
        RowView {
            pkg,
            installed: self.installed.get(&pkg.name),
            queued: self.queue.get(&pkg.name).copied(),
            update_to: self.update_to.get(&pkg.name).map(String::as_str),
        }
    }

    /// Recomputes the state derived from the installed snapshot.
    ///
    /// `updates` comes from the core so hold, repository and security rules stay
    /// in one place; the repository flags are derived from the installed map
    /// exactly like the web app does (`reposFromInstalled`).
    fn refresh_derived(&mut self) {
        let repos = repos_from_installed(&self.installed);
        self.update_to = self
            .catalog
            .updates(&self.installed, &repos)
            .into_iter()
            .map(|update| (update.name, update.to))
            .collect();
        self.orphans = self
            .installed
            .iter()
            .filter(|(name, inst)| {
                !inst.explicit
                    && !self
                        .catalog
                        .revdeps(name)
                        .iter()
                        .any(|dependent| self.installed.contains_key(&dependent.name))
            })
            .map(|(name, _)| name.clone())
            .collect();
    }

    /// Visible catalog indices for the current filter, search and category.
    ///
    /// Built into a fresh vector so a caller can compare before committing: a
    /// keystroke in the search field that does not change the visible set must
    /// not reset the model, or QML loses the scroll position and selection.
    fn compute_rows(&self) -> Vec<u32> {
        let filter = self.filter.to_string();
        let category = self.category.to_string();
        let search = self.search.to_string().to_lowercase();
        let mut rows = Vec::new();
        for &catalog_index in &self.order {
            let pkg = &self.catalog.packages()[catalog_index as usize];
            if self.matches(pkg, &filter, &category, &search) {
                rows.push(catalog_index);
            }
        }
        rows
    }

    /// Commits `rows` as the visible set and keeps `count` in step.
    fn assign_rows(&mut self, rows: Vec<u32>) {
        self.count = i32::try_from(rows.len()).unwrap_or(i32::MAX);
        self.rows = rows;
    }

    /// Recomputes `rows` and `count`; returns whether the visible set changed.
    fn rebuild(&mut self) -> bool {
        let rows = self.compute_rows();
        if rows == self.rows {
            return false;
        }
        self.assign_rows(rows);
        true
    }

    fn matches(&self, pkg: &Pkg, filter: &str, category: &str, search: &str) -> bool {
        if category != "all" && category_key(pkg.category) != category {
            return false;
        }
        if !search.is_empty()
            && !pkg.name.to_lowercase().contains(search)
            && !pkg.short.to_lowercase().contains(search)
        {
            return false;
        }
        let installed = self.installed.get(&pkg.name);
        match filter {
            "installed" => installed.is_some(),
            "updates" => self.update_to.contains_key(&pkg.name),
            "explicit" => installed.is_some_and(|inst| inst.explicit),
            "orphans" => self.orphans.contains(&pkg.name),
            "held" => installed.is_some_and(|inst| inst.held),
            "queued" => self.queue.contains_key(&pkg.name),
            _ => true,
        }
    }
}

impl cxx_qt::Constructor<(QString, QString, QObjectMutPtr)> for qobject::PackageModel {
    type BaseArguments = (QObjectMutPtr,);
    type NewArguments = (QString, QString);
    type InitializeArguments = ();

    fn route_arguments(
        arguments: (QString, QString, QObjectMutPtr),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        let (catalog_json, installed_json, parent) = arguments;
        ((catalog_json, installed_json), (parent,), ())
    }

    fn new((catalog_json, installed_json): Self::NewArguments) -> PackageModelRust {
        PackageModelRust::new(&catalog_json.to_string(), &installed_json.to_string())
    }
}

impl qobject::PackageModel {
    fn row_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { self.count }
    }

    fn column_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { 1 }
    }

    fn data(&self, index: &cxx_qt_lib::QModelIndex, role: i32) -> cxx_qt_lib::QVariant {
        let Some(pkg) = self.package_at(index.row()) else {
            return cxx_qt_lib::QVariant::default();
        };
        role_value(role, &self.row_view(pkg))
    }

    fn role_names(&self) -> QHash_i32_QByteArray {
        ROLES
            .iter()
            .map(|(id, name)| (*id, QByteArray::from(*name)))
            .collect()
    }

    fn index(
        &self,
        row: i32,
        column: i32,
        parent: &cxx_qt_lib::QModelIndex,
    ) -> cxx_qt_lib::QModelIndex {
        if parent.is_valid() || column != 0 || row < 0 || row >= self.count {
            return cxx_qt_lib::QModelIndex::default();
        }
        let base = <qobject::PackageModel as Upcast<qobject::QAbstractItemModel>>::upcast(self);
        qobject::model_create_index(base, row, column)
    }

    fn parent(&self, _index: &cxx_qt_lib::QModelIndex) -> cxx_qt_lib::QModelIndex {
        cxx_qt_lib::QModelIndex::default()
    }

    fn set_filter(mut self: Pin<&mut Self>, value: cxx_qt_lib::QString) {
        if self.filter == value {
            return;
        }
        apply_change(self.as_mut(), |model| model.filter = value);
        self.as_mut().filter_changed();
    }

    fn set_search(mut self: Pin<&mut Self>, value: cxx_qt_lib::QString) {
        if self.search == value {
            return;
        }
        apply_change(self.as_mut(), |model| model.search = value);
        self.as_mut().search_changed();
    }

    fn set_category(mut self: Pin<&mut Self>, value: cxx_qt_lib::QString) {
        if self.category == value {
            return;
        }
        apply_change(self.as_mut(), |model| model.category = value);
        self.as_mut().category_changed();
    }
}

/// Applies a property change and resets the model only if the visible set moved.
///
/// The new rows are computed before anything is committed, so a search keystroke
/// that matches exactly the same packages costs one comparison instead of a full
/// `beginResetModel`/`endResetModel` cycle (which would rebuild every delegate
/// and drop the scroll position and selection).
fn apply_change(
    mut model: Pin<&mut qobject::PackageModel>,
    mutate: impl FnOnce(&mut PackageModelRust),
) {
    let rows = {
        let data = Pin::get_mut(model.as_mut().rust_mut());
        mutate(data);
        data.compute_rows()
    };
    let unchanged = {
        let data = Pin::get_mut(model.as_mut().rust_mut());
        data.rows == rows
    };
    if unchanged {
        return;
    }

    qobject::model_begin_reset(base(model.as_mut()));
    {
        let data = Pin::get_mut(model.as_mut().rust_mut());
        data.assign_rows(rows);
    }
    qobject::model_end_reset(base(model.as_mut()));
    model.count_changed();
}

fn base(model: Pin<&mut qobject::PackageModel>) -> Pin<&mut qobject::QAbstractItemModel> {
    <qobject::PackageModel as Upcast<qobject::QAbstractItemModel>>::upcast_pin(model)
}

fn text(value: &str) -> QVariant {
    QVariant::from(&QString::from(value))
}

fn boolean(value: bool) -> QVariant {
    QVariant::from(&value)
}

/// Raw byte count, for sorting and arithmetic in QML; the formatted `size` /
/// `download` roles are what the delegate displays.
fn integer(value: u64) -> QVariant {
    QVariant::from(&i64::try_from(value).unwrap_or(i64::MAX))
}

/// `4.0 MiB`, using the core's thresholds and the web app's decimal count.
fn format_size(bytes: u64) -> String {
    let bytes = i64::try_from(bytes).unwrap_or(i64::MAX);
    let (value, unit) = format_bytes(bytes);
    format!(
        "{:.*} {}",
        usize::from(byte_decimals(bytes)),
        value,
        unit.symbol()
    )
}

fn repo_key(repo: RepoId) -> &'static str {
    match repo {
        RepoId::Current => "current",
        RepoId::Nonfree => "nonfree",
        RepoId::Multilib => "multilib",
        RepoId::Debug => "debug",
    }
}

fn category_key(category: Category) -> &'static str {
    match category {
        Category::Base => "base",
        Category::Devel => "devel",
        Category::Desktop => "desktop",
        Category::Net => "net",
        Category::Media => "media",
        Category::Graphics => "graphics",
        Category::Office => "office",
        Category::Games => "games",
        Category::Fonts => "fonts",
        Category::Admin => "admin",
    }
}

fn queue_key(action: QueueAction) -> &'static str {
    match action {
        QueueAction::Install => "install",
        QueueAction::Remove => "remove",
        QueueAction::Update => "update",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use khaxbps_core::{ListFilter, Store};

    fn catalog() -> Arc<Catalog> {
        Arc::new(Catalog::load_from_str(include_str!("../resources/catalog.json")).unwrap())
    }

    fn model_with(installed: InstalledMap) -> PackageModelRust {
        PackageModelRust::with_data(catalog(), installed)
    }

    fn visible_names(model: &PackageModelRust) -> Vec<&str> {
        model
            .rows
            .iter()
            .map(|index| model.catalog.packages()[*index as usize].name.as_str())
            .collect()
    }

    fn set_filter(model: &mut PackageModelRust, filter: &str) {
        model.filter = QString::from(filter);
        model.rebuild();
    }

    #[test]
    fn all_filter_shows_the_whole_catalog_in_name_order() {
        let model = model_with(InstalledMap::new());
        assert_eq!(model.rows.len(), 115);
        assert_eq!(model.count, 115);
        let names = visible_names(&model);
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "rows follow the catalog's name order");
    }

    #[test]
    fn installed_filter_matches_the_seed_image() {
        let store = Store::fresh(&catalog());
        assert_eq!(store.installed.len(), 56, "seed image size");
        let mut model = model_with(store.installed.clone());
        assert_eq!(model.rows.len(), 115, "no filter shows everything");
        set_filter(&mut model, "installed");
        assert_eq!(model.rows.len(), 56);
        assert_eq!(model.count, 56);
        assert!(visible_names(&model).contains(&"vim"));
        assert!(
            !visible_names(&model).contains(&"ark"),
            "ark is not in the seed image"
        );
    }

    #[test]
    fn updates_filter_uses_the_core_update_rules() {
        let mut installed = Store::fresh(&catalog()).installed;
        let before = model_with(installed.clone()).update_to.len();
        assert!(before > 0, "the seed image has outdated packages");
        // Holding every installed package must clear the update list.
        for inst in installed.values_mut() {
            inst.held = true;
        }
        let mut model = model_with(installed);
        set_filter(&mut model, "updates");
        assert_eq!(model.rows.len(), 0, "held packages are not updates");
    }

    #[test]
    fn explicit_and_orphan_filters_are_disjoint_for_the_seed() {
        let store = Store::fresh(&catalog());
        let mut explicit = model_with(store.installed.clone());
        set_filter(&mut explicit, "explicit");
        let explicit_names: HashSet<&str> = visible_names(&explicit).into_iter().collect();
        assert!(!explicit_names.is_empty());
        assert!(explicit_names.contains("vim"));

        let mut orphans = model_with(store.installed.clone());
        set_filter(&mut orphans, "orphans");
        let orphan_names: HashSet<&str> = visible_names(&orphans).into_iter().collect();
        assert!(orphan_names.is_disjoint(&explicit_names));
        assert!(
            orphan_names
                .iter()
                .all(|name| !store.installed[*name].explicit)
        );
    }

    #[test]
    fn queued_filter_follows_the_queue_map() {
        let mut model = model_with(InstalledMap::new());
        set_filter(&mut model, "queued");
        assert_eq!(model.rows.len(), 0, "Task 6 has no queue yet");

        model.queue.insert("vim".to_string(), QueueAction::Install);
        model.rebuild();
        assert_eq!(visible_names(&model), vec!["vim"]);
    }

    #[test]
    fn update_roles_describe_an_outdated_package() {
        let store = Store::fresh(&catalog());
        let model = model_with(store.installed.clone());
        // The seed pins `linux` below the catalog version on purpose.
        let pkg = model.catalog.by_name("linux").unwrap();
        let row = model.row_view(pkg);
        assert!(
            role_value(role::IS_UPDATE, &row).value::<bool>().unwrap(),
            "linux has an update"
        );
        assert_eq!(
            role_value(role::UPDATE_VERSION, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            pkg.version
        );
        assert_ne!(
            role_value(role::INSTALLED_VERSION, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            pkg.version
        );
    }

    #[test]
    fn rebuild_reports_no_change_when_the_visible_set_is_identical() {
        let mut model = model_with(InstalledMap::new());
        model.search = QString::from("vim");
        assert!(model.rebuild(), "first narrowing changes the rows");
        let before = model.rows.clone();

        // Same matches, different spelling: QML must not reset the model.
        model.search = QString::from("VIM");
        assert!(!model.rebuild(), "same visible set, no reset");
        assert_eq!(model.rows, before);
        assert_eq!(model.count, i32::try_from(before.len()).unwrap());

        model.search = QString::from("vim-spell");
        assert!(model.rebuild(), "a narrower query changes the rows");
        assert!(model.rows.len() < before.len());
    }

    #[test]
    fn search_and_category_narrow_without_touching_the_catalog() {
        let mut model = model_with(InstalledMap::new());
        let catalog_ptr = Arc::as_ptr(&model.catalog);
        model.search = QString::from("vim");
        model.rebuild();
        assert!(
            visible_names(&model)
                .iter()
                .all(|name| name.contains("vim"))
        );
        assert_eq!(
            Arc::as_ptr(&model.catalog),
            catalog_ptr,
            "catalog not rebuilt"
        );

        model.search = QString::from("");
        model.category = QString::from("devel");
        model.rebuild();
        assert!(!model.rows.is_empty());
        assert!(visible_names(&model).iter().all(|name| {
            model
                .catalog
                .by_name(name)
                .is_some_and(|pkg| pkg.category == Category::Devel)
        }));
    }

    #[test]
    fn filter_names_match_the_core_enum() {
        // Guards against a QML-side typo drifting from `ListFilter`.
        for (id, filter) in [
            (0, ListFilter::All),
            (1, ListFilter::Installed),
            (2, ListFilter::Updates),
            (3, ListFilter::Explicit),
            (4, ListFilter::Orphans),
            (5, ListFilter::Held),
            (6, ListFilter::Queued),
        ] {
            let json = serde_json::to_string(&filter).unwrap();
            let json = json.trim_matches('"');
            let mut model = model_with(InstalledMap::new());
            set_filter(&mut model, json);
            assert_eq!(model.filter.to_string(), json, "filter {id} round-trips");
        }
    }

    #[test]
    fn roles_cover_every_documented_name() {
        assert_eq!(ROLES.len(), 18);
        let names: Vec<&str> = ROLES.iter().map(|(_, name)| *name).collect();
        assert_eq!(
            names,
            vec![
                "name",
                "version",
                "repo",
                "category",
                "size",
                "download",
                "short",
                "homepage",
                "license",
                "depends",
                "installedVersion",
                "explicit",
                "held",
                "queuedAction",
                "isUpdate",
                "updateVersion",
                "sizeBytes",
                "downloadBytes",
            ]
        );
    }

    #[test]
    fn role_ids_start_at_user_role_and_are_unique() {
        // Qt owns 0-255 (`DisplayRole`, `EditRole`, `CheckStateRole`, …).
        let mut ids: Vec<i32> = ROLES.iter().map(|(id, _)| *id).collect();
        assert!(
            ids.iter().all(|id| *id >= 256),
            "ids must be >= Qt::UserRole"
        );
        ids.sort_unstable();
        let unique = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), unique, "role ids are unique");
        assert_eq!(role::NAME, 256);
    }

    #[test]
    fn role_values_describe_a_row() {
        let store = Store::fresh(&catalog());
        let model = model_with(store.installed.clone());
        let pkg = model.catalog.by_name("vim").unwrap();
        let row = model.row_view(pkg);
        assert_eq!(
            role_value(role::NAME, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "vim"
        );
        assert_eq!(
            role_value(role::REPO, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "current"
        );
        assert_eq!(
            role_value(role::INSTALLED_VERSION, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            store.installed["vim"].version
        );
        assert!(
            role_value(role::EXPLICIT, &row).value::<bool>().unwrap(),
            "vim is explicit"
        );
        assert_eq!(
            role_value(role::SIZE, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "4.0 MiB"
        );
        assert_eq!(
            role_value(role::SIZE_BYTES, &row).value::<i64>().unwrap(),
            4_194_304,
            "raw bytes for sorting"
        );
        assert_eq!(
            role_value(role::DOWNLOAD_BYTES, &row)
                .value::<i64>()
                .unwrap(),
            1_468_006
        );
        assert!(!role_value(role::IS_UPDATE, &row).value::<bool>().unwrap());
        assert!(
            role_value(role::DOWNLOAD_BYTES + 1, &row)
                .type_name()
                .is_empty(),
            "unknown role"
        );
    }
}
