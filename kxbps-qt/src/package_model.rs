//! Read-only `QAbstractItemModel` over the embedded package catalog.
//!
//! The catalog is parsed once at construction and never copied: the model keeps
//! an `Arc<Catalog>` plus a `rows` vector of indices into it. Changing the
//! filter, the search text or the category only recomputes `rows` and emits a
//! full model reset, so no package struct is cloned when the user clicks a chip.
//!
//! The list itself is not the catalog: `InstalledBackend` pushes the snapshot
//! `xbps-query -l` answered, and then the rows are the installed records —
//! every package the system has, not the 115 the catalog knows. The catalog
//! stays as the known-package table: a name it has decorates its row with the
//! repository version, category and home page, and a name it does not have
//! shows an empty string where xbps has nothing cheap to offer. `Available`
//! (filter `all`) keeps listing the catalog alone, because xbps only lists what
//! is already installed.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
use kxbps_core::{
    Catalog, Category, Inst, InstalledMap, InstalledPackage, Pkg, QueueAction, QueueItem, RepoId,
    UpdateEntry, byte_decimals, format_bytes,
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
        #[cxx_name = "kxbps_model_begin_reset"]
        fn model_begin_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_model_end_reset"]
        fn model_end_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_model_create_index"]
        fn model_create_index(model: &QAbstractItemModel, row: i32, column: i32) -> QModelIndex;

        // `dataChanged` is a signal and cannot be emitted from Rust.
        #[cxx_name = "kxbps_model_data_changed"]
        fn model_data_changed(model: Pin<&mut QAbstractItemModel>, first: i32, last: i32);
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

        // Pushes a new snapshot; called by the installed backend through
        // `kxbps_package_model_set_state`.
        fn set_state(self: Pin<&mut PackageModel>, installed_json: &QString, queue_json: &QString);

        // Pushes a whole `Snapshot` (see `xbps_snapshot`) plus the queue; called
        // by the installed backend through `kxbps_package_model_set_snapshot`.
        fn set_snapshot(
            self: Pin<&mut PackageModel>,
            snapshot_json: &QString,
            queue_json: &QString,
        );
    }

    // QML never creates the model: `InstalledBackend` owns it and hands it out
    // as a `QObject*`. It starts without a snapshot — the worker thread has not
    // answered yet — so the first frame shows the catalog and the installed
    // list fills in when xbps has been read.
    impl
        cxx_qt::Constructor<
            (QString, QObjectMutPtr),
            BaseArguments = (QObjectMutPtr,),
            NewArguments = (QString,),
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
///
/// A row the catalog knows borrows its `Pkg`; a row that only xbps reports
/// has no `Pkg` at all, because inventing one would mean inventing a size, a
/// category and a home page xbps never printed.
pub struct RowView<'a> {
    /// Name and version as displayed; for an installed-only row the version is
    /// the installed one.
    pub name: &'a str,
    pub version: &'a str,
    /// The catalog entry, when the catalog knows the package.
    pub pkg: Option<&'a Pkg>,
    pub installed: Option<&'a Inst>,
    pub queued: Option<QueueAction>,
    pub update_to: Option<&'a str>,
    /// The record's own description, when it has one the catalog does not.
    pub description: Option<&'a str>,
}

impl RowView<'_> {
    /// A row the catalog knows: its own fields fill everything.
    ///
    /// `installed`, `queued` and `update_to` are the store's view of it, so a
    /// query hit and a list row answer the same roles.
    pub fn catalog<'a>(
        pkg: &'a Pkg,
        installed: Option<&'a Inst>,
        queued: Option<QueueAction>,
        update_to: Option<&'a str>,
    ) -> RowView<'a> {
        RowView {
            name: &pkg.name,
            version: if let Some(inst) = installed {
                &inst.version
            } else {
                &pkg.version
            },
            pkg: Some(pkg),
            installed,
            queued,
            update_to,
            description: None,
        }
    }

    /// Short description: the catalog's own `short` when it knows the package
    /// (the curated text), the record's description otherwise.
    fn short(&self) -> &str {
        match self.pkg {
            Some(pkg) => &pkg.short,
            None => self.description.unwrap_or(""),
        }
    }
}

/// Updates and orphans derived from one installed snapshot.
///
/// The package model and the query façade both answer `isUpdate`,
/// `updateVersion` and the orphan predicate from this value, so the two views
/// cannot disagree. Both lists come from xbps itself — `xbps-install -un` and
/// `xbps-query -O` — because xbps knows what the repositories offer and what
/// nothing needs, and the store has no second opinion to offer.
pub(crate) struct InstalledState {
    /// name -> version, for packages that have an update.
    pub(crate) update_to: HashMap<String, String>,
    /// Installed packages nothing depends on, as `xbps-query -O` reported.
    pub(crate) orphans: HashSet<String>,
}

impl InstalledState {
    pub(crate) fn new(updates: &[UpdateEntry], orphans: &[String]) -> Self {
        let update_to = updates
            .iter()
            .map(|update| (update.name.clone(), update.version.clone()))
            .collect();
        let orphans = orphans.iter().cloned().collect();
        Self { update_to, orphans }
    }

    pub(crate) fn is_update(&self, name: &str) -> bool {
        self.update_to.contains_key(name)
    }

    pub(crate) fn update_to(&self, name: &str) -> Option<&str> {
        self.update_to.get(name).map(String::as_str)
    }

    pub(crate) fn is_orphan(&self, name: &str) -> bool {
        self.orphans.contains(name)
    }
}

/// Value of `role` for one row; an invalid `QVariant` for unknown roles.
///
/// Every string the system does not report is empty rather than invented: a
/// catalog-known package keeps its repository size, download size and home
/// page, an installed-only one shows nothing there.
pub fn role_value(role: i32, row: &RowView<'_>) -> QVariant {
    let pkg = row.pkg;
    match role {
        role::NAME => text(row.name),
        role::VERSION => text(row.version),
        role::REPO => text(pkg.map_or("", |pkg| repo_key(pkg.repo))),
        role::CATEGORY => text(pkg.map_or("", |pkg| category_key(pkg.category))),
        role::SIZE => text(&pkg.map_or(String::new(), |pkg| format_size(pkg.size))),
        role::DOWNLOAD => text(&pkg.map_or(String::new(), |pkg| format_size(pkg.download))),
        role::SHORT => text(row.short()),
        role::HOMEPAGE => text(pkg.map_or("", |pkg| pkg.homepage.as_str())),
        role::LICENSE => text(pkg.map_or("", |pkg| pkg.license.as_str())),
        role::DEPENDS => text(&pkg.map_or(String::new(), |pkg| pkg.depends.join(", "))),
        role::INSTALLED_VERSION => text(row.installed.map_or("", |inst| inst.version.as_str())),
        role::EXPLICIT => boolean(row.installed.is_some_and(|inst| inst.explicit)),
        role::HELD => boolean(row.installed.is_some_and(|inst| inst.held)),
        role::QUEUED_ACTION => text(row.queued.map_or("", queue_key)),
        role::IS_UPDATE => boolean(row.update_to.is_some()),
        role::UPDATE_VERSION => text(row.update_to.unwrap_or("")),
        role::SIZE_BYTES => pkg.map_or_else(QVariant::default, |pkg| integer(pkg.size)),
        role::DOWNLOAD_BYTES => pkg.map_or_else(QVariant::default, |pkg| integer(pkg.download)),
        _ => QVariant::default(),
    }
}

/// Value for a map key: the role table plus `files`, which only the query view
/// needs and which has no role.
pub fn field_value(name: &str, row: &RowView<'_>) -> Option<QVariant> {
    if name == "files" {
        return Some(text(
            &row.pkg.map_or(String::new(), |pkg| pkg.files.join("\n")),
        ));
    }
    ROLES
        .iter()
        .find(|(_, key)| *key == name)
        .map(|(id, _)| role_value(*id, row))
}

/// One visible position: a catalog package or an installed record.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    /// Into [`PackageModelRust::catalog`].
    Catalog(u32),
    /// Into [`PackageModelRust::snapshot`].
    Installed(u32),
}

/// Rust side of [`qobject::PackageModel`].
pub struct PackageModelRust {
    catalog: Arc<Catalog>,
    /// Catalog indices ordered by package name; the `all` filter's rows.
    order: Vec<u32>,
    /// The records `xbps-query -l` answered, sorted by name; empty until the
    /// worker thread has read the system.
    snapshot: Vec<InstalledPackage>,
    /// Whether a snapshot ever arrived. Until it does, the list is the catalog.
    connected: bool,
    /// The installed set derived from `snapshot`, as the store holds it.
    installed: InstalledMap,
    /// The pending updates xbps reported, held ones included.
    updates: Vec<UpdateEntry>,
    /// The names `xbps-query -O` reported.
    orphans: Vec<String>,
    /// The visible rows, in display order.
    rows: Vec<Row>,
    queue: HashMap<String, QueueAction>,
    /// Updates, held names and orphans derived from the snapshot.
    state: InstalledState,
    held_names: HashSet<String>,
    filter: QString,
    search: QString,
    category: QString,
    count: i32,
}

impl PackageModelRust {
    /// Parses the catalog; the list shows it until the first snapshot arrives.
    ///
    /// `CatalogBackend` validates the catalog before handing it over, so a
    /// parse failure here can only mean the two halves disagree; the model
    /// then starts empty instead of panicking.
    fn new(catalog_json: &str) -> Self {
        let catalog = Catalog::load_from_str(catalog_json).unwrap_or_default();
        Self::with_catalog(Arc::new(catalog))
    }

    fn with_catalog(catalog: Arc<Catalog>) -> Self {
        let mut order: Vec<u32> = (0..catalog.len() as u32).collect();
        order.sort_by(|left, right| {
            catalog.packages()[*left as usize]
                .name
                .cmp(&catalog.packages()[*right as usize].name)
        });
        let mut model = Self {
            catalog,
            order,
            snapshot: Vec::new(),
            connected: false,
            installed: InstalledMap::new(),
            updates: Vec::new(),
            orphans: Vec::new(),
            rows: Vec::new(),
            queue: HashMap::new(),
            state: InstalledState::new(&[], &[]),
            held_names: HashSet::new(),
            // The window opens on the installed list, so the first frame says
            // what is installed rather than showing a demo catalog as "available".
            filter: QString::from("installed"),
            search: QString::from(""),
            category: QString::from("all"),
            count: 0,
        };
        model.refresh_derived();
        model.rebuild();
        model
    }

    /// Replaces the installed set, the pending updates, the orphan list and
    /// the queue in one commit.
    ///
    /// The records are the definition: `installed` must be the map derived from
    /// them, so the list order and the descriptions stay those of
    /// `xbps-query -l`. A store-only change (a queue toggle, a hold) goes
    /// through [`Self::store_state`] instead, which keeps the records as they
    /// are.
    ///
    /// Returns `(rows_changed, new_rows, affected_rows)`. When the visible set
    /// moved (the list gained or lost a row) the caller resets the model; when
    /// it did not, only the rows whose fingerprint changed are reported, so the
    /// caller can emit `dataChanged` for those instead of rebuilding every
    /// delegate.
    fn snapshot(
        &mut self,
        records: Vec<InstalledPackage>,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
        updates: Vec<UpdateEntry>,
        orphans: Vec<String>,
    ) -> (bool, Vec<Row>, Vec<i32>) {
        // The caller's records are the authority when it has them (the read
        // layer does, descriptions included); otherwise the map is turned into
        // records and every record the list already knew keeps its description.
        self.snapshot = if records.is_empty() {
            self.records_from_map(&installed)
        } else {
            records
        };
        self.snapshot
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.connected = true;
        self.updates = updates;
        self.orphans = orphans;
        self.install(installed, queue)
    }

    /// Turns an installed map into the records the list shows.
    fn records_from_map(&self, installed: &InstalledMap) -> Vec<InstalledPackage> {
        installed
            .iter()
            .map(|(name, inst)| InstalledPackage {
                name: name.clone(),
                version: inst.version.clone(),
                pkgver: format!("{name}-{}", inst.version),
                state: kxbps_core::InstallState::Installed,
                description: self
                    .snapshot
                    .iter()
                    .find(|record| &record.name == name)
                    .map_or_else(String::new, |record| record.description.clone()),
            })
            .collect()
    }

    /// Applies a store change on top of the records the last read produced.
    ///
    /// Nothing about xbps changes here — the updates and the orphans stay as
    /// read — only the installed set, its `explicit`/`held` flags and the
    /// queue move. The record list follows the map, and every record keeps the
    /// description the read printed.
    fn store_state(
        &mut self,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
    ) -> (bool, Vec<Row>, Vec<i32>) {
        self.snapshot = self.records_from_map(&installed);
        self.snapshot
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.connected = true;
        self.install(installed, queue)
    }

    fn install(
        &mut self,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
    ) -> (bool, Vec<Row>, Vec<i32>) {
        let before: Vec<(Option<Inst>, Option<QueueAction>, Option<String>)> =
            self.rows.iter().map(|row| self.fingerprint(row)).collect();

        self.installed = installed;
        self.queue = queue
            .into_iter()
            .map(|item| (item.name, item.action))
            .collect();
        self.refresh_derived();

        let rows = self.compute_rows();
        if rows != self.rows {
            return (true, rows, Vec::new());
        }
        let affected = self
            .rows
            .iter()
            .enumerate()
            .filter(|(row, candidate)| before[*row] != self.fingerprint(candidate))
            .map(|(row, _)| i32::try_from(row).unwrap_or(i32::MAX))
            .collect();
        (false, rows, affected)
    }

    fn row_at(&self, row: i32) -> Option<&Row> {
        let index = usize::try_from(row).ok()?;
        self.rows.get(index)
    }

    /// The name a row stands for.
    fn name_of(&self, row: &Row) -> &str {
        match row {
            Row::Catalog(index) => &self.catalog.packages()[*index as usize].name,
            Row::Installed(index) => &self.snapshot[*index as usize].name,
        }
    }

    /// The catalog entry of a row, when the catalog knows the name.
    fn pkg_of(&self, row: &Row) -> Option<&Pkg> {
        match row {
            Row::Catalog(index) => self.catalog.packages().get(*index as usize),
            Row::Installed(index) => {
                let record = self.snapshot.get(*index as usize)?;
                self.catalog.by_name(&record.name)
            }
        }
    }

    fn row_view<'a>(&'a self, row: &'a Row) -> RowView<'a> {
        let name = self.name_of(row);
        let pkg = self.pkg_of(row);
        let installed = self.installed.get(name);
        let record = match row {
            Row::Installed(index) => self.snapshot.get(*index as usize),
            Row::Catalog(_) => None,
        };
        let version = installed
            .map(|inst| inst.version.as_str())
            .or_else(|| pkg.map(|pkg| pkg.version.as_str()))
            .or_else(|| record.map(|record| record.version.as_str()))
            .unwrap_or("");
        RowView {
            name,
            version,
            pkg,
            installed,
            queued: self.queue.get(name).copied(),
            update_to: self.state.update_to(name),
            description: record.map(|record| record.description.as_str()),
        }
    }

    /// Recomputes the state derived from the installed snapshot.
    ///
    /// See [`InstalledState`]: the same value backs the query façade, so
    /// `isUpdate` cannot mean two different things in the app. A held package
    /// keeps its place in `updates` for the "held updates" note but never shows
    /// in the Updates filter, because a hold toggled in this window must hide
    /// its row immediately.
    fn refresh_derived(&mut self) {
        let held: HashSet<String> = self
            .installed
            .iter()
            .filter(|(_, inst)| inst.held)
            .map(|(name, _)| name.clone())
            .collect();
        self.state = InstalledState::new(&self.updates, &self.orphans);
        self.held_names = held;
    }

    /// Visible rows for the current filter, search and category.
    ///
    /// Built into a fresh vector so a caller can compare before committing: a
    /// keystroke in the search field that does not change the visible set must
    /// not reset the model, or QML loses the scroll position and selection.
    fn compute_rows(&self) -> Vec<Row> {
        let mut candidates: Vec<Row> = if self.connected {
            (0..self.snapshot.len() as u32)
                .map(Row::Installed)
                .collect()
        } else {
            self.order.iter().copied().map(Row::Catalog).collect()
        };
        // `xbps-query -l` prints in name order already; sorting here too keeps
        // the list stable when a record list arrives from another source.
        candidates.sort_by(|left, right| self.name_of(left).cmp(self.name_of(right)));
        let filter = self.filter.to_string();
        let category = self.category.to_string();
        let search = self.search.to_string().to_lowercase();
        candidates
            .into_iter()
            .filter(|row| self.matches(row, &filter, &category, &search))
            .collect()
    }

    /// Commits `rows` as the visible set and keeps `count` in step.
    fn assign_rows(&mut self, rows: Vec<Row>) {
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

    /// Everything the installed/queue snapshot feeds one row.
    fn fingerprint(&self, row: &Row) -> (Option<Inst>, Option<QueueAction>, Option<String>) {
        let name = self.name_of(row);
        (
            self.installed.get(name).cloned(),
            self.queue.get(name).copied(),
            self.state.update_to(name).map(str::to_string),
        )
    }

    /// All visible names, for the tests.
    #[cfg(test)]
    fn visible_names(&self) -> Vec<&str> {
        self.rows.iter().map(|row| self.name_of(row)).collect()
    }

    fn matches(&self, row: &Row, filter: &str, category: &str, search: &str) -> bool {
        let pkg = self.pkg_of(row);
        if category != "all" && pkg.is_none_or(|pkg| category_key(pkg.category) != category) {
            return false;
        }
        let name = self.name_of(row);
        if !search.is_empty()
            && !name.to_lowercase().contains(search)
            && !self.short_of(row).to_lowercase().contains(search)
        {
            return false;
        }
        let installed = self.installed.get(name);
        match filter {
            // Every row the list has. Before the first read that is the
            // catalog; afterwards it is the system's packages, because xbps has
            // no command that lists what is *not* installed.
            "all" => true,
            "installed" => installed.is_some(),
            "updates" => self.state.is_update(name) && !self.held_names.contains(name),
            "explicit" => installed.is_some_and(|inst| inst.explicit),
            "orphans" => self.state.is_orphan(name),
            "held" => installed.is_some_and(|inst| inst.held),
            "queued" => self.queue.contains_key(name),
            _ => true,
        }
    }

    /// The searchable short text of a row.
    fn short_of(&self, row: &Row) -> &str {
        let name = self.name_of(row);
        match self.catalog.by_name(name) {
            Some(pkg) => &pkg.short,
            None => self
                .snapshot
                .get(match row {
                    Row::Installed(index) => *index as usize,
                    Row::Catalog(_) => 0,
                })
                .map_or("", |record| record.description.as_str()),
        }
    }
}

impl cxx_qt::Constructor<(QString, QObjectMutPtr)> for qobject::PackageModel {
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
        let (catalog_json, parent) = arguments;
        ((catalog_json,), (parent,), ())
    }

    fn new((catalog_json,): Self::NewArguments) -> PackageModelRust {
        PackageModelRust::new(&catalog_json.to_string())
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
        let Some(row) = self.row_at(index.row()) else {
            return cxx_qt_lib::QVariant::default();
        };
        role_value(role, &self.row_view(row))
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

    /// Replaces the whole state from the backend, with the snapshot the read
    /// layer took.
    ///
    /// `snapshot_json` is a [`crate::xbps_snapshot::Snapshot`]: the installed
    /// records, the pending updates, the explicit/held/orphan names. The model
    /// derives the installed map from it, so the two cannot disagree about
    /// `explicit` or `held`. Called by the backend through
    /// `kxbps_package_model_set_snapshot`; not invokable from QML.
    fn set_snapshot(
        mut self: Pin<&mut Self>,
        snapshot_json: &cxx_qt_lib::QString,
        queue_json: &cxx_qt_lib::QString,
    ) {
        let snapshot: crate::xbps_snapshot::Snapshot =
            serde_json::from_str(&snapshot_json.to_string()).unwrap_or_default();
        let installed = kxbps_core::installed_map(
            &snapshot.installed,
            &snapshot.explicit,
            &snapshot.held,
            &|_| 0,
        );
        let queue: Vec<QueueItem> =
            serde_json::from_str(&queue_json.to_string()).unwrap_or_default();
        let (rows_changed, rows, affected) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.snapshot(
                snapshot.installed,
                installed,
                queue,
                snapshot.updates,
                snapshot.orphans,
            )
        };
        commit(self.as_mut(), rows_changed, rows, affected);
    }

    /// Replaces the installed and queue snapshots after a store mutation.
    ///
    /// The snapshot list, the updates and the orphan list are kept: they come
    /// from xbps, and a store mutation (queueing, a hold toggle, a finished job)
    /// does not change what the repositories offer or what is installed. A
    /// visible-set change resets the model (a filter gains or loses a row);
    /// otherwise only the affected rows get `dataChanged`, so selection, scroll
    /// position and the other delegates survive a transaction.
    fn set_state(
        mut self: Pin<&mut Self>,
        installed_json: &cxx_qt_lib::QString,
        queue_json: &cxx_qt_lib::QString,
    ) {
        let installed: InstalledMap =
            serde_json::from_str(&installed_json.to_string()).unwrap_or_default();
        let queue: Vec<QueueItem> =
            serde_json::from_str(&queue_json.to_string()).unwrap_or_default();
        let (rows_changed, rows, affected) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.store_state(installed, queue)
        };
        commit(self.as_mut(), rows_changed, rows, affected);
    }
}

/// Commits the result of a snapshot change and notifies QML.
fn commit(
    mut model: Pin<&mut qobject::PackageModel>,
    rows_changed: bool,
    rows: Vec<Row>,
    affected: Vec<i32>,
) {
    if rows_changed {
        qobject::model_begin_reset(base(model.as_mut()));
        {
            let data = Pin::get_mut(model.as_mut().rust_mut());
            data.assign_rows(rows);
        }
        qobject::model_end_reset(base(model.as_mut()));
        model.as_mut().count_changed();
        return;
    }
    for (first, last) in ranges(&affected) {
        qobject::model_data_changed(base(model.as_mut()), first, last);
    }
}

/// Groups ascending row ids into inclusive `[first, last]` ranges.
fn ranges(rows: &[i32]) -> Vec<(i32, i32)> {
    let mut ranges: Vec<(i32, i32)> = Vec::new();
    for &row in rows {
        match ranges.last_mut() {
            Some((_, last)) if *last + 1 == row => *last = row,
            _ => ranges.push((row, row)),
        }
    }
    ranges
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
pub(crate) fn format_size(bytes: u64) -> String {
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

/// Wire key of a queue action, the same spelling the store persists.
pub(crate) fn queue_key(action: QueueAction) -> &'static str {
    match action {
        QueueAction::Install => "install",
        QueueAction::Remove => "remove",
        QueueAction::Update => "update",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::{InstallState, ListFilter, Store};

    fn catalog() -> Arc<Catalog> {
        Arc::new(Catalog::load_from_str(include_str!("../resources/catalog.json")).unwrap())
    }

    /// One record as `xbps-query -l` printed it.
    fn record(pkgver: &str, description: &str) -> InstalledPackage {
        let (name, version) = kxbps_core::split_pkgver(pkgver).expect("pkgver splits");
        InstalledPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            state: InstallState::Installed,
            description: description.to_owned(),
        }
    }

    /// One update as `xbps-install -un` printed it.
    fn update(pkgver: &str) -> UpdateEntry {
        let (name, version) = kxbps_core::split_pkgver(pkgver).expect("pkgver splits");
        UpdateEntry {
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: pkgver.to_owned(),
            action: kxbps_core::XbpsAction::Update,
            arch: "x86_64".to_owned(),
            repository: "https://repo.example/current".to_owned(),
            installed_size: 1,
            download_size: 1,
        }
    }

    fn model_with(installed: InstalledMap) -> PackageModelRust {
        let mut model = PackageModelRust::with_catalog(catalog());
        install_records(&mut model, &installed, Vec::new(), Vec::new(), Vec::new());
        model
    }

    /// Feeds a snapshot the way the backend does: records for the list, the
    /// map derived from them, then the lists xbps reported.
    fn install(
        model: &mut PackageModelRust,
        installed: &InstalledMap,
        updates: Vec<UpdateEntry>,
        orphans: Vec<String>,
    ) {
        install_records(model, installed, Vec::new(), updates, orphans);
    }

    fn install_records(
        model: &mut PackageModelRust,
        installed: &InstalledMap,
        records: Vec<InstalledPackage>,
        updates: Vec<UpdateEntry>,
        orphans: Vec<String>,
    ) {
        let (_, rows, _) = model.snapshot(records, installed.clone(), Vec::new(), updates, orphans);
        model.assign_rows(rows);
    }

    fn set_filter(model: &mut PackageModelRust, filter: &str) {
        model.filter = QString::from(filter);
        model.rebuild();
    }

    #[test]
    fn catalogue_fills_the_first_frame_before_any_read() {
        // Until the worker answers, the list is the catalog: an empty window
        // would say "nothing is installed" about a system nobody has looked at.
        let model = PackageModelRust::with_catalog(catalog());
        assert_eq!(
            model.filter.to_string(),
            "installed",
            "the window opens on Installed"
        );
        let mut model = model;
        assert_eq!(model.count, 0, "nothing is known to be installed yet");
        set_filter(&mut model, "all");
        assert_eq!(model.count, 115);
        assert_eq!(model.visible_names().len(), 115);
        let mut sorted = model.visible_names();
        sorted.sort_unstable();
        assert_eq!(model.visible_names(), sorted, "rows are in name order");
    }

    #[test]
    fn a_snapshot_replaces_the_catalog_rows_with_the_system_records() {
        let mut model = PackageModelRust::with_catalog(catalog());
        let installed: InstalledMap = [("zramen", ("0.2.0_1", true)), ("ark", ("25.08.0_1", true))]
            .into_iter()
            .map(|(name, (version, explicit))| {
                (
                    name.to_owned(),
                    Inst {
                        version: version.to_owned(),
                        explicit,
                        held: false,
                        at: 0,
                    },
                )
            })
            .collect();
        let records = vec![
            record("ark-25.08.0_1", "Archive manager"),
            record("zramen-0.2.0_1", "Zram controller"),
        ];

        let (rows_changed, rows, _) =
            model.snapshot(records, installed, Vec::new(), Vec::new(), Vec::new());
        assert!(rows_changed, "two records are not 115 catalog rows");
        model.assign_rows(rows);

        assert_eq!(model.count, 2);
        assert_eq!(model.visible_names(), vec!["ark", "zramen"]);
        assert_eq!(
            model.name_of(&model.rows[1]),
            "zramen",
            "a name the catalog does not know is still a row"
        );
    }

    #[test]
    fn an_installed_only_row_shows_what_the_catalog_does_not_know_as_empty() {
        let mut model = PackageModelRust::with_catalog(catalog());
        let installed: InstalledMap = [(
            "zramen".to_owned(),
            Inst {
                version: "0.2.0_1".to_owned(),
                explicit: true,
                held: false,
                at: 0,
            },
        )]
        .into_iter()
        .collect();
        install_records(
            &mut model,
            &installed,
            vec![record("zramen-0.2.0_1", "Zram controller")],
            Vec::new(),
            Vec::new(),
        );

        let row = model.row_view(&model.rows[0]);
        assert_eq!(
            role_value(role::NAME, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "zramen"
        );
        assert_eq!(
            role_value(role::VERSION, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "0.2.0_1",
            "the installed version is the only one there is"
        );
        assert_eq!(
            role_value(role::SHORT, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "Zram controller",
            "the record's description fills the short text"
        );
        for role_id in [role::SIZE, role::REPO, role::CATEGORY, role::HOMEPAGE] {
            assert!(
                role_value(role_id, &row)
                    .value::<QString>()
                    .unwrap()
                    .to_string()
                    .is_empty(),
                "role {role_id} is empty rather than invented"
            );
        }
        assert!(
            role_value(role::SIZE_BYTES, &row).value::<i64>().is_none(),
            "no size in bytes either"
        );
        assert!(role_value(role::EXPLICIT, &row).value::<bool>().unwrap());
    }

    #[test]
    fn a_catalog_known_record_keeps_the_catalog_facts() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        set_filter(&mut model, "installed");

        let row = model
            .rows
            .iter()
            .position(|row| model.name_of(row) == "vim")
            .map(|index| model.row_view(&model.rows[index]))
            .expect("vim is installed");
        assert_eq!(
            role_value(role::SIZE, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "4.0 MiB"
        );
        assert_eq!(
            role_value(role::SIZE_BYTES, &row).value::<i64>().unwrap(),
            4_194_304
        );
        assert_eq!(
            role_value(role::REPO, &row)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "current"
        );
        assert!(!role_value(role::IS_UPDATE, &row).value::<bool>().unwrap());
    }

    #[test]
    fn installed_filter_lists_the_records_not_the_catalog_intersection() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        assert_eq!(model.rows.len(), 56, "the snapshot has 56 records");
        set_filter(&mut model, "installed");
        assert_eq!(model.count, 56);
        assert!(model.visible_names().contains(&"vim"));
        assert!(
            !model.visible_names().contains(&"ark"),
            "ark is not installed"
        );
    }

    #[test]
    fn updates_filter_follows_the_reported_list() {
        let store = Store::fresh(&catalog());
        let updates = vec![update("linux-6.18.63_1"), update("curl-8.16.0_1")];
        let mut model = model_with(store.installed.clone());
        install(&mut model, &store.installed, updates, Vec::new());
        set_filter(&mut model, "updates");
        assert_eq!(model.visible_names(), vec!["curl", "linux"]);
    }

    #[test]
    fn a_hold_hides_its_update_immediately() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        install(
            &mut model,
            &store.installed,
            vec![update("linux-6.18.63_1")],
            Vec::new(),
        );
        set_filter(&mut model, "updates");
        assert_eq!(model.visible_names(), vec!["linux"]);

        // The hold lives in the store, not in the reported list: the model must
        // hide the row anyway.
        let mut installed = store.installed.clone();
        installed.get_mut("linux").unwrap().held = true;
        let (_, rows, _) = model.store_state(installed, Vec::new());
        model.assign_rows(rows);
        set_filter(&mut model, "updates");
        assert!(
            model.visible_names().is_empty(),
            "held packages are not updates"
        );
    }

    #[test]
    fn orphans_filter_follows_the_reported_list() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        install(
            &mut model,
            &store.installed,
            Vec::new(),
            vec!["cowsay".to_owned()],
        );
        set_filter(&mut model, "orphans");
        assert_eq!(model.visible_names(), vec!["cowsay"]);
    }

    #[test]
    fn explicit_and_orphan_filters_are_disjoint_for_the_seed() {
        let store = Store::fresh(&catalog());
        let mut explicit = model_with(store.installed.clone());
        set_filter(&mut explicit, "explicit");
        let explicit_names: HashSet<String> = explicit
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert!(!explicit_names.is_empty());
        assert!(explicit_names.contains("vim"));

        let mut orphans = model_with(store.installed.clone());
        install(
            &mut orphans,
            &store.installed,
            Vec::new(),
            vec!["cowsay".to_owned()],
        );
        set_filter(&mut orphans, "orphans");
        let orphan_names: HashSet<String> = orphans
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert!(orphan_names.is_disjoint(&explicit_names));
    }

    #[test]
    fn queued_filter_follows_the_queue_map() {
        let mut model = PackageModelRust::with_catalog(catalog());
        set_filter(&mut model, "all");
        set_filter(&mut model, "queued");
        assert_eq!(model.rows.len(), 0, "nothing is queued");

        model.queue.insert("vim".to_string(), QueueAction::Install);
        model.rebuild();
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn rebuild_reports_no_change_when_the_visible_set_is_identical() {
        let mut model = PackageModelRust::with_catalog(catalog());
        set_filter(&mut model, "all");
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
    fn search_needle_is_case_insensitive() {
        let mut model = PackageModelRust::with_catalog(catalog());
        set_filter(&mut model, "all");
        model.search = QString::from("Bash");
        model.rebuild();
        assert!(!model.rows.is_empty(), "Bash matches the catalog");
        assert!(model.visible_names().contains(&"bash"));
    }

    #[test]
    fn category_narrows_without_touching_the_catalog() {
        let mut model = PackageModelRust::with_catalog(catalog());
        set_filter(&mut model, "all");
        let catalog_ptr = Arc::as_ptr(&model.catalog);
        model.category = QString::from("devel");
        model.rebuild();
        assert!(!model.rows.is_empty());
        assert!(model.rows.iter().all(|row| {
            model
                .pkg_of(row)
                .is_some_and(|pkg| pkg.category == Category::Devel)
        }));
        assert_eq!(
            Arc::as_ptr(&model.catalog),
            catalog_ptr,
            "catalog not rebuilt"
        );
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
            let mut model = PackageModelRust::with_catalog(catalog());
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
        let mut model = model_with(store.installed.clone());
        let row = model
            .rows
            .iter()
            .position(|row| model.name_of(row) == "vim")
            .expect("vim is installed");
        let view = model.row_view(&model.rows[row]);
        assert!(role_value(role::EXPLICIT, &view).value::<bool>().unwrap());
        assert!(
            role_value(role::QUEUED_ACTION, &view)
                .value::<QString>()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            role_value(role::DOWNLOAD_BYTES, &view)
                .value::<i64>()
                .unwrap(),
            1_468_006
        );
        assert!(
            role_value(role::DOWNLOAD_BYTES + 1, &view)
                .type_name()
                .is_empty(),
            "unknown role"
        );
        let _ = &mut model;
    }

    /// The seed's installed set plus `ark`: a store change after a read, so
    /// the tests that use it go through `store_state`.
    fn installed_with_ark() -> InstalledMap {
        let mut installed = Store::fresh(&catalog()).installed;
        installed.insert(
            "ark".to_string(),
            Inst {
                version: "25.08.0_1".into(),
                explicit: true,
                held: false,
                at: 0,
            },
        );
        installed
    }

    #[test]
    fn a_store_change_reports_only_the_rows_that_moved() {
        // A finished job, a queue toggle: the store gains a package, the list
        // gains one row and nothing else is rebuilt.
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        set_filter(&mut model, "installed");
        assert_eq!(model.count, 56);

        let (rows_changed, rows, affected) = model.store_state(installed_with_ark(), Vec::new());
        assert!(rows_changed, "the installed filter gained a row");
        assert_eq!(rows.len(), 57);
        assert!(affected.is_empty(), "a reset does not need dataChanged");
    }

    #[test]
    fn a_queue_change_only_reports_the_queued_row() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        set_filter(&mut model, "installed");
        let before = model.rows.clone();

        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];
        let (rows_changed, rows, affected) = model.store_state(store.installed.clone(), queue);
        assert!(!rows_changed, "the same records, so no reset");
        assert!(
            affected
                .iter()
                .all(|row| model.name_of(&before[*row as usize]) == "vim"),
            "only the queued row is reported"
        );
        assert_eq!(rows, before, "the visible set is unchanged");
    }

    #[test]
    fn snapshot_resets_when_a_queue_row_appears() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        set_filter(&mut model, "installed");
        assert_eq!(model.count, 56);

        let (rows_changed, rows, affected) = model.store_state(installed_with_ark(), Vec::new());
        assert!(rows_changed, "the installed filter gained a record");
        assert_eq!(rows.len(), 57);
        assert!(affected.is_empty(), "a reset does not need dataChanged");
    }

    #[test]
    fn queue_snapshot_marks_the_queued_row() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        set_filter(&mut model, "all");
        let before = model.rows.clone();
        assert_eq!(before.len(), 56, "the seed's records, in the `all` filter");

        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];

        let (rows_changed, _, affected) = model.store_state(store.installed.clone(), queue);
        assert!(!rows_changed, "the list itself does not move");
        assert_eq!(affected.len(), 1, "only vim changed");
        assert_eq!(model.name_of(&model.rows[affected[0] as usize]), "vim");
    }

    #[test]
    fn queue_snapshot_drives_the_queued_filter_and_role() {
        let store = Store::fresh(&catalog());
        let mut model = model_with(store.installed.clone());
        model.queue.clear();
        model.rebuild();
        set_filter(&mut model, "queued");
        assert_eq!(model.rows.len(), 0);

        // The store's own wire format: `[{name, action}]`, not a map. The
        // queue can only hold a row the list has, so this queues `vim`.
        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];
        let (rows_changed, rows, _) = model.store_state(store.installed.clone(), queue);
        assert!(rows_changed, "the queued filter gains a row");
        assert_eq!(rows.len(), 1);
        model.assign_rows(rows);
        assert_eq!(model.visible_names(), vec!["vim"]);

        let view = model.row_view(&model.rows[0]);
        assert_eq!(
            role_value(role::QUEUED_ACTION, &view)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "install"
        );
    }

    #[test]
    fn ranges_group_consecutive_rows() {
        assert_eq!(ranges(&[]), Vec::new());
        assert_eq!(ranges(&[3]), vec![(3, 3)]);
        assert_eq!(ranges(&[1, 2, 3, 7, 9, 10]), vec![(1, 3), (7, 7), (9, 10)]);
    }
}
