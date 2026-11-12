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
use cxx_qt_lib::{QByteArray, QObjectMutPtr, QString, QVariant};
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
}

impl Default for PackageModelRust {
    fn default() -> Self {
        Self {
            snapshot: Vec::new(),
            installed: InstalledMap::new(),
            updates: Vec::new(),
            orphans: Vec::new(),
            rows: Vec::new(),
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
    fn snapshot(
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
    fn store_state(
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
        let size = self.installed_sizes.get(name).copied();
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

    /// Visible rows for the current filter and search.
    ///
    /// Built into a fresh vector so a caller can compare before committing: a
    /// keystroke in the search field that does not change the visible set must
    /// not reset the model, or QML loses the scroll position and selection.
    fn compute_rows(&self) -> Vec<u32> {
        let mut candidates: Vec<u32> = (0..self.snapshot.len() as u32).collect();
        // The order belongs to the whole snapshot: sorting after filtering
        // would order the visible rows by the values of the rows that survived
        // the filter, so the same package could move when a keystroke narrows
        // the list.
        match self.sort.to_string().as_str() {
            "size" => self.sort_by_size(&mut candidates),
            _ => {
                let descending = self.sort_descending;
                candidates.sort_by(|left, right| {
                    let order = self.name_of(left).cmp(self.name_of(right));
                    if descending { order.reverse() } else { order }
                });
            }
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
    fn assign_rows(&mut self, rows: Vec<u32>) {
        self.count = i32::try_from(rows.len()).unwrap_or(i32::MAX);
        self.rows = rows;
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

    /// All visible names, for the tests.
    #[cfg(test)]
    fn visible_names(&self) -> Vec<&str> {
        self.rows.iter().map(|row| self.name_of(row)).collect()
    }

    fn matches(&self, row: &u32, filter: &str, search: &str) -> bool {
        let name = self.name_of(row);
        if !search.is_empty() {
            let description = self.snapshot[*row as usize].description.to_lowercase();
            if !name.to_lowercase().contains(search) && !description.contains(search) {
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
}
