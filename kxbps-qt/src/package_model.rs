//! Read-only `QAbstractItemModel` over the installed records xbps reports.
//!
//! The model holds the records `xbps-query -l` answered plus the installed map
//! derived from them, and a `rows` vector of indices into the record list.
//! Changing the filter or the search text only recomputes `rows` and emits a
//! full model reset, so no record is cloned when the user clicks a chip.
//!
//! Nothing is invented: a field xbps did not print stays empty. The repository,
//! category and size columns of the old catalog are gone with the catalog; the
//! details pane reads those from `xbps-query -p` instead.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QObjectMutPtr, QString, QStringList, QVariant};
use kxbps_core::{
    Inst, InstalledMap, InstalledPackage, QueueAction, QueueItem, UpdateEntry, byte_decimals,
    format_bytes,
};
use qobject::QHash_i32_QByteArray;

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

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;

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
        // `name` (default) or `size`; the key `compute_rows` sorts every
        // candidate by before filtering. `sortDescending` is its direction.
        #[qproperty(QString, sort, READ, WRITE = set_sort, NOTIFY)]
        #[qproperty(bool, sort_descending, READ, WRITE = set_sort_descending, NOTIFY)]
        #[qproperty(i32, count, READ, NOTIFY)]
        // Whether the list is showing the ranked repository answer of a
        // non-empty search field rather than the filter's own rows.
        #[qproperty(bool, searching, READ, NOTIFY)]
        // Whether a repository read the list asked for is still in flight. The
        // Available page uses it to say it is reading the list instead of
        // printing the installed count under a repository heading.
        #[qproperty(bool, search_loading, READ, NOTIFY)]
        // The failure the newest search read reported: an i18n key and its
        // `%1…%3` arguments, empty when the read succeeded. A reader that does
        // not know the key shows nothing rather than the raw text.
        #[qproperty(QString, search_error, READ, NOTIFY)]
        #[qproperty(QStringList, search_error_args, READ, NOTIFY)]
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
        fn set_sort(self: Pin<&mut PackageModel>, value: QString);
        fn set_sort_descending(self: Pin<&mut PackageModel>, value: bool);

        /// A search asked for the repository hits of `pattern`.
        ///
        /// The list cannot read xbps itself: it holds no transport. Whoever
        /// owns it answers through `setSearchResults`. The empty pattern is the
        /// whole repository listing, which is what the Available page shows
        /// when its search box is empty.
        #[qsignal]
        fn search_requested(self: Pin<&mut PackageModel>, pattern: QString);

        /// Applies the repository answer for the current pattern: the ranked
        /// hits as JSON, plus the failure the read reported (a key and its
        /// arguments), empty when it succeeded.
        ///
        /// Called by the installed backend with what `read_list_search`
        /// returned; not invokable from QML.
        fn set_search_results(
            self: Pin<&mut PackageModel>,
            hits_json: &QString,
            error_key: &QString,
            error_args: &QStringList,
        );

        // Pushes a whole `Snapshot` (see `xbps_snapshot`) plus the queue; called
        // by the installed backend through `kxbps_package_model_set_snapshot`.
        fn set_snapshot(
            self: Pin<&mut PackageModel>,
            snapshot_json: &QString,
            queue_json: &QString,
        );

        // Pushes the installed map and the queue after a store mutation; the
        // records, updates and orphans xbps reported are kept.
        fn set_state(self: Pin<&mut PackageModel>, installed_json: &QString, queue_json: &QString);
    }

    // QML never creates the model: `InstalledBackend` owns it and hands it out
    // as a `QObject*`. It starts empty and fills in when xbps has been read.
    impl cxx_qt::Constructor<(QObjectMutPtr,), BaseArguments = (QObjectMutPtr,), NewArguments = ()>
        for PackageModel
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
    pub const SHORT: i32 = USER_ROLE + 2;
    pub const INSTALLED_VERSION: i32 = USER_ROLE + 3;
    pub const EXPLICIT: i32 = USER_ROLE + 4;
    pub const HELD: i32 = USER_ROLE + 5;
    pub const QUEUED_ACTION: i32 = USER_ROLE + 6;
    pub const IS_UPDATE: i32 = USER_ROLE + 7;
    pub const UPDATE_VERSION: i32 = USER_ROLE + 8;
    pub const SIZE: i32 = USER_ROLE + 9;
    pub const SIZE_BYTES: i32 = USER_ROLE + 10;
    pub const UPDATE_SIZE: i32 = USER_ROLE + 11;
    pub const UPDATE_SIZE_BYTES: i32 = USER_ROLE + 12;
    pub const DOWNLOAD: i32 = USER_ROLE + 13;
    pub const DOWNLOAD_BYTES: i32 = USER_ROLE + 14;
    /// The `name-version_revision` xbps printed, for the details pane and the
    /// selection tuple. A snapshot row derives it from its record, so it is
    /// not in [`ROLES`]: only a search row carries it as data.
    pub const PKGVER: i32 = USER_ROLE + 15;
}

/// Role ids and the names QML sees, in declaration order.
///
/// `data()` and the `QVariantMap`s built by `QueryBackend` share this table, so
/// a delegate reading `model.installedVersion` and a query result reading
/// `result.installedVersion` always agree.
///
/// The `*Bytes` roles carry the sortable value and an invalid `QVariant` when
/// the size is unknown; the display roles carry the humanised text or an empty
/// string. A size the read did not report is never a zero.
pub const ROLES: [(i32, &str); 15] = [
    (role::NAME, "name"),
    (role::VERSION, "version"),
    (role::SHORT, "short"),
    (role::INSTALLED_VERSION, "installedVersion"),
    (role::EXPLICIT, "explicit"),
    (role::HELD, "held"),
    (role::QUEUED_ACTION, "queuedAction"),
    (role::IS_UPDATE, "isUpdate"),
    (role::UPDATE_VERSION, "updateVersion"),
    (role::SIZE, "size"),
    (role::SIZE_BYTES, "sizeBytes"),
    (role::UPDATE_SIZE, "updateSize"),
    (role::UPDATE_SIZE_BYTES, "updateSizeBytes"),
    (role::DOWNLOAD, "download"),
    (role::DOWNLOAD_BYTES, "downloadBytes"),
];

/// Everything one row needs beyond the record itself.
pub struct RowView<'a> {
    /// Name and version as displayed; the version is the installed one.
    pub name: &'a str,
    pub version: &'a str,
    pub installed: Option<&'a Inst>,
    pub queued: Option<QueueAction>,
    pub update_to: Option<&'a str>,
    /// The record's own description.
    pub description: Option<&'a str>,
    /// The installed size of the installed version, if the read reported one.
    pub size: Option<u64>,
    /// The unpacked size of the version an update would install: the exact
    /// `xbps-install -un` value, otherwise the repository's rounded one.
    pub update_size: Option<u64>,
    /// The archive a queued install or update would download: the exact
    /// `-un` value for an update, otherwise the repository's `filename-size`.
    pub download: Option<u64>,
}

impl RowView<'_> {
    /// The size the list shows: the new version's when the row has an update,
    /// the installed one otherwise.
    pub fn displayed_size(&self) -> Option<u64> {
        self.update_size.or(self.size)
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

/// Whether an empty search box on `filter` means the whole repository listing.
///
/// Available is the one page that reads the repositories with no pattern: the
/// user asked to see what can be installed, not what is installed. Every other
/// filter narrows the installed set and shows it while the box is empty.
fn wants_repository_listing(filter: &str) -> bool {
    filter == "all"
}

/// The read a page asks for when the user enters it, if any.
///
/// The answer is the pattern to send: `Some("")` is the whole repository
/// listing, which is what Available asks for on every entry so the page never
/// depends on an answer the backend may or may not still hold. Every other
/// filter narrows the installed set and reads nothing while the box is empty;
/// a typed pattern is that field's own read, not a filter's.
pub(crate) fn listing_read_on_filter(filter: &str, search: &str) -> Option<String> {
    (wants_repository_listing(filter) && search.trim().is_empty()).then(String::new)
}

/// Whether an answer belongs on the page the user is looking at now.
///
/// A typed pattern keeps its answer whatever the filter: the box is one box for
/// packages. An empty pattern is only an answer where the page asked for the
/// listing — anywhere else it describes a page the user already left. Dropping
/// it must still settle the loading mark, or the next page waits forever.
pub(crate) fn accepts_search_answer(filter: &str, search: &str) -> bool {
    !search.trim().is_empty() || wants_repository_listing(filter)
}

/// Whether one repository hit takes a row while the list filters by `filter`.
///
/// The filter narrows the installed set and a search ranks what the repositories
/// have, so a hit the filter would not show must not take a row: with the
/// "Installed" filter active, typing must not surface repository hits that are
/// not installed, install action included. An unknown filter keeps every hit.
fn search_hit_visible(
    filter: &str,
    installed: Option<&Inst>,
    update_to: Option<&str>,
    queued: Option<QueueAction>,
    orphan: bool,
) -> bool {
    let installed_here = installed.is_some();
    let held = installed.is_some_and(|inst| inst.held);
    match filter {
        // The Available page reads the repositories, so its filter keeps every
        // hit: there is no installed set to narrow.
        "all" => true,
        "installed" => installed_here,
        "updates" => installed_here && update_to.is_some() && !held,
        "explicit" => installed.is_some_and(|inst| inst.explicit),
        "held" => held,
        "orphans" => installed_here && orphan,
        "queued" => queued.is_some(),
        _ => true,
    }
}

/// Value of `role` for one row; an invalid `QVariant` for unknown roles.
pub fn role_value(role: i32, row: &RowView<'_>) -> QVariant {
    match role {
        role::NAME => text(row.name),
        role::VERSION => text(row.version),
        role::SHORT => text(row.description.unwrap_or("")),
        role::INSTALLED_VERSION => text(row.installed.map_or("", |inst| inst.version.as_str())),
        role::EXPLICIT => boolean(row.installed.is_some_and(|inst| inst.explicit)),
        role::HELD => boolean(row.installed.is_some_and(|inst| inst.held)),
        role::QUEUED_ACTION => text(row.queued.map_or("", queue_key)),
        role::IS_UPDATE => boolean(row.update_to.is_some()),
        role::UPDATE_VERSION => text(row.update_to.unwrap_or("")),
        role::SIZE => size_text(row.size),
        role::SIZE_BYTES => size_bytes(row.size),
        role::UPDATE_SIZE => size_text(row.update_size),
        role::UPDATE_SIZE_BYTES => size_bytes(row.update_size),
        role::DOWNLOAD => size_text(row.download),
        role::DOWNLOAD_BYTES => size_bytes(row.download),
        _ => QVariant::default(),
    }
}

/// Value for a map key, using the shared role table.
pub fn field_value(name: &str, row: &RowView<'_>) -> Option<QVariant> {
    ROLES
        .iter()
        .find(|(_, key)| *key == name)
        .map(|(id, _)| role_value(*id, row))
}

/// One visible row as the tests compare two pushes: the name plus the two
/// columns a store mutation can move.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleRow {
    pub name: String,
    pub queued: Option<QueueAction>,
    pub update_to: Option<String>,
}

/// Rust side of [`qobject::PackageModel`].
pub struct PackageModelRust {
    /// The records `xbps-query -l` answered, sorted by name; empty until the
    /// worker thread has read the system.
    snapshot: Vec<InstalledPackage>,
    /// The installed set derived from `snapshot`, as the store holds it.
    installed: InstalledMap,
    /// The pending updates xbps reported, held ones included.
    updates: Vec<UpdateEntry>,
    /// The names `xbps-query -O` reported.
    orphans: Vec<String>,
    /// Indices into `snapshot`, in display order.
    rows: Vec<u32>,
    /// The lowercase name of each record, indexed like `snapshot`: the search
    /// runs over these instead of lowercasing 1340 names per keystroke.
    name_lower: Vec<String>,
    /// The lowercase description of each record, same indexing.
    description_lower: Vec<String>,
    queue: HashMap<String, QueueAction>,
    /// Updates, held names and orphans derived from the snapshot.
    state: InstalledState,
    held_names: HashSet<String>,
    /// name -> bytes, from the installed size take of the last read.
    installed_sizes: HashMap<String, u64>,
    /// name -> bytes, from the repository unpacked-size take.
    repo_installed_sizes: HashMap<String, u64>,
    /// name -> bytes, from the repository archive-size take.
    repo_download_sizes: HashMap<String, u64>,
    /// name -> exact sizes of a pending update, from `xbps-install -un`.
    update_sizes: HashMap<String, (u64, u64)>,
    filter: QString,
    search: QString,
    sort: QString,
    sort_descending: bool,
    count: i32,
    /// Whether the list shows the repository answer of a non-empty search
    /// field. Mirrors `active_search.is_some()`, kept as a field because a
    /// `#[qproperty]` reads it directly.
    searching: bool,
    /// Whether a repository read the list asked for is still in flight. The
    /// header says so instead of counting rows that are about to be replaced.
    search_loading: bool,
    /// The failure the newest search read reported, as a key and its
    /// arguments; both empty on success.
    search_error: QString,
    search_error_args: QStringList,
    /// The repository answer the list is showing, when the search field holds a
    /// pattern. `None` means the list shows the filter's own rows.
    ///
    /// A filter narrows the installed set; a search asks xbps a different
    /// question — what the repositories have — so the two cannot share one
    /// row list. While this is `Some`, `rows` is empty and the search rows are
    /// what `data()` answers from.
    active_search: Option<ActiveSearch>,
}

/// The ranked repository answer the list is showing.
struct ActiveSearch {
    /// The pattern the rows were read for, trimmed.
    pattern: String,
    /// The hits in the order `read_list_search` returned them: ranked, exact
    /// name first.
    hits: Vec<SearchRow>,
    /// The failure the read reported, if it failed; a failed search has no
    /// rows and still keeps the pattern, so the field is not silently emptied.
    /// The arguments of its sentence live in the `searchErrorArgs` property.
    error_key: String,
}

/// One repository hit as a row.
struct SearchRow {
    name: String,
    version: String,
    pkgver: String,
    description: String,
    /// The installed record the hit's own tag implies: `Some` when xbps
    /// printed `[*]`, so the row reports an install even for a package the
    /// store has not read yet.
    tagged: Option<Inst>,
}

impl SearchRow {
    fn of(hit: &kxbps_core::SearchHit) -> Self {
        Self {
            name: hit.name.clone(),
            version: hit.version.clone(),
            pkgver: hit.pkgver.clone(),
            description: hit.description.clone(),
            tagged: hit.installed.then(|| Inst {
                version: hit.version.clone(),
                explicit: false,
                held: false,
                at: 0,
            }),
        }
    }

    /// The `RowView` this hit renders as, read against the live store: the
    /// installed set says whether it is installed (the tag only says what the
    /// index held when the search ran), the queue says what a plan left behind.
    ///
    /// The one thing the tag decides: when the store does not know the name
    /// but xbps tagged the hit `[*]`, the row reports the hit's own version as
    /// installed rather than claiming the package is absent.
    fn view<'a>(&'a self, model: &'a PackageModelRust) -> RowView<'a> {
        let installed = model.installed.get(&self.name).or(self.tagged.as_ref());
        RowView {
            name: &self.name,
            version: &self.version,
            installed,
            queued: model.queue.get(&self.name).copied(),
            update_to: model.state.update_to(&self.name),
            description: Some(&self.description),
            // A `-Rs` line names no size, so the row borrows the repository's
            // unpacked size for that name: it is what the package will take on
            // disk, and it is the same number the installed list shows. An
            // index that moved between the two reads could be one version off,
            // which the row would show as the repository version anyway.
            size: model.repo_installed_sizes.get(&self.name).copied(),
            update_size: None,
            download: model.repo_download_sizes.get(&self.name).copied(),
        }
    }
}

impl Default for PackageModelRust {
    fn default() -> Self {
        Self {
            snapshot: Vec::new(),
            installed: InstalledMap::new(),
            updates: Vec::new(),
            orphans: Vec::new(),
            rows: Vec::new(),
            name_lower: Vec::new(),
            description_lower: Vec::new(),
            queue: HashMap::new(),
            state: InstalledState::new(&[], &[]),
            held_names: HashSet::new(),
            installed_sizes: HashMap::new(),
            repo_installed_sizes: HashMap::new(),
            repo_download_sizes: HashMap::new(),
            update_sizes: HashMap::new(),
            // The window opens on the installed list.
            filter: QString::from("installed"),
            search: QString::from(""),
            sort: QString::from("name"),
            sort_descending: false,
            count: 0,
            searching: false,
            search_loading: false,
            search_error: QString::from(""),
            search_error_args: QStringList::default(),
            active_search: None,
        }
    }
}

/// What one visible row showed before a change: enough to tell whether the
/// row's data moved, without comparing every field of the model.
#[derive(PartialEq, Eq)]
struct RowFingerprint {
    name: String,
    installed: Option<Inst>,
    queued: Option<QueueAction>,
    update_to: Option<String>,
}

impl PackageModelRust {
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
    pub(crate) fn snapshot(
        &mut self,
        records: Vec<InstalledPackage>,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
        updates: Vec<UpdateEntry>,
        orphans: Vec<String>,
    ) -> (bool, Vec<u32>, Vec<i32>) {
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
        self.update_sizes = update_sizes_of(&updates);
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
    pub(crate) fn store_state(
        &mut self,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
    ) -> (bool, Vec<u32>, Vec<i32>) {
        self.snapshot = self.records_from_map(&installed);
        self.snapshot
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.install(installed, queue)
    }

    fn install(
        &mut self,
        installed: InstalledMap,
        queue: Vec<QueueItem>,
    ) -> (bool, Vec<u32>, Vec<i32>) {
        // The records may have been replaced, so a row index of the previous
        // set can point past the new one: keep a fingerprint per visible name
        // rather than per row number.
        let before: Vec<RowFingerprint> = self
            .rows
            .iter()
            .filter(|row| self.snapshot.get(**row as usize).is_some())
            .map(|row| self.fingerprint(row))
            .collect();

        self.installed = installed;
        self.queue = queue
            .into_iter()
            .map(|item| (item.name, item.action))
            .collect();
        self.refresh_derived();
        self.cache_lowercases();

        let rows = self.compute_rows();
        if rows != self.rows {
            return (true, rows, Vec::new());
        }
        let affected = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                let after = self.fingerprint(candidate);
                !before.contains(&after)
            })
            .map(|(row, _)| i32::try_from(row).unwrap_or(i32::MAX))
            .collect();
        (false, rows, affected)
    }

    fn row_at(&self, row: i32) -> Option<&u32> {
        let index = usize::try_from(row).ok()?;
        self.rows.get(index)
    }

    /// The name a row stands for.
    fn name_of(&self, row: &u32) -> &str {
        &self.snapshot[*row as usize].name
    }

    fn row_view<'a>(&'a self, row: &u32) -> RowView<'a> {
        let record = &self.snapshot[*row as usize];
        let name = record.name.as_str();
        let installed = self.installed.get(name);
        let version = installed
            .map(|inst| inst.version.as_str())
            .unwrap_or(record.version.as_str());
        // The size the package takes on disk: the installed one, or — for a
        // package that is not installed — what the repository says its unpacked
        // version weighs. Both are `xbps-query -p installed_size`; an unknown
        // size stays empty rather than becoming a zero.
        let size = self
            .installed_sizes
            .get(name)
            .or_else(|| self.repo_installed_sizes.get(name))
            .copied();
        // Only a pending update has a "new version" size: the exact one from
        // `xbps-install -un`. The repository archive size is what a queued
        // install would fetch, so it is reported even without an update; the
        // repository unpacked size is not a new version, so it never fills
        // `update_size`.
        let (update_size, download) = match self.update_sizes.get(name) {
            Some((installed_size, download_size)) => (Some(*installed_size), Some(*download_size)),
            None => (None, self.repo_download_sizes.get(name).copied()),
        };
        RowView {
            name,
            version,
            installed,
            queued: self.queue.get(name).copied(),
            update_to: self.state.update_to(name),
            description: Some(record.description.as_str()),
            size,
            update_size,
            download,
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

    /// Rebuilds the lowercase search cache in place.
    ///
    /// Called from `install` only, after `snapshot` and the derived state are
    /// final: a record list that did not move keeps its cache, and one that
    /// did cannot leave a stale string behind. `clear` keeps the allocations
    /// instead of rebuilding the vectors per refresh.
    fn cache_lowercases(&mut self) {
        self.name_lower.clear();
        self.description_lower.clear();
        self.name_lower.reserve(self.snapshot.len());
        self.description_lower.reserve(self.snapshot.len());
        for record in &self.snapshot {
            self.name_lower.push(record.name.to_lowercase());
            self.description_lower
                .push(record.description.to_lowercase());
        }
    }

    /// Visible rows for the current filter and search.
    ///
    /// Built into a fresh vector so a caller can compare before committing: a
    /// keystroke in the search field that does not change the visible set must
    /// not reset the model, or QML loses the scroll position and selection.
    fn compute_rows(&self) -> Vec<u32> {
        // The name sort below relies on this: `snapshot` and `store_state`
        // both sort the records by name, so index order is name order.
        debug_assert!(
            self.snapshot
                .windows(2)
                .all(|pair| pair[0].name <= pair[1].name),
            "the records must stay sorted by name"
        );
        let mut candidates: Vec<u32> = (0..self.snapshot.len() as u32).collect();
        // The order belongs to the whole snapshot: sorting after filtering
        // would order the visible rows by the values of the rows that survived
        // the filter, so the same package could move when a keystroke narrows
        // the list.
        match self.sort.to_string().as_str() {
            "size" => self.sort_by_size(&mut candidates),
            // The records arrive sorted by name, so ascending needs no second
            // sort; descending reverses. A reverse is only a descending sort
            // because names are unique — a duplicate name would need a stable
            // descending sort instead.
            _ if self.sort_descending => candidates.reverse(),
            _ => {}
        }
        let filter = self.filter.to_string();
        let search = self.search.to_string().to_lowercase();
        candidates
            .into_iter()
            .filter(|row| self.matches(row, &filter, &search))
            .collect()
    }

    /// Orders candidates by their displayed size, the unknown ones last.
    fn sort_by_size(&self, candidates: &mut [u32]) {
        let descending = self.sort_descending;
        candidates.sort_by(|left, right| {
            let left_size = self.display_size_of(left);
            let right_size = self.display_size_of(right);
            // A row with no size is not a zero-byte row: it goes to the end in
            // both directions, so the user can ignore the unknown tail instead
            // of finding it at the top of a descending list.
            match (left_size, right_size) {
                (Some(left_size), Some(right_size)) => {
                    let order = left_size.cmp(&right_size);
                    if descending { order.reverse() } else { order }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| self.name_of(left).cmp(self.name_of(right)))
        });
    }

    /// The size the list shows for a row: the new version's when it has an
    /// update, the installed size otherwise.
    fn display_size_of(&self, row: &u32) -> Option<u64> {
        let name = self.name_of(row);
        match self.update_sizes.get(name) {
            Some((installed_size, _)) => Some(*installed_size),
            None => self.installed_sizes.get(name).copied(),
        }
    }

    /// Replaces the three size maps and the exact update sizes.
    ///
    /// The maps live in the snapshot, so a successful read always ships all
    /// three; a failed size take ships an empty map for its take, and the rows
    /// then report no size instead of a stale one.
    fn set_sizes(
        &mut self,
        installed_sizes: HashMap<String, u64>,
        repo_installed_sizes: HashMap<String, u64>,
        repo_download_sizes: HashMap<String, u64>,
    ) {
        self.installed_sizes = installed_sizes;
        self.repo_installed_sizes = repo_installed_sizes;
        self.repo_download_sizes = repo_download_sizes;
    }

    /// Commits `rows` as the visible set and keeps `count` in step.
    ///
    /// The count is what the view shows, and a search answer is what it shows
    /// while one is on screen: a snapshot that lands after the answer rebuilds
    /// the filter's own rows, and the header must keep counting the hits
    /// instead of falling back to the installed set.
    pub(crate) fn assign_rows(&mut self, rows: Vec<u32>) {
        self.rows = rows;
        self.count = self.visible_count();
    }

    /// The number of rows the view is showing: the ranked repository hits while
    /// a search is on screen, the filter's own rows otherwise.
    fn visible_count(&self) -> i32 {
        let rows = self
            .active_search
            .as_ref()
            .map_or(self.rows.len(), |search| search.hits.len());
        i32::try_from(rows).unwrap_or(i32::MAX)
    }

    /// Recomputes `rows` and `count` in place, without resetting the model.
    ///
    /// Production code does not use this: [`apply_change`] resets through Qt
    /// whenever the visible set moves. The tests call it after poking the
    /// fields directly, and `#[cfg(test)]` keeps the dead-code lint honest
    /// instead of silencing it.
    #[cfg(test)]
    fn rebuild(&mut self) -> bool {
        let rows = self.compute_rows();
        if rows == self.rows {
            return false;
        }
        self.assign_rows(rows);
        true
    }

    /// Sets the sort key and direction the way the Qt setters do, for the
    /// tests that drive the model without a `Pin`.
    #[cfg(test)]
    fn set_sort(&mut self, key: &str, descending: bool) {
        self.sort = QString::from(key);
        self.sort_descending = descending;
        self.rebuild();
    }

    /// Everything the installed/queue snapshot feeds one row.
    fn fingerprint(&self, row: &u32) -> RowFingerprint {
        let name = self.name_of(row);
        RowFingerprint {
            name: name.to_owned(),
            installed: self.installed.get(name).cloned(),
            queued: self.queue.get(name).copied(),
            update_to: self.state.update_to(name).map(str::to_string),
        }
    }

    /// Whether the derived state calls `name` an orphan, for the tests.
    #[cfg(test)]
    pub(crate) fn is_orphan_for_test(&self, name: &str) -> bool {
        self.state.is_orphan(name)
    }

    /// The visible row indices, for the tests.
    #[cfg(test)]
    pub(crate) fn row_indices(&self) -> Vec<u32> {
        self.rows.clone()
    }

    /// What each visible row stands for, for the tests: the name and the
    /// queue action, which is all a queue or hold click can move.
    #[cfg(test)]
    pub(crate) fn visible_rows(&self) -> Vec<VisibleRow> {
        self.rows
            .iter()
            .map(|row| VisibleRow {
                name: self.name_of(row).to_owned(),
                queued: self.queue.get(self.name_of(row)).copied(),
                update_to: self.state.update_to(self.name_of(row)).map(str::to_owned),
            })
            .collect()
    }

    /// Sets the filter and recomputes the visible rows, as the Qt setter does.
    #[cfg(test)]
    pub(crate) fn set_filter(&mut self, filter: &str) {
        self.filter = QString::from(filter);
        self.rebuild();
    }

    /// Enters a filter page the way the Qt setter does, and answers with the
    /// repository read that page asks for, if any. `search_loading` is only set
    /// when a request comes back, which is the invariant the window relies on.
    #[cfg(test)]
    fn set_filter_and_report(&mut self, filter: &str) -> Option<String> {
        self.filter = QString::from(filter);
        self.clear_search_rows();
        self.searching = false;
        self.search_error = QString::from("");
        self.rebuild();
        let read = listing_read_on_filter(filter, &self.search.to_string());
        self.search_loading = read.is_some();
        read
    }

    /// All visible names, for the tests.
    #[cfg(test)]
    pub(crate) fn visible_names(&self) -> Vec<&str> {
        // While a search is active the visible rows are the repository hits,
        // in the order the backend ranked them; the filter's rows are still
        // `rows`, waiting for the pattern to be cleared.
        if let Some(search) = &self.active_search {
            return search.hits.iter().map(|hit| hit.name.as_str()).collect();
        }
        self.rows.iter().map(|row| self.name_of(row)).collect()
    }

    /// Sets the search text and answers with the pattern the list must read
    /// from the repositories, if any.
    ///
    /// The search box is one box for packages, whatever filter the list shows:
    /// a non-empty pattern always asks xbps about the repositories, because a
    /// filter narrows the installed set while a search asks a different
    /// question. An emptied pattern restores the filter's own rows, and on
    /// Available that means asking for the whole repository listing again.
    ///
    /// `None` means nothing to read: an emptied field on a page that shows the
    /// installed set, or the answer already on screen.
    #[cfg(test)]
    fn set_search_and_report(&mut self, text: &str) -> Option<String> {
        self.search = QString::from(text);
        let pattern = text.trim().to_string();
        if pattern.is_empty() {
            self.clear_search_rows();
            return listing_read_on_filter(&self.filter.to_string(), &pattern);
        }
        if self
            .active_search
            .as_ref()
            .is_some_and(|search| search.pattern.eq_ignore_ascii_case(&pattern))
        {
            return None;
        }
        // The rows stay as they are until the answer lands: an empty list on
        // every keystroke would flash.
        Some(pattern)
    }

    /// Drops the repository answer and shows the filter's rows again.
    fn clear_search_rows(&mut self) {
        if self.active_search.take().is_some() {
            self.count = self.visible_count();
        }
    }

    /// Applies a repository answer the way `set_search_results` does; an empty
    /// `hits` with an error key is a failed read that keeps the pattern and no
    /// rows.
    ///
    /// The guard and the settled mark are the production ones: this helper used
    /// to accept everything and leave `search_loading` set, so a test could pass
    /// on a behaviour the window never has.
    #[cfg(test)]
    fn apply_search_results(&mut self, hits: &[kxbps_core::SearchHit], key: &str, _args: &[&str]) {
        if !accepts_search_answer(&self.filter.to_string(), &self.search.to_string()) {
            self.search_loading = false;
            return;
        }
        let hits: Vec<SearchRow> = hits.iter().map(SearchRow::of).collect();
        let hits: Vec<SearchRow> = if key.is_empty() {
            hits.into_iter()
                .filter(|hit| self.hit_visible(hit))
                .collect()
        } else {
            hits
        };
        self.active_search = Some(ActiveSearch {
            pattern: self.search.to_string().trim().to_owned(),
            hits,
            error_key: key.to_owned(),
        });
        self.count = self.visible_count();
        self.searching = true;
        self.search_loading = false;
        self.search_error = QString::from(key);
    }

    #[cfg(test)]
    fn search_results_active(&self) -> bool {
        self.active_search.is_some()
    }

    #[cfg(test)]
    fn search_error_key(&self) -> String {
        self.active_search
            .as_ref()
            .map_or_else(String::new, |search| search.error_key.clone())
    }

    /// The visible rows while a search is active: name, the tag, the version.
    #[cfg(test)]
    fn search_row_fingerprints(&self) -> Vec<(String, bool, String)> {
        self.active_search.as_ref().map_or_else(Vec::new, |search| {
            search
                .hits
                .iter()
                .map(|row| (row.name.clone(), row.tagged.is_some(), row.version.clone()))
                .collect()
        })
    }

    /// The queue action each visible search row shows.
    #[cfg(test)]
    fn search_row_actions(&self) -> Vec<String> {
        self.active_search.as_ref().map_or_else(Vec::new, |search| {
            search
                .hits
                .iter()
                .map(|row| {
                    self.queue
                        .get(&row.name)
                        .map_or_else(String::new, |action| queue_key(*action).to_owned())
                })
                .collect()
        })
    }

    /// Re-derives what a search row shows from the live store, without a new
    /// read: a queue toggle or a finished plan must move the pill.
    ///
    /// Search rows read the store on demand (`data()` for an active search),
    /// so nothing is copied here; the method keeps the call sites explicit.
    #[cfg(test)]
    fn refresh_search_rows(&mut self) {}

    /// Whether one repository hit the search returned takes a row now, under
    /// the filter the list currently shows.
    fn hit_visible(&self, hit: &SearchRow) -> bool {
        let filter = self.filter.to_string();
        let view = hit.view(self);
        search_hit_visible(
            &filter,
            view.installed,
            view.update_to,
            view.queued,
            self.state.is_orphan(&hit.name),
        )
    }

    fn matches(&self, row: &u32, filter: &str, search: &str) -> bool {
        let name = self.name_of(row);
        if !search.is_empty() {
            // The cache is filled in `install`, right after the records and
            // the derived state are final, and indexed like `snapshot`.
            let index = *row as usize;
            let name_hit = self
                .name_lower
                .get(index)
                .is_some_and(|lower| lower.contains(search));
            let description_hit = self
                .description_lower
                .get(index)
                .is_some_and(|lower| lower.contains(search));
            if !name_hit && !description_hit {
                return false;
            }
        }
        let installed = self.installed.get(name);
        match filter {
            // Every row the list has: xbps has no command that lists what is
            // not installed, so this is the system's own set.
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
}

impl cxx_qt::Constructor<(QObjectMutPtr,)> for qobject::PackageModel {
    type BaseArguments = (QObjectMutPtr,);
    type NewArguments = ();
    type InitializeArguments = ();

    fn route_arguments(
        arguments: (QObjectMutPtr,),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        ((), arguments, ())
    }

    fn new(_: Self::NewArguments) -> PackageModelRust {
        PackageModelRust::default()
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
        // While a search is active the rows are the ranked repository hits,
        // rendered against the live store so a queue toggle or a finished plan
        // moves the pill without a second read.
        if let Some(search) = &self.active_search {
            let Some(hit) = usize::try_from(index.row())
                .ok()
                .and_then(|row| search.hits.get(row))
            else {
                return cxx_qt_lib::QVariant::default();
            };
            // `pkgver` is not a role of the snapshot rows, but the details pane
            // and the selection tuple read it from every row: a search row
            // carries the `name-version` xbps printed.
            if role == role::PKGVER {
                return QVariant::from(&QString::from(hit.pkgver.as_str()));
            }
            return role_value(role, &hit.view(self));
        }
        let Some(row) = self.row_at(index.row()) else {
            return cxx_qt_lib::QVariant::default();
        };
        role_value(role, &self.row_view(row))
    }

    fn role_names(&self) -> QHash_i32_QByteArray {
        // `pkgver` rides with the other roles so a search row reads
        // `model.pkgver`; a snapshot row answers it as an invalid variant and
        // QML derives the same string from name and version.
        ROLES
            .iter()
            .copied()
            .chain(std::iter::once((role::PKGVER, "pkgver")))
            .map(|(id, name)| (id, QByteArray::from(name)))
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
        // A filter chip narrows the installed set; a search asks xbps a
        // different question, so picking a chip leaves the search and shows
        // that filter's own rows.
        let had_search = self.active_search.is_some();
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.clear_search_rows();
            data.searching = false;
            data.search_loading = false;
            data.search_error = QString::from("");
            data.search_error_args = QStringList::default();
        }
        self.as_mut().search_loading_changed();
        apply_change(self.as_mut(), |model| model.filter = value);
        // The rolling back to the filter's rows is a different set from the
        // search hits, so the view rebuilds; `apply_change` only resets when
        // its own rows moved.
        if had_search {
            qobject::model_begin_reset(base(self.as_mut()));
            qobject::model_end_reset(base(self.as_mut()));
        }
        self.as_mut().filter_changed();
        if had_search {
            self.as_mut().count_changed();
            self.as_mut().searching_changed();
            self.as_mut().search_error_changed();
            self.as_mut().search_error_args_changed();
        }
        // Available is the repository listing, not a view of the installed set:
        // with an empty search box it asks for every hit, the same read a
        // typed pattern asks for. Without this the page showed the installed
        // packages under a name that promises the repositories.
        if let Some(pattern) =
            listing_read_on_filter(&self.filter.to_string(), &self.search.to_string())
        {
            self.as_mut().mark_search_loading();
            self.as_mut().search_requested(QString::from(pattern));
        }
    }

    fn set_search(mut self: Pin<&mut Self>, value: cxx_qt_lib::QString) {
        if self.search == value {
            return;
        }
        let pattern = value.to_string().trim().to_owned();
        // An emptied field goes back to the filter's rows, with no read; a
        // changed pattern asks the repositories and leaves the current rows in
        // place until the answer lands. On Available the emptied field means
        // "the whole repository", which is a read of its own.
        if pattern.is_empty() {
            let had_search = self.active_search.is_some();
            {
                let data = Pin::get_mut(self.as_mut().rust_mut());
                data.search = value;
                data.clear_search_rows();
                data.searching = false;
                data.search_error = QString::from("");
                data.search_error_args = QStringList::default();
            }
            if had_search {
                qobject::model_begin_reset(base(self.as_mut()));
                qobject::model_end_reset(base(self.as_mut()));
                self.as_mut().count_changed();
                self.as_mut().searching_changed();
                self.as_mut().search_error_changed();
                self.as_mut().search_error_args_changed();
            }
            self.as_mut().search_changed();
            if let Some(read) = listing_read_on_filter(&self.filter.to_string(), &pattern) {
                self.as_mut().mark_search_loading();
                self.as_mut().search_requested(QString::from(read));
            }
            return;
        }
        // The answer already on screen is not read again: padding and case are
        // not a new question.
        let answered = self.active_search.as_ref().is_some_and(|search| {
            search.pattern.eq_ignore_ascii_case(&pattern) && search.error_key.is_empty()
        });
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.search = value;
        }
        self.as_mut().search_changed();
        if !answered {
            self.as_mut().mark_search_loading();
            self.as_mut()
                .search_requested(QString::from(pattern.as_str()));
        }
    }

    fn set_search_results(
        mut self: Pin<&mut Self>,
        hits_json: &cxx_qt_lib::QString,
        error_key: &cxx_qt_lib::QString,
        error_args: &cxx_qt_lib::QStringList,
    ) {
        // An answer the window no longer asks for: the field is empty and the
        // page is not the repository listing, so the read belongs to a page
        // the user already left. Applying it would put repository rows under a
        // filter that does not show them, and the loading mark must still be
        // cleared or the page waits for an answer that never comes.
        if !accepts_search_answer(&self.filter.to_string(), &self.search.to_string()) {
            self.as_mut().mark_search_settled();
            return;
        }
        let hits: Vec<kxbps_core::SearchHit> =
            serde_json::from_str(&hits_json.to_string()).unwrap_or_default();
        let key = error_key.to_string();
        let args: Vec<String> = error_args.iter().map(QString::to_string).collect();
        let hits: Vec<SearchRow> = hits.iter().map(SearchRow::of).collect();
        let hits: Vec<SearchRow> = if key.is_empty() {
            hits.into_iter()
                .filter(|hit| self.hit_visible(hit))
                .collect()
        } else {
            hits
        };
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.active_search = Some(ActiveSearch {
                pattern: data.search.to_string().trim().to_owned(),
                hits,
                error_key: key.clone(),
            });
            data.count = data.visible_count();
            data.searching = true;
            data.search_loading = false;
            data.search_error = QString::from(key.as_str());
            data.search_error_args = args
                .iter()
                .map(|arg| QString::from(arg.as_str()))
                .collect::<QStringList>();
        }
        // The rows are a different set, not a re-read of the same one: this is
        // a full reset, which is what tells the view to rebuild its delegates.
        qobject::model_begin_reset(base(self.as_mut()));
        qobject::model_end_reset(base(self.as_mut()));
        self.as_mut().count_changed();
        self.as_mut().searching_changed();
        self.as_mut().search_loading_changed();
        self.as_mut().search_error_changed();
        self.as_mut().search_error_args_changed();
    }

    /// Marks a repository read as in flight, so the header says it is reading
    /// instead of counting rows the answer is about to replace.
    fn mark_search_loading(mut self: Pin<&mut Self>) {
        if self.search_loading {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.search_loading = true;
        }
        self.as_mut().search_loading_changed();
    }

    /// Clears the in-flight mark without applying an answer.
    fn mark_search_settled(mut self: Pin<&mut Self>) {
        if !self.search_loading {
            return;
        }
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.search_loading = false;
        }
        self.as_mut().search_loading_changed();
    }

    fn set_sort(mut self: Pin<&mut Self>, value: cxx_qt_lib::QString) {
        // Only the two keys the model knows: an unknown word is the name
        // order, which is what the header falls back to.
        let value = match value.to_string().as_str() {
            "size" => cxx_qt_lib::QString::from("size"),
            _ => cxx_qt_lib::QString::from("name"),
        };
        if self.sort == value {
            return;
        }
        apply_change(self.as_mut(), |model| model.sort = value.clone());
        self.as_mut().sort_changed();
    }

    fn set_sort_descending(mut self: Pin<&mut Self>, value: bool) {
        if self.sort_descending == value {
            return;
        }
        apply_change(self.as_mut(), |model| model.sort_descending = value);
        self.as_mut().sort_descending_changed();
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
            data.set_sizes(
                sizes_of(&snapshot.installed_sizes),
                sizes_of(&snapshot.repo_installed_sizes),
                sizes_of(&snapshot.repo_download_sizes),
            );
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
    rows: Vec<u32>,
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

/// The humanised size, or an empty string when the read reported none: a row
/// with no size must not render `0 B`.
fn size_text(bytes: Option<u64>) -> QVariant {
    match bytes {
        // A zero byte size is a real answer xbps printed — a transitional
        // package — and must not render as `0 B`: the row shows an empty field.
        Some(0) => text(""),
        Some(bytes) => text(&format_size(bytes)),
        // Unknown is not zero: an invalid variant keeps QML from printing "0 B".
        None => QVariant::default(),
    }
}

/// The raw byte count for sorting; invalid when unknown, so QML's comparison
/// sees "no value" rather than zero.
fn size_bytes(bytes: Option<u64>) -> QVariant {
    match bytes {
        Some(bytes) => QVariant::from(&bytes),
        None => QVariant::default(),
    }
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

/// Wire key of a queue action, the same spelling the store persists.
pub(crate) fn queue_key(action: QueueAction) -> &'static str {
    match action {
        QueueAction::Install => "install",
        QueueAction::Remove => "remove",
        QueueAction::Update => "update",
    }
}

/// A size list as the `name -> bytes` map the model sorts on.
///
/// xbps prints one line per package, so a repeated name is a broken answer;
/// the first one wins and the rest are dropped rather than summed.
pub(crate) fn sizes_of(entries: &[kxbps_core::SizeEntry]) -> HashMap<String, u64> {
    let mut sizes: HashMap<String, u64> = HashMap::with_capacity(entries.len());
    for entry in entries {
        sizes.entry(entry.name.clone()).or_insert(entry.bytes);
    }
    sizes
}

/// The exact sizes of the pending updates, keyed by package name.
fn update_sizes_of(updates: &[UpdateEntry]) -> HashMap<String, (u64, u64)> {
    updates
        .iter()
        .map(|update| {
            (
                update.name.clone(),
                (update.installed_size, update.download_size),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::{InstallState, ListFilter};

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

    fn inst(version: &str, explicit: bool, held: bool) -> Inst {
        Inst {
            version: version.to_owned(),
            explicit,
            held,
            at: 0,
        }
    }

    /// A small installed set: vim explicit, bash and linux dependencies, curl
    /// explicit.
    fn installed() -> InstalledMap {
        [
            ("bash", inst("5.2.037_1", false, false)),
            ("curl", inst("8.16.0_1", true, false)),
            ("linux", inst("6.18.63_1", false, false)),
            ("vim", inst("9.2.1031_1", true, false)),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
    }

    fn records() -> Vec<InstalledPackage> {
        vec![
            record("bash-5.2.037_1", "The GNU Bourne Again Shell"),
            record("curl-8.16.0_1", "Command line tool for transferring data"),
            record("linux-6.18.63_1", "Linux kernel"),
            record("vim-9.2.1031_1", "Vi Improved"),
        ]
    }

    fn model() -> PackageModelRust {
        let mut model = PackageModelRust::default();
        install_records(&mut model, &installed(), records(), Vec::new(), Vec::new());
        model
    }

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
    fn an_empty_model_says_nothing_is_installed() {
        let model = PackageModelRust::default();
        assert_eq!(
            model.filter.to_string(),
            "installed",
            "the window opens on Installed"
        );
        assert_eq!(model.count, 0, "nothing is known to be installed yet");
        assert!(model.visible_names().is_empty());
    }

    #[test]
    fn a_snapshot_replaces_the_rows_with_the_system_records() {
        let mut model = PackageModelRust::default();
        let (rows_changed, rows, _) =
            model.snapshot(records(), installed(), Vec::new(), Vec::new(), Vec::new());
        assert!(rows_changed, "the first read fills the list");
        model.assign_rows(rows);

        assert_eq!(model.count, 4);
        assert_eq!(model.visible_names(), vec!["bash", "curl", "linux", "vim"]);
        assert_eq!(model.name_of(&model.rows[1]), "curl");
    }

    #[test]
    fn an_installed_row_shows_only_what_xbps_printed() {
        let model = model();
        let row = model
            .rows
            .iter()
            .position(|row| model.name_of(row) == "vim")
            .expect("vim is installed");
        let view = model.row_view(&model.rows[row]);
        assert_eq!(
            role_value(role::NAME, &view)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "vim"
        );
        assert_eq!(
            role_value(role::VERSION, &view)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "9.2.1031_1",
            "the installed version is the only one there is"
        );
        assert_eq!(
            role_value(role::SHORT, &view)
                .value::<QString>()
                .unwrap()
                .to_string(),
            "Vi Improved",
            "the record's description fills the short text"
        );
        assert!(role_value(role::EXPLICIT, &view).value::<bool>().unwrap());
        assert!(
            role_value(999, &view).type_name().is_empty(),
            "a role the model does not have is invalid, not invented"
        );
    }

    #[test]
    fn installed_filter_lists_the_records() {
        let model = model();
        assert_eq!(model.rows.len(), 4);
        assert_eq!(model.count, 4);
        assert!(model.visible_names().contains(&"vim"));
    }

    #[test]
    fn updates_filter_follows_the_reported_list() {
        let mut model = model();
        install(
            &mut model,
            &installed(),
            vec![update("linux-6.18.63_1"), update("curl-8.16.0_1")],
            Vec::new(),
        );
        set_filter(&mut model, "updates");
        assert_eq!(model.visible_names(), vec!["curl", "linux"]);
    }

    #[test]
    fn a_hold_hides_its_update_immediately() {
        let mut model = model();
        install(
            &mut model,
            &installed(),
            vec![update("linux-6.18.63_1")],
            Vec::new(),
        );
        set_filter(&mut model, "updates");
        assert_eq!(model.visible_names(), vec!["linux"]);

        // The hold lives in the store, not in the reported list: the model must
        // hide the row anyway.
        let mut held = installed();
        held.get_mut("linux").unwrap().held = true;
        let (_, rows, _) = model.store_state(held, Vec::new());
        model.assign_rows(rows);
        set_filter(&mut model, "updates");
        assert!(
            model.visible_names().is_empty(),
            "held packages are not updates"
        );
    }

    #[test]
    fn orphans_filter_follows_the_reported_list() {
        let mut model = model();
        install(
            &mut model,
            &installed(),
            Vec::new(),
            vec!["curl".to_owned()],
        );
        set_filter(&mut model, "orphans");
        assert_eq!(model.visible_names(), vec!["curl"]);
    }

    #[test]
    fn the_explicit_filter_shows_only_the_packages_installed_on_purpose() {
        let mut explicit = model();
        set_filter(&mut explicit, "explicit");
        let explicit_names: HashSet<String> = explicit
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(explicit_names.len(), 2, "vim and curl are explicit");
        assert!(explicit_names.contains("vim"));
    }

    #[test]
    fn an_orphan_that_is_still_installed_and_explicit_is_in_both_filters() {
        // The two filters answer different questions — "installed on purpose"
        // and "nothing needs it" — so a package can be in both. Nothing here
        // invents an exclusion the backend does not have.
        let mut orphans = model();
        install(
            &mut orphans,
            &installed(),
            Vec::new(),
            vec!["curl".to_owned()],
        );
        set_filter(&mut orphans, "orphans");
        assert_eq!(orphans.visible_names(), vec!["curl"]);
        set_filter(&mut orphans, "explicit");
        assert!(orphans.visible_names().contains(&"curl"));
    }

    #[test]
    fn an_orphan_xbps_no_longer_lists_is_not_a_row() {
        let mut orphans = model();
        let mut without_curl = installed();
        without_curl.remove("curl");
        install(
            &mut orphans,
            &without_curl,
            Vec::new(),
            vec!["curl".to_owned()],
        );
        set_filter(&mut orphans, "orphans");
        assert!(
            orphans.visible_names().is_empty(),
            "the rows are the installed set, not the orphan list"
        );
    }

    #[test]
    fn queued_filter_follows_the_queue_map() {
        let mut model = model();
        set_filter(&mut model, "queued");
        assert_eq!(model.rows.len(), 0, "nothing is queued");

        model.queue.insert("vim".to_string(), QueueAction::Install);
        model.rebuild();
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn rebuild_reports_no_change_when_the_visible_set_is_identical() {
        let mut model = model();
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
        let mut model = model();
        model.search = QString::from("Bourne");
        model.rebuild();
        assert_eq!(
            model.visible_names(),
            vec!["bash"],
            "matches the description"
        );

        model.search = QString::from("VIM");
        model.rebuild();
        assert_eq!(model.visible_names(), vec!["vim"], "matches the name");
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
            let mut model = model();
            set_filter(&mut model, json);
            assert_eq!(model.filter.to_string(), json, "filter {id} round-trips");
        }
    }

    #[test]
    fn roles_cover_every_documented_name() {
        assert_eq!(ROLES.len(), 15);
        let names: Vec<&str> = ROLES.iter().map(|(_, name)| *name).collect();
        assert_eq!(
            names,
            vec![
                "name",
                "version",
                "short",
                "installedVersion",
                "explicit",
                "held",
                "queuedAction",
                "isUpdate",
                "updateVersion",
                "size",
                "sizeBytes",
                "updateSize",
                "updateSizeBytes",
                "download",
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
        let model = model();
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
        assert!(!role_value(role::IS_UPDATE, &view).value::<bool>().unwrap());
        assert!(
            role_value(role::UPDATE_VERSION + 1, &view)
                .type_name()
                .is_empty(),
            "unknown role"
        );
    }

    /// The model's installed set plus `ark`: a store change after a read, so
    /// the tests that use it go through `store_state`.
    fn installed_with_ark() -> InstalledMap {
        let mut installed = installed();
        installed.insert("ark".to_string(), inst("25.08.0_1", true, false));
        installed
    }

    #[test]
    fn a_store_change_reports_only_the_rows_that_moved() {
        // A finished job, a queue toggle: the store gains a package, the list
        // gains one row and nothing else is rebuilt.
        let mut model = model();
        set_filter(&mut model, "installed");
        assert_eq!(model.count, 4);

        let (rows_changed, rows, affected) = model.store_state(installed_with_ark(), Vec::new());
        assert!(rows_changed, "the installed filter gained a row");
        assert_eq!(rows.len(), 5);
        assert!(affected.is_empty(), "a reset does not need dataChanged");
    }

    #[test]
    fn a_queue_change_only_reports_the_queued_row() {
        let mut model = model();
        set_filter(&mut model, "installed");
        let before = model.rows.clone();

        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];
        let (rows_changed, rows, affected) = model.store_state(installed(), queue);
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
    fn queue_snapshot_drives_the_queued_filter_and_role() {
        let mut model = model();
        set_filter(&mut model, "queued");
        assert_eq!(model.rows.len(), 0);

        // The store's own wire format: `[{name, action}]`, not a map.
        let queue = vec![QueueItem {
            name: "vim".into(),
            action: QueueAction::Install,
        }];
        let (rows_changed, rows, _) = model.store_state(installed(), queue);
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

    /// The size maps a snapshot carries: installed sizes, the repository's
    /// unpacked sizes, and the repository's archive sizes.
    fn sizes() -> (
        HashMap<String, u64>,
        HashMap<String, u64>,
        HashMap<String, u64>,
    ) {
        let map = |pairs: [(&str, u64); 4]| -> HashMap<String, u64> {
            pairs
                .into_iter()
                .map(|(name, bytes)| (name.to_owned(), bytes))
                .collect()
        };
        (
            map([
                ("bash", 9_000_000),
                ("curl", 1_200_000),
                ("linux", 500_000_000),
                ("vim", 2_200_000),
            ]),
            map([
                ("bash", 9_100_000),
                ("curl", 1_300_000),
                ("linux", 510_000_000),
                ("vim", 1_800_000),
            ]),
            map([
                ("bash", 2_000_000),
                ("curl", 300_000),
                ("linux", 150_000_000),
                ("vim", 1_700_000),
            ]),
        )
    }

    fn model_with_sizes(updates: Vec<UpdateEntry>) -> PackageModelRust {
        let (installed_sizes, repo_installed, repo_download) = sizes();
        let mut model = PackageModelRust::default();
        model.set_sizes(installed_sizes, repo_installed, repo_download);
        install_records(&mut model, &installed(), records(), updates, Vec::new());
        model
    }

    fn string_of(variant: QVariant) -> Option<String> {
        variant.value::<QString>().map(|text| text.to_string())
    }

    fn bytes_of(variant: QVariant) -> Option<u64> {
        variant.value::<u64>()
    }

    /// The six size roles of the row called `name`.
    fn size_roles(model: &PackageModelRust, name: &str) -> [QVariant; 6] {
        let row = model
            .rows
            .iter()
            .find(|row| model.name_of(row) == name)
            .unwrap_or_else(|| panic!("{name} is not visible"));
        let view = model.row_view(row);
        [
            role_value(role::SIZE, &view),
            role_value(role::SIZE_BYTES, &view),
            role_value(role::UPDATE_SIZE, &view),
            role_value(role::UPDATE_SIZE_BYTES, &view),
            role_value(role::DOWNLOAD, &view),
            role_value(role::DOWNLOAD_BYTES, &view),
        ]
    }

    #[test]
    fn an_installed_row_shows_the_installed_size() {
        let model = model_with_sizes(Vec::new());
        let [
            size,
            size_bytes,
            update,
            update_bytes,
            download,
            download_bytes,
        ] = size_roles(&model, "vim");
        assert_eq!(string_of(size).as_deref(), Some("2.1 MiB"));
        assert_eq!(bytes_of(size_bytes), Some(2_200_000));
        assert_eq!(string_of(update), None, "no update, no new size");
        assert_eq!(bytes_of(update_bytes), None);
        assert_eq!(
            string_of(download).as_deref(),
            Some("1.6 MiB"),
            "the repository archive size is what a queued install would fetch"
        );
        assert_eq!(bytes_of(download_bytes), Some(1_700_000));
    }

    #[test]
    fn an_update_row_shows_the_new_size_and_the_exact_download() {
        // The exact sizes the dry run reported win over the rounded repository
        // metadata: the index only printed a humanised value.
        let mut update = update("linux-6.19.0_1");
        update.installed_size = 600_000_000;
        update.download_size = 160_000_000;
        let model = model_with_sizes(vec![update]);
        let [
            size,
            size_bytes,
            update_size,
            update_bytes,
            download,
            download_bytes,
        ] = size_roles(&model, "linux");
        assert_eq!(
            string_of(size).as_deref(),
            Some("476.8 MiB"),
            "the version the user has keeps its installed size"
        );
        assert_eq!(bytes_of(size_bytes), Some(500_000_000));
        assert_eq!(
            string_of(update_size).as_deref(),
            Some("572.2 MiB"),
            "the new version's unpacked size"
        );
        assert_eq!(bytes_of(update_bytes), Some(600_000_000));
        assert_eq!(string_of(download).as_deref(), Some("152.6 MiB"));
        assert_eq!(bytes_of(download_bytes), Some(160_000_000));
    }

    #[test]
    fn sorting_uses_the_size_the_column_shows() {
        // An update row shows the new version's size, so that is the sort key:
        // curl's installed size is 1.2 MB but its update is 700 MB, which puts
        // it above linux (500 MB) even though the old version is tiny.
        let mut update = update("curl-8.17.0_1");
        update.installed_size = 700_000_000;
        update.download_size = 200_000_000;
        let mut model = model_with_sizes(vec![update]);
        set_filter(&mut model, "all");
        model.set_sort("size", true);
        assert_eq!(
            model.visible_names(),
            vec!["curl", "linux", "bash", "vim"],
            "the update's new size is the sort key, not the installed one"
        );
    }

    #[test]
    fn a_row_without_a_size_reports_nothing_rather_than_zero() {
        let (installed_sizes, repo_installed, repo_download) = sizes();
        let mut model = PackageModelRust::default();
        // The maps arrive with `curl` missing from all three takes: the row is
        // still there, its sizes are simply unknown.
        let without = |map: HashMap<String, u64>| {
            map.into_iter()
                .filter(|(name, _)| name != "curl")
                .collect::<HashMap<_, _>>()
        };
        model.set_sizes(
            without(installed_sizes),
            without(repo_installed),
            without(repo_download),
        );
        install_records(&mut model, &installed(), records(), Vec::new(), Vec::new());

        let [
            size,
            size_bytes,
            _update,
            _update_bytes,
            download,
            download_bytes,
        ] = size_roles(&model, "curl");
        assert_eq!(string_of(size), None, "no size is not zero bytes");
        assert_eq!(bytes_of(size_bytes), None);
        assert_eq!(string_of(download), None);
        assert_eq!(bytes_of(download_bytes), None);
    }

    #[test]
    fn sorting_by_size_orders_the_whole_list_before_the_filter() {
        let mut model = model_with_sizes(Vec::new());
        set_filter(&mut model, "all");
        model.set_sort("size", false);
        assert_eq!(
            model.visible_names(),
            vec!["curl", "vim", "bash", "linux"],
            "ascending by installed bytes"
        );

        model.set_sort("size", true);
        assert_eq!(
            model.visible_names(),
            vec!["linux", "bash", "vim", "curl"],
            "descending by installed bytes"
        );

        // The order belongs to the whole snapshot: a search keeps the relative
        // order of the rows it keeps. "i" matches every fixture row (bash and
        // curl through their descriptions); descending by size that is
        // linux, bash, vim, curl.
        model.search = QString::from("i");
        model.rebuild();
        assert_eq!(model.visible_names(), vec!["linux", "bash", "vim", "curl"]);

        model.set_sort("name", false);
        assert_eq!(model.visible_names(), vec!["bash", "curl", "linux", "vim"]);
        model.set_sort("name", true);
        assert_eq!(
            model.visible_names(),
            vec!["vim", "linux", "curl", "bash"],
            "the name order reverses too"
        );
    }

    #[test]
    fn sorting_by_size_before_filtering_is_not_a_visible_rows_sort() {
        // `ark` (5 MB) sorts last by name among the visible rows, and a
        // visible-only sort that ran after filtering would have left it there.
        // The full-list ascending sort puts it between curl (1.2 MB) and vim
        // (2.2 MB). The query keeps ark and vim, whose relative order is the
        // full-list one: vim after ark.
        let (mut installed_sizes, repo_installed, repo_download) = sizes();
        installed_sizes.insert("ark".to_owned(), 5_000_000);
        let mut model = PackageModelRust::default();
        model.set_sizes(installed_sizes, repo_installed, repo_download);
        let with_ark: InstalledMap = installed()
            .into_iter()
            .chain([("ark".to_owned(), inst("25.08.0_1", true, false))])
            .collect();
        let mut records = records();
        records.push(record("ark-25.08.0_1", "Archives"));
        install_records(&mut model, &with_ark, records, Vec::new(), Vec::new());
        set_filter(&mut model, "all");

        model.set_sort("size", false);
        assert_eq!(
            model.visible_names(),
            vec!["curl", "vim", "ark", "bash", "linux"],
            "the whole snapshot is ordered by size"
        );

        model.search = QString::from("ar");
        model.rebuild();
        assert_eq!(
            model.visible_names(),
            vec!["ark"],
            "a filter keeps the full-list order of the rows it kept"
        );
    }

    #[test]
    fn a_row_without_a_size_sorts_last_in_both_directions() {
        let (installed_sizes, repo_installed, repo_download) = sizes();
        let mut model = PackageModelRust::default();
        model.set_sizes(
            installed_sizes
                .into_iter()
                .filter(|(name, _)| name != "bash")
                .collect(),
            repo_installed,
            repo_download,
        );
        install_records(&mut model, &installed(), records(), Vec::new(), Vec::new());
        set_filter(&mut model, "all");

        model.set_sort("size", false);
        assert_eq!(
            model.visible_names(),
            vec!["curl", "vim", "linux", "bash"],
            "an unknown size is last when ascending"
        );
        model.set_sort("size", true);
        assert_eq!(
            model.visible_names(),
            vec!["linux", "vim", "curl", "bash"],
            "and still last when descending"
        );
    }

    /// 1340 records, names sorted, descriptions two words each — the size of
    /// this machine's installed set, so a test can time the real code paths.
    fn scale_records(n: usize) -> Vec<InstalledPackage> {
        (0..n)
            .map(|i| {
                record(
                    &format!("pkg{i:04}-1.{i}_1"),
                    &format!("Package number {i}"),
                )
            })
            .collect()
    }

    fn scale_installed(n: usize) -> InstalledMap {
        (0..n)
            .map(|i| (format!("pkg{i:04}"), inst("1.0_1", true, false)))
            .collect()
    }

    /// A model over `n` synthetic packages, sorted by name, Installed filter.
    fn scale_model(n: usize) -> PackageModelRust {
        let mut model = PackageModelRust::default();
        install_records(
            &mut model,
            &scale_installed(n),
            scale_records(n),
            Vec::new(),
            Vec::new(),
        );
        model
    }

    #[test]
    fn name_order_is_the_snapshot_order_reversed_for_descending() {
        // The records arrive name-sorted and the ascending path must not sort
        // them again; descending reverses. Both must equal the full sort the
        // code used to do, on the whole 1340-row set.
        let mut model = scale_model(1340);
        let sorted = {
            let mut names: Vec<String> = model
                .snapshot
                .iter()
                .map(|record| record.name.clone())
                .collect();
            names.sort();
            names
        };

        model.set_sort("name", false);
        assert_eq!(
            model.visible_names(),
            sorted,
            "ascending is the record order"
        );

        model.set_sort("name", true);
        let mut reversed = sorted.clone();
        reversed.reverse();
        assert_eq!(model.visible_names(), reversed, "descending is its reverse");
    }

    #[test]
    fn a_store_change_refreshes_the_cached_lowercase_text() {
        // A stale lowercase cache would keep matching a description the record
        // no longer has: the old text must stop matching after the change.
        let mut model = model();
        set_filter(&mut model, "all");
        model.search = QString::from("bourne");
        model.rebuild();
        assert_eq!(model.visible_names(), vec!["bash"], "matches the old text");

        let mut changed = records();
        changed[0].description = String::from("A different shell entirely");
        let (_, rows, _) = model.snapshot(changed, installed(), Vec::new(), Vec::new(), Vec::new());
        model.assign_rows(rows);

        assert!(
            model.visible_names().is_empty(),
            "the previous description no longer matches"
        );
        model.search = QString::from("DIFFERENT");
        model.rebuild();
        assert_eq!(
            model.visible_names(),
            vec!["bash"],
            "the new description matches case-insensitively"
        );
    }

    #[test]
    fn the_lowercase_cache_tracks_the_records_it_was_built_from() {
        // The cache is indexed like `snapshot`: every record change that
        // replaces the list must rebuild it, or a row would be tested against
        // another row's text.
        let mut model = scale_model(3);
        set_filter(&mut model, "all");
        model.search = QString::from("package number 1");
        model.rebuild();
        assert_eq!(
            model.visible_names(),
            vec!["pkg0001"],
            "the description cache belongs to the record at the same index"
        );

        let mut renamed = scale_records(3);
        renamed[0].name = String::from("bas");
        let (_, rows, _) = model.snapshot(
            renamed,
            scale_installed(3),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        model.assign_rows(rows);
        model.search = QString::from("bas");
        model.rebuild();
        assert_eq!(
            model.visible_names(),
            vec!["bas"],
            "the name cache follows the renamed record"
        );
        model.search = QString::from("pkg0000");
        model.rebuild();
        assert!(
            model.visible_names().is_empty(),
            "the record stopped matching its previous name"
        );
    }

    #[test]
    fn scale_search_timing_is_a_smoke_bound_not_a_budget() {
        // Smoke bound only: a loaded machine must never fail the suite. The
        // numbers are printed for the report; nothing asserts their size.
        use std::time::Instant;

        let mut model = scale_model(1340);
        set_filter(&mut model, "all");
        let mut per_call: Vec<f64> = Vec::with_capacity(30);
        for _ in 0..30 {
            let start = Instant::now();
            model.search = QString::from("Number 12");
            model.rebuild();
            per_call.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert!(
            !model.visible_names().is_empty(),
            "the needle matches rows: 12, 120-129, 1200-1299 minus the filter"
        );
        per_call.sort_by(|left, right| left.total_cmp(right));
        let min = per_call[0];
        let median = per_call[per_call.len() / 2];
        let max = per_call[per_call.len() - 1];
        eprintln!("scale_search_timing: min {min:.3} ms, median {median:.3} ms, max {max:.3} ms");
        assert!(
            max < 200.0,
            "smoke bound (not a performance requirement): {max:.3} ms"
        );
    }

    /// A repository hit as the list receives it from `read_list_search`.
    fn search_hit(name: &str, version: &str, installed: bool) -> kxbps_core::SearchHit {
        kxbps_core::SearchHit {
            installed,
            name: name.to_owned(),
            version: version.to_owned(),
            pkgver: format!("{name}-{version}"),
            description: format!("{name} description"),
        }
    }

    #[test]
    fn a_non_empty_search_asks_for_repository_hits_on_every_filter() {
        // The search box is one box for packages: it does not matter which tab
        // the user is on, typing asks xbps about the repositories.
        for filter in ["installed", "all", "updates", "queued", "held", "orphans"] {
            let mut model = model();
            set_filter(&mut model, filter);
            let asked = model.set_search_and_report("vim");
            assert_eq!(
                asked.as_deref(),
                Some("vim"),
                "filter {filter}: a non-empty search asks the repositories"
            );
        }
    }

    #[test]
    fn a_pattern_already_asked_for_is_not_asked_again() {
        let mut model = model();
        assert_eq!(model.set_search_and_report("vim").as_deref(), Some("vim"));
        model.apply_search_results(&[search_hit("vim", "9.2.1031_1", false)], "", &[]);
        assert_eq!(
            model.set_search_and_report("vim"),
            None,
            "the same pattern is the answer already on screen"
        );
        assert_eq!(
            model.set_search_and_report(" VIM "),
            None,
            "padding and case are not a new question"
        );
        assert_eq!(
            model.set_search_and_report("vim-"),
            Some("vim-".to_owned()),
            "a longer pattern is a new question"
        );
    }

    #[test]
    fn a_snapshot_after_the_listing_answer_keeps_the_hit_count() {
        let mut model = model();
        set_filter(&mut model, "all");
        model.apply_search_results(
            &[
                search_hit("ark", "25.08.0_1", false),
                search_hit("vim", "9.2.1031_1", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.count, 2);

        // The startup read lands after the listing: the header must keep
        // counting what the view shows, not the filter's rows behind it.
        install_records(&mut model, &installed(), records(), Vec::new(), Vec::new());
        assert_eq!(model.count, 2, "the answer is still on screen");
        assert_eq!(model.visible_names(), ["ark", "vim"]);
    }

    #[test]
    fn a_listing_answer_clears_the_loading_mark_and_sets_the_count() {
        // The model enters `searchLoading` before it asks for the listing; the
        // answer is the only thing that clears it. A cached answer goes through
        // the same path, so this is also the cache-hit stop condition.
        let mut model = model();
        set_filter(&mut model, "all");
        model.search_loading = true;
        model.apply_search_results(
            &[
                search_hit("ark", "25.08.0_1", false),
                search_hit("vim", "9.2.1031_1", true),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.count, 3, "the count is the size of the answer");
        assert!(
            !model.search_loading,
            "the page stops loading when the answer lands"
        );
        assert!(model.search_results_active(), "the hits are the rows");
        assert_eq!(model.search_error_key(), "", "a good read reports no error");
    }

    #[test]
    fn an_answer_under_installed_with_an_empty_search_is_dropped_and_settles() {
        // The read belonged to a page the user already left: applying it would
        // put repository rows under a filter that does not show them, and
        // leaving the loading mark set would strand the next page.
        let mut model = model();
        set_filter(&mut model, "installed");
        let before: Vec<String> = model
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        model.search_loading = true;
        model.apply_search_results(&[search_hit("vim", "9.2.1031_1", false)], "", &[]);
        assert!(
            !model.search_results_active(),
            "a stale answer does not become the rows"
        );
        assert!(!model.search_loading, "dropping an answer settles the page");
        let after: Vec<String> = model
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(after, before, "the installed rows stay as they were");
    }

    #[test]
    fn available_asks_for_the_listing_every_time_the_page_is_entered() {
        // The backend may answer from its cache, but the model may never assume
        // it: every entry asks, and the loading mark exists only together with
        // that request.
        let mut model = model();
        assert_eq!(
            model.set_filter_and_report("all"),
            Some(String::new()),
            "entering Available asks for the whole repository"
        );
        assert!(
            model.search_loading,
            "loading is set together with the request"
        );
        model.apply_search_results(&[search_hit("vim", "9.2.1031_1", false)], "", &[]);
        assert_eq!(model.count, 1);
        assert!(!model.search_loading, "the answer settles the page");

        assert_eq!(
            model.set_filter_and_report("installed"),
            None,
            "a filter that shows the installed set reads nothing"
        );
        assert!(!model.search_loading, "no request means no loading mark");

        assert_eq!(
            model.set_filter_and_report("all"),
            Some(String::new()),
            "a second entry still asks; the answer may come from the cache"
        );
        assert_eq!(
            listing_read_on_filter("all", "   "),
            Some(String::new()),
            "a field of blanks is still the whole repository"
        );
        assert_eq!(listing_read_on_filter("all", "vim"), None);
        assert_eq!(listing_read_on_filter("orphans", ""), None);
        assert!(accepts_search_answer("all", ""));
        assert!(accepts_search_answer("all", "vim"));
        assert!(accepts_search_answer("installed", "vim"));
        assert!(!accepts_search_answer("installed", ""));
        assert!(!accepts_search_answer("updates", ""));
    }

    #[test]
    fn an_empty_search_restores_the_filters_rows_without_reading_xbps() {
        let mut model = model();
        set_filter(&mut model, "installed");
        let installed_rows: Vec<String> = model
            .visible_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        model.set_search_and_report("vim");
        model.apply_search_results(&[search_hit("vim", "9.2.1031_1", true)], "", &[]);
        assert_eq!(
            model.visible_names(),
            vec!["vim"],
            "the hit replaces the list"
        );
        let asked = model.set_search_and_report("");
        assert_eq!(asked, None, "an empty field asks xbps nothing");
        assert_eq!(
            model.visible_names(),
            installed_rows,
            "the filter's own rows come back from the snapshot"
        );
        assert!(
            !model.search_results_active(),
            "the search state is gone with the pattern"
        );
    }

    #[test]
    fn an_empty_search_on_available_asks_for_the_whole_repository_listing() {
        // Available is not a view of the installed set: an empty box there means
        // "everything the repositories have", which is a read of its own.
        let mut model = model();
        set_filter(&mut model, "all");
        assert_eq!(
            model.set_search_and_report(""),
            Some(String::new()),
            "the empty pattern is the repository listing"
        );

        // Every other filter keeps its rows and reads nothing.
        set_filter(&mut model, "installed");
        assert_eq!(model.set_search_and_report(""), None);
    }

    #[test]
    fn search_hit_rows_are_the_ranked_repository_answer_not_the_installed_set() {
        let mut model = model();
        set_filter(&mut model, "all");
        model.set_search_and_report("vim");
        // The Available page reads the repositories, so every hit is in.
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", false),
                search_hit("vimb", "4.0.0_1", false),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        // The order is the backend's: ranking happens before the list sees it.
        assert_eq!(model.visible_names(), vec!["vim", "vimb", "bash"]);
        assert_eq!(model.count, 3);
    }

    #[test]
    fn a_search_hit_row_reports_its_own_installed_state() {
        let mut model = model();
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("bash", "5.3_2", false),
                search_hit("curl", "8.16.0_1", true),
            ],
            "",
            &[],
        );
        let rows = model.search_row_fingerprints();
        assert_eq!(
            rows,
            vec![
                ("bash".to_owned(), false, "5.3_2".to_owned()),
                ("curl".to_owned(), true, "8.16.0_1".to_owned()),
            ],
            "the hit's own tag decides installed, not the snapshot"
        );
    }

    #[test]
    fn a_search_failure_shows_no_rows_and_keeps_the_pattern() {
        let mut model = model();
        model.set_search_and_report("vim");
        model.apply_search_results(&[], "errorXbpsExit", &["xbps-query", "2", "broken"]);
        assert_eq!(model.count, 0, "a failed search invents no row");
        assert_eq!(model.search_error_key(), "errorXbpsExit");
        assert!(
            model.search_results_active(),
            "the field still holds the pattern"
        );
    }

    #[test]
    fn a_queue_change_moves_a_search_hit_row_without_a_new_read() {
        let mut model = model();
        model.set_search_and_report("retext");
        model.apply_search_results(&[search_hit("retext", "8.0.2_1", true)], "", &[]);
        // The row now shows the state a plan left behind: the same hits, read
        // again against the live queue.
        model.store_state(
            InstalledMap::new(),
            vec![QueueItem {
                name: "retext".into(),
                action: QueueAction::Install,
            }],
        );
        model.refresh_search_rows();
        assert_eq!(model.search_row_actions(), vec!["install".to_owned()]);
    }

    #[test]
    fn installed_filter_plus_a_search_returns_only_installed_hits() {
        // The search box is one box for packages, but it must not escape the
        // filter: with Installed active, a repository hit that is not installed
        // must not take a row, install action included.
        let mut model = model();
        set_filter(&mut model, "installed");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("vimb", "4.0.0_1", false),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
        assert_eq!(model.count, 1);
    }

    #[test]
    fn updates_filter_plus_a_search_returns_only_updatable_hits() {
        let mut model = model();
        install(
            &mut model,
            &installed(),
            vec![update("vim-9.2.1031_1")],
            Vec::new(),
        );
        set_filter(&mut model, "updates");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("vimb", "4.0.0_1", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn explicit_filter_plus_a_search_returns_only_explicit_hits() {
        let mut model = model();
        set_filter(&mut model, "explicit");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn held_filter_plus_a_search_returns_only_held_hits() {
        let mut model = model();
        let mut held_installed = installed();
        held_installed.get_mut("vim").unwrap().held = true;
        install(&mut model, &held_installed, Vec::new(), Vec::new());
        set_filter(&mut model, "held");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn orphans_filter_plus_a_search_returns_only_orphan_hits() {
        let mut model = model();
        install(&mut model, &installed(), Vec::new(), vec!["vim".to_owned()]);
        set_filter(&mut model, "orphans");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn queued_filter_plus_a_search_returns_only_queued_hits() {
        let mut model = model();
        model.queue.insert("vim".to_string(), QueueAction::Install);
        set_filter(&mut model, "queued");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", true),
                search_hit("vimb", "4.0.0_1", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim"]);
    }

    #[test]
    fn the_all_filter_keeps_every_hit() {
        let mut model = model();
        set_filter(&mut model, "all");
        model.set_search_and_report("vim");
        model.apply_search_results(
            &[
                search_hit("vim", "9.2.1031_1", false),
                search_hit("vimb", "4.0.0_1", false),
                search_hit("bash", "5.3_2", true),
            ],
            "",
            &[],
        );
        assert_eq!(model.visible_names(), vec!["vim", "vimb", "bash"]);
    }

    #[test]
    fn a_zero_byte_size_reports_an_empty_field() {
        let (mut installed_sizes, repo_installed, repo_download) = sizes();
        installed_sizes.insert("vim".to_owned(), 0);
        let mut model = PackageModelRust::default();
        model.set_sizes(installed_sizes, repo_installed, repo_download);
        install_records(&mut model, &installed(), records(), Vec::new(), Vec::new());

        let [
            size,
            size_bytes,
            update,
            update_bytes,
            download,
            download_bytes,
        ] = size_roles(&model, "vim");
        assert_eq!(
            string_of(size).as_deref(),
            Some(""),
            "a zero size is an empty field, never `0 B`"
        );
        assert_eq!(bytes_of(size_bytes), Some(0));
        assert_eq!(string_of(update), None, "no update, no new size");
        assert_eq!(bytes_of(update_bytes), None);
        assert_eq!(
            string_of(download).as_deref(),
            Some("1.6 MiB"),
            "the repository archive size is what a queued install would fetch"
        );
        assert_eq!(bytes_of(download_bytes), Some(1_700_000));
    }
}
