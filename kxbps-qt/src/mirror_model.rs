//! The mirror list as a flat `QAbstractItemModel`.
//!
//! CXX-Qt 0.10 has no `QAbstractListModel` wrapper, so this follows the same
//! `#[base = QAbstractItemModel]` + C++ shim recipe as [`crate::package_model`].
//!
//! Rows are built once from [`MIRRORS`] and never re-created; there is no
//! rebuild of the row data. What changes is the *visible* order: `sortDescending`
//! and `filter` recompute a permutation (`display`) over the stable row list, and
//! `rowCount`/`index`/`data` read through it, so the view can sort by latency and
//! hide failed rows without cloning a mirror. One `dataChanged` keeps the view's
//! scroll position and selection alive while a batch runs (Global Constraints:
//! no needless copying, no needless resets).
//!
//! Row identity is the *source* row, i.e. the `MIRRORS` index, and it survives
//! every permutation: [`crate::mirror_backend`] probes by source row and the
//! model answers by source row, so a sorted or filtered view still writes each
//! result into the row that holds that URL. A visible-set or order change resets
//! the model; a single result does not, unless it moves a row across the active
//! filter, in which case the reset is the only honest announcement.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
use kxbps_core::{MIRRORS, MirrorInfo, ProbeState, Region};
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
        #[cxx_name = "kxbps_mirror_model_create_index"]
        fn mirror_model_create_index(
            model: &QAbstractItemModel,
            row: i32,
            column: i32,
        ) -> QModelIndex;

        // `beginResetModel`/`endResetModel` are protected and cannot be called
        // from Rust; a filter or sort change resets the visible rows through
        // these.
        #[cxx_name = "kxbps_mirror_model_begin_reset"]
        fn mirror_model_begin_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_mirror_model_end_reset"]
        fn mirror_model_end_reset(model: Pin<&mut QAbstractItemModel>);

        // `dataChanged` is a signal and cannot be emitted from Rust.
        #[cxx_name = "kxbps_mirror_model_data_changed"]
        fn mirror_model_data_changed(model: Pin<&mut QAbstractItemModel>, first: i32, last: i32);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractItemModel]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, count, READ, NOTIFY)]
        /// Latency order: ascending (fastest first) by default. A row without a
        /// latency is last in both directions.
        #[qproperty(bool, sort_descending, READ, WRITE = set_sort_descending, NOTIFY)]
        /// State filter: `0` all (default), `1` answered (`ok`), `2` failed.
        /// An unknown value means all.
        #[qproperty(i32, filter, READ, WRITE = set_filter, NOTIFY)]
        /// Region filter: empty for every region, otherwise one of the codes
        /// `regionCodes` names. An unknown code means every region.
        #[qproperty(QString, region_filter, READ, WRITE = set_region_filter, NOTIFY)]
        /// The region codes the list knows, as a JSON array. A view builds its
        /// filter from this instead of guessing the set from the rows it can
        /// see.
        #[qproperty(QString, region_codes, READ, NOTIFY)]
        type MirrorModel = super::MirrorModelRust;

        #[qinvokable]
        #[cxx_override]
        fn row_count(self: &MirrorModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn column_count(self: &MirrorModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &MirrorModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        fn role_names(self: &MirrorModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        fn index(self: &MirrorModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &MirrorModel, index: &QModelIndex) -> QModelIndex;

        // Called by the mirror backend through `kxbps_mirror_model_*`; not
        // invokable, because only a probe batch may move a row's state.
        fn set_all_running(self: Pin<&mut MirrorModel>);

        /// Puts only the rows the filter leaves visible into `running`.
        fn set_visible_running(self: Pin<&mut MirrorModel>);

        /// The source rows the filter leaves visible, as a JSON array.
        #[qinvokable]
        fn visible_rows_json(self: &MirrorModel) -> QString;

        fn finish_running(self: Pin<&mut MirrorModel>);

        fn set_result(
            self: Pin<&mut MirrorModel>,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );

        fn set_sort_descending(self: Pin<&mut MirrorModel>, value: bool);

        fn set_filter(self: Pin<&mut MirrorModel>, value: i32);

        fn set_region_filter(self: Pin<&mut MirrorModel>, value: &QString);
    }

    // QML never creates the model: `MirrorBackend` owns it and hands it out as a
    // `QObject*`. The constructor only takes the parent, because the rows are
    // the compiled-in mirror list.
    impl cxx_qt::Constructor<(QObjectMutPtr,), BaseArguments = (QObjectMutPtr,), NewArguments = ()>
        for MirrorModel
    {
    }
}

/// Role ids handed to `data()`; QML reads the names from [`ROLES`].
///
/// Every id starts at `Qt::UserRole` (256); ids 0-255 belong to Qt.
pub mod role {
    /// `Qt::UserRole`.
    const USER_ROLE: i32 = 256;

    pub const NAME: i32 = USER_ROLE;
    pub const URL: i32 = USER_ROLE + 1;
    pub const REGION: i32 = USER_ROLE + 2;
    pub const COUNTRY: i32 = USER_ROLE + 3;
    pub const LATENCY_MS: i32 = USER_ROLE + 4;
    pub const STATE: i32 = USER_ROLE + 5;
    pub const ERROR_KEY: i32 = USER_ROLE + 6;
    pub const TIER: i32 = USER_ROLE + 7;
}

/// Role ids and the names QML sees, in declaration order.
pub const ROLES: [(i32, &str); 8] = [
    (role::NAME, "name"),
    (role::URL, "url"),
    (role::REGION, "region"),
    (role::COUNTRY, "country"),
    (role::LATENCY_MS, "latencyMs"),
    (role::STATE, "state"),
    (role::ERROR_KEY, "errorKey"),
    (role::TIER, "tier"),
];

/// The four states a row can be in, as QML sees them.
///
/// `idle` — never probed in this session; `running` — a batch is asking;
/// `ok` — the mirror answered, `latencyMs` is set; `fail` — `errorKey` names
/// the reason. The strings are stable keys, never sentences.
pub const STATE_IDLE: &str = "idle";
pub const STATE_RUNNING: &str = "running";
pub const STATE_OK: &str = "ok";
pub const STATE_FAIL: &str = "fail";

/// The region codes the list knows, as the JSON array the view reads.
fn region_codes_json() -> QString {
    let codes: Vec<&str> = Region::ALL.iter().map(|region| region.as_str()).collect();
    QString::from(serde_json::to_string(&codes).unwrap_or_else(|_| String::from("[]")))
}

/// One mirror and the result of its last probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorRow {
    pub info: MirrorInfo,
    pub state: ProbeState,
}

/// `filter` value: every row.
pub const FILTER_ALL: i32 = 0;
/// `filter` value: rows that answered.
pub const FILTER_OK: i32 = 1;
/// `filter` value: rows whose probe failed.
pub const FILTER_FAILED: i32 = 2;

/// What a row-set change did, so the caller knows how much to announce.
///
/// The model does the right thing by itself; this exists for the tests, which
/// check the behaviour (what is visible) rather than the emission calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOutcome {
    /// The state is the one the row already had.
    Unchanged,
    /// The state changed and the visible rows are the same list in the same
    /// order: one `dataChanged` covers it.
    RowChanged,
    /// The state changed and the active filter or the sort moved the rows:
    /// only a reset can describe the new visible list.
    Reset,
}

/// The `filter` value QML asked for, folded onto the three known codes.
pub const fn normalise_filter(filter: i32) -> i32 {
    match filter {
        FILTER_OK => FILTER_OK,
        FILTER_FAILED => FILTER_FAILED,
        _ => FILTER_ALL,
    }
}

/// Whether a row passes the region filter: an empty code takes every row, an
/// unknown one does too, so a stale value can never empty the list.
pub fn passes_region(region: Region, wanted: &str) -> bool {
    wanted.is_empty() || region.as_str() == wanted
}

/// Whether a row passes `filter`.
pub const fn passes_filter(state: &ProbeState, filter: i32) -> bool {
    match normalise_filter(filter) {
        FILTER_OK => matches!(state, ProbeState::Ok { .. }),
        FILTER_FAILED => matches!(state, ProbeState::Fail { .. }),
        _ => true,
    }
}

/// The visible order as a permutation of source rows.
///
/// Sort first, filter after, so a filter never changes what "fastest" means.
/// Ordering: the rows with a latency, ascending or descending; then the rows
/// without one, in both directions (an unknown latency is not "0 ms", so it is
/// neither the fastest nor the slowest), tie-broken by name and then by source
/// index, which is the order the user can predict from the list.
///
/// `states` and `mirrors` are parallel slices; a row past the end of `mirrors`
/// takes an empty name and sorts with the unnamed rows instead of panicking.
pub fn visible_order(
    states: &[ProbeState],
    mirrors: &[MirrorInfo],
    sort_descending: bool,
    filter: i32,
    region: &str,
) -> Vec<usize> {
    let name_of = |row: usize| mirrors.get(row).map_or("", |info| info.name);
    let mut order: Vec<usize> = (0..states.len())
        .filter(|row| passes_filter(&states[*row], filter))
        .filter(|row| {
            mirrors
                .get(*row)
                .is_none_or(|info| passes_region(info.region, region))
        })
        .collect();
    order.sort_by(|left, right| {
        let left_ms = latency_of(&states[*left]);
        let right_ms = latency_of(&states[*right]);
        match (left_ms > 0, right_ms > 0) {
            (true, true) => {
                let by_latency = left_ms.cmp(&right_ms);
                let by_latency = if sort_descending {
                    by_latency.reverse()
                } else {
                    by_latency
                };
                by_latency.then_with(|| (name_of(*left), *left).cmp(&(name_of(*right), *right)))
            }
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => (name_of(*left), *left).cmp(&(name_of(*right), *right)),
        }
    });
    order
}

/// Rust side of [`qobject::MirrorModel`].
pub struct MirrorModelRust {
    rows: Vec<MirrorRow>,
    count: i32,
    /// Source rows in visible order; rebuilt from `rows`, the sort direction and
    /// the filter, never reordered in place.
    display: Vec<usize>,
    sort_descending: bool,
    filter: i32,
    /// Empty for every region, otherwise one of [`Region::ALL`]'s codes.
    region_filter: QString,
    /// The codes a view can filter by, as a JSON array.
    region_codes: QString,
}

impl MirrorModelRust {
    /// The whole core mirror list, every row `idle`.
    pub fn new() -> Self {
        Self::with_mirrors(MIRRORS)
    }

    /// Builds rows from any list, so tests can use a short one.
    pub fn with_mirrors(mirrors: &[MirrorInfo]) -> Self {
        let rows: Vec<MirrorRow> = mirrors
            .iter()
            .map(|info| MirrorRow {
                info: *info,
                state: ProbeState::Idle,
            })
            .collect();
        let mut model = Self {
            count: i32::try_from(rows.len()).unwrap_or(i32::MAX),
            rows,
            display: Vec::new(),
            sort_descending: false,
            filter: FILTER_ALL,
            region_filter: QString::from(""),
            region_codes: region_codes_json(),
        };
        model.display = model.compute_display();
        model
    }

    fn row_at(&self, row: i32) -> Option<&MirrorRow> {
        usize::try_from(row).ok().and_then(|row| self.rows.get(row))
    }

    /// Recomputes the visible order from the current states.
    fn compute_display(&self) -> Vec<usize> {
        let states: Vec<ProbeState> = self.rows.iter().map(|row| row.state.clone()).collect();
        let mirrors: Vec<MirrorInfo> = self.rows.iter().map(|row| row.info).collect();
        let region = self.region_filter.to_string();
        visible_order(
            &states,
            &mirrors,
            self.sort_descending,
            self.filter,
            &region,
        )
    }

    /// Source rows in visible order, for the view and the tests.
    pub fn visible_rows(&self) -> Vec<usize> {
        self.display.clone()
    }

    /// How many rows the view shows.
    fn visible_count(&self) -> i32 {
        i32::try_from(self.display.len()).unwrap_or(i32::MAX)
    }

    /// The filter QML will see, folded onto the three known codes.
    pub const fn filter_value(&self) -> i32 {
        normalise_filter(self.filter)
    }

    /// The source row at a visible row, `None` past the end.
    fn source_of_visible(&self, row: i32) -> Option<usize> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.display.get(row).copied())
    }

    /// The visible row that currently shows a source row, `None` when hidden.
    fn visible_of_source(&self, source: usize) -> Option<usize> {
        self.display.iter().position(|row| *row == source)
    }

    /// Sets the sort direction; `true` when the visible order changed.
    pub fn set_sort_descending(&mut self, value: bool) -> bool {
        if self.sort_descending == value {
            return false;
        }
        self.sort_descending = value;
        self.recompute_display()
    }

    /// Sets the filter; `true` when the visible order changed.
    pub fn set_filter(&mut self, value: i32) -> bool {
        let value = normalise_filter(value);
        if self.filter == value {
            return false;
        }
        self.filter = value;
        self.recompute_display()
    }

    /// Sets the region filter; `true` when the visible order changed. A code
    /// the list does not know is the same as no filter at all.
    pub fn set_region_filter(&mut self, value: &str) -> bool {
        let wanted =
            if value.is_empty() || !Region::ALL.iter().any(|region| region.as_str() == value) {
                String::new()
            } else {
                value.to_owned()
            };
        if self.region_filter.to_string() == wanted {
            return false;
        }
        self.region_filter = QString::from(wanted.as_str());
        self.recompute_display()
    }

    /// The region filter the model holds, for the tests.
    pub fn region_filter_value(&self) -> String {
        self.region_filter.to_string()
    }

    fn recompute_display(&mut self) -> bool {
        let next = self.compute_display();
        if next == self.display {
            return false;
        }
        self.display = next;
        true
    }

    /// Records one source row's probe result.
    ///
    /// The visible list decides the answer: a row that was hidden and stays
    /// hidden changes nothing on screen (`RowChanged`), as does a visible row
    /// that keeps its place; a row that enters or leaves the filter, or moves in
    /// the latency order, needs a reset because the list itself is different.
    pub fn assign(&mut self, row: i32, state: ProbeState) -> AssignOutcome {
        let Some(source) = usize::try_from(row)
            .ok()
            .filter(|row| *row < self.rows.len())
        else {
            return AssignOutcome::Unchanged;
        };
        if self.rows[source].state == state {
            return AssignOutcome::Unchanged;
        }
        let was_visible = self.visible_of_source(source).is_some();
        self.rows[source].state = state;
        let region = self.region_filter.to_string();
        let passes = passes_filter(&self.rows[source].state, self.filter)
            && passes_region(self.rows[source].info.region, &region);
        let moved = self.recompute_display();
        if moved && was_visible != passes {
            // The row entered or left the filter: the visible list itself changed.
            return AssignOutcome::Reset;
        }
        if moved && was_visible && passes {
            // Same rows, new place in the latency order.
            return AssignOutcome::Reset;
        }
        AssignOutcome::RowChanged
    }

    /// Puts the visible rows into `running`; `false` when they all already were.
    ///
    /// A probe batch covers what the user can see: a row hidden by the region
    /// or state filter would otherwise sit on "probing" with nothing to finish
    /// it.
    pub fn mark_visible_running(&mut self) -> bool {
        let mut changed = false;
        for source in self.display.clone() {
            if self.rows[source].state != ProbeState::Running {
                self.rows[source].state = ProbeState::Running;
                changed = true;
            }
        }
        changed
    }

    /// Puts every row into `running`; `false` when they all already were.
    pub fn mark_all_running(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.rows {
            if entry.state != ProbeState::Running {
                entry.state = ProbeState::Running;
                changed = true;
            }
        }
        changed
    }

    /// Returns the rows a cancelled batch left `running` to `idle`; rows that
    /// already have a result keep it, so nothing measured is thrown away.
    pub fn clear_running(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.rows {
            if entry.state == ProbeState::Running {
                entry.state = ProbeState::Idle;
                changed = true;
            }
        }
        changed
    }
}

impl Default for MirrorModelRust {
    fn default() -> Self {
        Self::new()
    }
}

impl cxx_qt::Constructor<(QObjectMutPtr,)> for qobject::MirrorModel {
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
        let (parent,) = arguments;
        ((), (parent,), ())
    }

    fn new(_: Self::NewArguments) -> MirrorModelRust {
        MirrorModelRust::new()
    }
}

impl qobject::MirrorModel {
    fn row_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() {
            0
        } else {
            i32::try_from(self.display.len()).unwrap_or(i32::MAX)
        }
    }

    fn column_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { 1 }
    }

    fn data(&self, index: &cxx_qt_lib::QModelIndex, role: i32) -> cxx_qt_lib::QVariant {
        let Some(source) = self.source_of_visible(index.row()) else {
            return cxx_qt_lib::QVariant::default();
        };
        let Some(row) = self.row_at(i32::try_from(source).unwrap_or(i32::MAX)) else {
            return cxx_qt_lib::QVariant::default();
        };
        match role {
            role::NAME => text(row.info.name),
            role::URL => text(row.info.url),
            role::REGION => text(row.info.region.as_str()),
            role::COUNTRY => text(row.info.country),
            role::LATENCY_MS => integer(latency_of(&row.state)),
            role::STATE => text(state_name(&row.state)),
            role::ERROR_KEY => text(error_name(&row.state)),
            role::TIER => integer(u32::from(row.info.tier)),
            _ => cxx_qt_lib::QVariant::default(),
        }
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
        if parent.is_valid()
            || column != 0
            || row < 0
            || usize::try_from(row).map_or(true, |row| row >= self.display.len())
        {
            return cxx_qt_lib::QModelIndex::default();
        }
        let base = <qobject::MirrorModel as Upcast<qobject::QAbstractItemModel>>::upcast(self);
        qobject::mirror_model_create_index(base, row, column)
    }

    fn parent(&self, _index: &cxx_qt_lib::QModelIndex) -> cxx_qt_lib::QModelIndex {
        cxx_qt_lib::QModelIndex::default()
    }

    /// Every row `running`, announced as one range over the visible rows so
    /// nothing resets; a running row never leaves a state filter, because
    /// `running` is neither `ok` nor `fail`, so the visible set is unchanged
    /// only when the filter is `all` — otherwise the list legitimately shrinks
    /// and the caller gets a reset.
    fn visible_rows_json(&self) -> cxx_qt_lib::QString {
        cxx_qt_lib::QString::from(
            serde_json::to_string(&self.visible_rows()).unwrap_or_else(|_| String::from("[]")),
        )
    }

    fn set_visible_running(mut self: Pin<&mut Self>) {
        let (changed, reset, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let changed = data.mark_visible_running();
            let reset = changed && data.recompute_display();
            (changed, reset, data.visible_count())
        };
        if changed {
            reset_visible(base(self.as_mut()), reset, rows);
        }
    }

    fn set_all_running(mut self: Pin<&mut Self>) {
        let (changed, reset, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let changed = data.mark_all_running();
            let reset = changed && data.recompute_display();
            (changed, reset, data.visible_count())
        };
        if changed {
            reset_visible(base(self.as_mut()), reset, rows);
        }
    }

    /// Ends a batch that stopped before its last row: the unanswered rows go back
    /// to `idle`, so a cancelled sweep leaves no spinner behind.
    fn finish_running(mut self: Pin<&mut Self>) {
        let (changed, reset, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let changed = data.clear_running();
            let reset = changed && data.recompute_display();
            (changed, reset, data.visible_count())
        };
        if changed {
            reset_visible(base(self.as_mut()), reset, rows);
        }
    }

    /// One row's result, announced for that source row only.
    ///
    /// `row` is the `MIRRORS` index, not a visible one: the permutation may put
    /// the mirror anywhere, and a result must reach the mirror it measured. When
    /// the result moves the row across the active filter, or the latency order
    /// changes, the reset is the only announcement that describes the new list.
    fn set_result(
        mut self: Pin<&mut Self>,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        let state = parse_state(&state_key.to_string(), latency_ms, &error_key.to_string());
        let (outcome, visible, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let outcome = data.assign(row, state);
            let visible = usize::try_from(row)
                .ok()
                .and_then(|row| data.visible_of_source(row));
            (outcome, visible, data.visible_count())
        };
        match outcome {
            AssignOutcome::Unchanged => {}
            AssignOutcome::Reset => reset_visible(base(self.as_mut()), true, rows),
            AssignOutcome::RowChanged => {
                if let Some(visible) = visible {
                    let visible = i32::try_from(visible).unwrap_or(i32::MAX);
                    qobject::mirror_model_data_changed(base(self.as_mut()), visible, visible);
                }
            }
        }
    }

    /// Flips the latency order and resets when the visible list moved.
    fn set_sort_descending(mut self: Pin<&mut Self>, value: bool) {
        let (moved, changed, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // Compare the stored value before and after: the write function
            // assigns the field itself, so testing `self.sort_descending` here
            // would always find it already equal and never notify. Without the
            // signal the heading's arrow and its accessible name kept saying
            // "ascending" while the list was sorted the other way.
            let before = data.sort_descending;
            let moved = data.set_sort_descending(value);
            (moved, data.sort_descending != before, data.visible_count())
        };
        if changed {
            self.as_mut().sort_descending_changed();
        }
        if moved {
            reset_visible(base(self.as_mut()), true, rows);
        }
    }

    /// Changes the region filter and resets when the visible list moved.
    ///
    /// The reset is not optional: without it the view keeps the rows it
    /// already built, so a narrowed list still paints the old count and the
    /// delegates past the new `rowCount` come out empty.
    fn set_region_filter(mut self: Pin<&mut Self>, value: &cxx_qt_lib::QString) {
        let text = value.to_string();
        let (changed, moved, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            let before = data.region_filter.to_string();
            let moved = data.set_region_filter(&text);
            (
                before != data.region_filter.to_string(),
                moved,
                data.visible_count(),
            )
        };
        if changed {
            self.as_mut().region_filter_changed();
        }
        if moved {
            reset_visible(base(self.as_mut()), true, rows);
        }
    }

    fn set_filter(mut self: Pin<&mut Self>, value: i32) {
        let (changed, moved, rows) = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            // Compare the stored value before and after, not with the raw input:
            // an unknown code normalises to `all`, and a property that stays
            // `all` must not notify.
            let before = data.filter;
            let changed = data.set_filter(value);
            (changed, data.filter != before, data.visible_count())
        };
        if moved {
            self.as_mut().filter_changed();
        }
        if changed {
            reset_visible(base(self.as_mut()), true, rows);
        }
    }
}

/// One model reset around a visible-set change, or a `dataChanged` over the
/// whole visible range when only the row data moved.
///
/// `rows` is the visible row count, which the caller reads from the data before
/// it hands the pin over: the base `QAbstractItemModel` does not expose this
/// model's own `rowCount`.
fn reset_visible(mut model: Pin<&mut qobject::QAbstractItemModel>, reset: bool, rows: i32) {
    if reset {
        qobject::mirror_model_begin_reset(model.as_mut());
        qobject::mirror_model_end_reset(model);
        return;
    }
    if rows > 0 {
        qobject::mirror_model_data_changed(model, 0, rows - 1);
    }
}

fn base(model: Pin<&mut qobject::MirrorModel>) -> Pin<&mut qobject::QAbstractItemModel> {
    <qobject::MirrorModel as Upcast<qobject::QAbstractItemModel>>::upcast_pin(model)
}

/// The wire name of a state, as the `state` role reports it.
pub const fn state_name(state: &ProbeState) -> &'static str {
    match state {
        ProbeState::Idle => STATE_IDLE,
        ProbeState::Running => STATE_RUNNING,
        ProbeState::Ok { .. } => STATE_OK,
        ProbeState::Fail { .. } => STATE_FAIL,
    }
}

/// Latency in milliseconds; `0` for every state that has none.
pub fn latency_of(state: &ProbeState) -> u32 {
    match state {
        ProbeState::Ok { ms } => u32::try_from(*ms).unwrap_or(u32::MAX),
        _ => 0,
    }
}

/// The core probe key of a failed row, `""` otherwise.
pub fn error_name(state: &ProbeState) -> &str {
    match state {
        ProbeState::Fail { error } => error.as_str(),
        _ => "",
    }
}

/// Builds a state from the three wire values the backend hands over.
pub fn parse_state(key: &str, latency_ms: u32, error_key: &str) -> ProbeState {
    match key {
        STATE_RUNNING => ProbeState::Running,
        STATE_OK => ProbeState::Ok {
            ms: u64::from(latency_ms),
        },
        STATE_FAIL => ProbeState::Fail {
            error: error_key.to_string(),
        },
        _ => ProbeState::Idle,
    }
}

fn text(value: &str) -> QVariant {
    QVariant::from(&QString::from(value))
}

fn integer(value: u32) -> QVariant {
    QVariant::from(&i32::try_from(value).unwrap_or(i32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::{DEFAULT_MIRROR, Region};

    fn mirrors() -> Vec<MirrorInfo> {
        vec![
            MirrorInfo {
                name: "Default redirect",
                url: DEFAULT_MIRROR,
                region: Region::World,
                country: "",
                tier: 1,
            },
            MirrorInfo {
                name: "Denmark",
                url: "http://ftp.dk.xemacs.org/voidlinux/",
                region: Region::Eu,
                country: "Denmark",
                tier: 2,
            },
        ]
    }

    #[test]
    fn a_region_filter_keeps_only_that_region() {
        let mut model = MirrorModelRust::with_mirrors(&mirrors());
        assert_eq!(model.visible_rows(), [0, 1], "both rows at first");

        assert!(model.set_region_filter("EU"));
        assert_eq!(model.visible_rows(), [1], "only Denmark is in Europe");
        assert_eq!(model.region_filter_value(), "EU");

        // The code it already holds moves nothing: a repeated write must not
        // reset the view.
        assert!(!model.set_region_filter("EU"));

        // A code the list does not know is the same as no filter: the combo can
        // only offer the codes the model publishes, so a stale one must not
        // empty the list.
        assert!(model.set_region_filter("Atlantis"));
        assert_eq!(model.visible_rows(), [0, 1]);
        assert_eq!(model.region_filter_value(), "");
    }

    #[test]
    fn the_region_and_state_filters_apply_together() {
        let mut model = MirrorModelRust::with_mirrors(&mirrors());
        model.assign(
            1,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        );
        model.set_region_filter("EU");
        model.set_filter(FILTER_OK);
        assert_eq!(
            model.visible_rows(),
            Vec::<usize>::new(),
            "Denmark is the only European row and it failed"
        );

        model.set_filter(FILTER_FAILED);
        assert_eq!(model.visible_rows(), [1]);
    }

    #[test]
    fn role_table_starts_at_user_role() {
        assert_eq!(
            ROLES,
            [
                (256, "name"),
                (257, "url"),
                (258, "region"),
                (259, "country"),
                (260, "latencyMs"),
                (261, "state"),
                (262, "errorKey"),
                (263, "tier"),
            ]
        );
    }

    #[test]
    fn rows_follow_the_core_mirror_list() {
        let model = MirrorModelRust::new();
        assert_eq!(model.count, 34, "every core mirror is a row");
        for (row, mirror) in MIRRORS.iter().enumerate() {
            let entry = model.row_at(i32::try_from(row).unwrap()).unwrap();
            assert_eq!(entry.info, *mirror, "row {row} is MIRRORS[{row}]");
            assert_eq!(entry.state, ProbeState::Idle);
        }
    }

    #[test]
    fn an_unprobed_list_sorts_by_name() {
        // Before the first probe every row is `idle`, so the latency sort falls
        // back to its name tie-break: the user sees a predictable order instead
        // of the compiled-in one.
        let model = MirrorModelRust::new();
        let names: Vec<&str> = model
            .visible_rows()
            .into_iter()
            .map(|row| model.row_at(i32::try_from(row).unwrap()).unwrap().info.name)
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert!(
            model.visible_rows().iter().any(|row| *row != 0),
            "the visible order is not the source order"
        );
    }

    #[test]
    fn a_probe_result_only_touches_its_row() {
        let mut model = MirrorModelRust::new();
        assert_eq!(
            model.assign(3, ProbeState::Ok { ms: 42 }),
            AssignOutcome::Reset,
            "the first latency moves the row in the name-sorted list"
        );
        assert_eq!(model.row_at(3).unwrap().state, ProbeState::Ok { ms: 42 });
        assert_eq!(model.row_at(2).unwrap().state, ProbeState::Idle);
        assert_eq!(model.row_at(4).unwrap().state, ProbeState::Idle);
        assert_eq!(
            model.assign(3, ProbeState::Ok { ms: 42 }),
            AssignOutcome::Unchanged,
            "the same result is not a change"
        );
        assert_eq!(
            model.assign(3, ProbeState::Running),
            AssignOutcome::Reset,
            "the latency is gone, so the row moves back into the name order"
        );
    }

    #[test]
    fn out_of_range_rows_are_ignored() {
        let mut model = MirrorModelRust::new();
        assert_eq!(
            model.assign(-1, ProbeState::Running),
            AssignOutcome::Unchanged
        );
        assert_eq!(
            model.assign(34, ProbeState::Running),
            AssignOutcome::Unchanged
        );
        assert_eq!(
            model.assign(i32::MAX, ProbeState::Running),
            AssignOutcome::Unchanged
        );
        assert!(model.row_at(-1).is_none());
    }

    #[test]
    fn a_batch_marks_every_row_running_once() {
        let mut model = MirrorModelRust::with_mirrors(&mirrors());
        assert!(model.mark_all_running());
        assert!(
            model
                .rows
                .iter()
                .all(|row| row.state == ProbeState::Running)
        );
        assert!(
            !model.mark_all_running(),
            "a second pass changes nothing, so it emits nothing"
        );
    }

    #[test]
    fn a_cancelled_batch_returns_the_running_rows_to_idle() {
        let mut model = MirrorModelRust::with_mirrors(&mirrors());
        assert!(model.mark_all_running());
        assert!(model.clear_running());
        assert!(
            model.rows.iter().all(|row| row.state == ProbeState::Idle),
            "no row is left running after a cancelled batch"
        );
        assert!(
            !model.clear_running(),
            "nothing is left running, so nothing changes"
        );
    }

    #[test]
    fn finishing_a_batch_keeps_the_results_it_already_has() {
        let mut model = MirrorModelRust::with_mirrors(&mirrors());
        assert_eq!(
            model.assign(1, ProbeState::Ok { ms: 55 }),
            AssignOutcome::Reset,
            "the measured row moves to the top of a two-row list"
        );
        assert_eq!(
            model.assign(0, ProbeState::Running),
            AssignOutcome::RowChanged,
            "it had no latency and still has none, so its name keeps the place"
        );
        assert!(model.clear_running());
        assert_eq!(
            model.row_at(1).unwrap().state,
            ProbeState::Ok { ms: 55 },
            "a measured latency survives the cancellation"
        );
        assert_eq!(model.row_at(0).unwrap().state, ProbeState::Idle);
    }

    #[test]
    fn state_names_and_latencies_match_the_roles() {
        assert_eq!(state_name(&ProbeState::Idle), "idle");
        assert_eq!(state_name(&ProbeState::Running), "running");
        assert_eq!(state_name(&ProbeState::Ok { ms: 7 }), "ok");
        assert_eq!(
            state_name(&ProbeState::Fail {
                error: "timeout".into()
            }),
            "fail"
        );
        assert_eq!(latency_of(&ProbeState::Ok { ms: 7 }), 7);
        assert_eq!(latency_of(&ProbeState::Idle), 0);
        assert_eq!(
            latency_of(&ProbeState::Fail {
                error: "timeout".into()
            }),
            0
        );
        assert_eq!(error_name(&ProbeState::Idle), "");
        assert_eq!(
            error_name(&ProbeState::Fail {
                error: "plainHttp".into()
            }),
            "plainHttp"
        );
    }

    #[test]
    fn parse_state_round_trips_the_wire_values() {
        assert_eq!(parse_state("running", 0, ""), ProbeState::Running);
        assert_eq!(parse_state("ok", 123, ""), ProbeState::Ok { ms: 123 });
        assert_eq!(
            parse_state("fail", 0, "noResponse"),
            ProbeState::Fail {
                error: "noResponse".into()
            }
        );
        assert_eq!(parse_state("idle", 5, "ignored"), ProbeState::Idle);
        assert_eq!(parse_state("nonsense", 0, ""), ProbeState::Idle);
    }

    /// Three mirrors with a known name order: `Alpha`, `Bravo`, `Charlie`.
    fn ordered() -> Vec<MirrorInfo> {
        vec![
            MirrorInfo {
                name: "Alpha",
                url: "https://alpha.example/voidlinux/",
                region: Region::Eu,
                country: "Alpha",
                tier: 2,
            },
            MirrorInfo {
                name: "Bravo",
                url: "https://bravo.example/voidlinux/",
                region: Region::Eu,
                country: "Bravo",
                tier: 2,
            },
            MirrorInfo {
                name: "Charlie",
                url: "https://charlie.example/voidlinux/",
                region: Region::Eu,
                country: "Charlie",
                tier: 2,
            },
        ]
    }

    /// The visible mirror names, which is what the view paints.
    fn visible_names(model: &MirrorModelRust) -> Vec<&str> {
        model
            .visible_rows()
            .into_iter()
            .map(|row| model.row_at(i32::try_from(row).unwrap()).unwrap().info.name)
            .collect()
    }

    #[test]
    fn sorting_by_latency_puts_the_fastest_first_and_unknown_last() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(2, ProbeState::Ok { ms: 80 });
        model.assign(0, ProbeState::Ok { ms: 20 });
        // Row 1 is never probed: no latency, so it sorts last.
        assert_eq!(visible_names(&model), ["Alpha", "Charlie", "Bravo"]);
        assert_eq!(model.visible_rows(), [0, 2, 1]);
    }

    #[test]
    fn unknown_latency_stays_last_in_both_directions() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(2, ProbeState::Ok { ms: 80 });
        model.assign(0, ProbeState::Ok { ms: 20 });
        model.set_sort_descending(true);
        assert_eq!(
            visible_names(&model),
            ["Charlie", "Alpha", "Bravo"],
            "the measured rows flip, the unmeasured one keeps its place at the end"
        );
        model.set_sort_descending(false);
        assert_eq!(visible_names(&model), ["Alpha", "Charlie", "Bravo"]);
    }

    #[test]
    fn equal_latencies_fall_back_to_name_then_to_the_source_index() {
        let mirrors = vec![
            MirrorInfo {
                name: "Zulu",
                url: "https://zulu.example/voidlinux/",
                region: Region::Eu,
                country: "Zulu",
                tier: 2,
            },
            MirrorInfo {
                name: "Alpha",
                url: "https://alpha-2.example/voidlinux/",
                region: Region::Eu,
                country: "Alpha",
                tier: 2,
            },
            MirrorInfo {
                name: "Alpha",
                url: "https://alpha-1.example/voidlinux/",
                region: Region::Eu,
                country: "Alpha",
                tier: 2,
            },
        ];
        let mut model = MirrorModelRust::with_mirrors(&mirrors);
        for row in 0..3 {
            model.assign(row, ProbeState::Ok { ms: 10 });
        }
        // Same latency: `Alpha` before `Zulu`; the two `Alpha` rows keep their
        // source order, which is the only tie-break a user can predict.
        assert_eq!(model.visible_rows(), [1, 2, 0]);
        model.set_sort_descending(true);
        assert_eq!(
            model.visible_rows(),
            [1, 2, 0],
            "a full tie is independent of the direction, name still wins"
        );
    }

    #[test]
    fn the_state_filter_runs_after_the_sort() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(2, ProbeState::Ok { ms: 80 });
        model.assign(0, ProbeState::Ok { ms: 20 });
        model.assign(
            1,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        );
        assert_eq!(
            model.visible_rows(),
            [0, 2, 1],
            "the two measured rows by latency, then the failed one by name"
        );

        model.set_filter(FILTER_OK);
        assert_eq!(
            model.visible_rows(),
            [0, 2],
            "only the answered rows, fastest first"
        );

        model.set_filter(FILTER_FAILED);
        assert_eq!(model.visible_rows(), [1]);

        model.set_filter(FILTER_ALL);
        assert_eq!(model.visible_rows(), [0, 2, 1]);
    }

    #[test]
    fn a_filter_change_never_alters_a_stored_latency() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(0, ProbeState::Ok { ms: 20 });
        model.assign(2, ProbeState::Ok { ms: 80 });
        model.set_filter(FILTER_FAILED);
        assert_eq!(model.visible_rows(), Vec::<usize>::new());
        assert_eq!(
            model.row_at(0).unwrap().state,
            ProbeState::Ok { ms: 20 },
            "a hidden row keeps its measurement"
        );
        assert_eq!(model.row_at(2).unwrap().state, ProbeState::Ok { ms: 80 });
        model.set_filter(FILTER_ALL);
        assert_eq!(visible_names(&model), ["Alpha", "Charlie", "Bravo"]);
    }

    #[test]
    fn a_source_row_below_is_the_same_mirror_after_a_permutation() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(2, ProbeState::Ok { ms: 80 });
        model.assign(0, ProbeState::Ok { ms: 20 });
        // Visible row 0 is source row 0 (`Alpha`, 20 ms); a result written by
        // source row must land in the visible row the permutation names.
        assert_eq!(model.visible_rows()[0], 0);
        assert_eq!(
            model.assign(2, ProbeState::Ok { ms: 5 }),
            AssignOutcome::Reset,
            "the latency order changed, so only a reset describes it"
        );
        assert_eq!(
            model.visible_rows(),
            [2, 0, 1],
            "Charlie is now the fastest"
        );
        assert_eq!(
            model.row_at(2).unwrap().info.name,
            "Charlie",
            "the source row is still Charlie"
        );
        assert_eq!(latency_of(&model.row_at(0).unwrap().state), 20);
    }

    #[test]
    fn a_hidden_row_that_becomes_visible_changes_the_visible_set() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        model.assign(
            1,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        );
        model.set_filter(FILTER_OK);
        assert_eq!(
            model.visible_rows(),
            Vec::<usize>::new(),
            "no row answered yet, so the filter hides the whole list"
        );
        // The failed row answers after a retry: it must enter the filtered view.
        let before = model.visible_rows();
        assert_eq!(
            model.assign(1, ProbeState::Ok { ms: 10 }),
            AssignOutcome::Reset,
            "a hidden row that becomes visible changes the visible set"
        );
        assert_ne!(model.visible_rows(), before, "the visible set changed");
        assert_eq!(
            model.visible_rows(),
            [1],
            "the retried row is the only one that answered"
        );
    }

    #[test]
    fn a_result_outside_the_active_filter_leaves_the_visible_set_alone() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        // Two rows already fail, so the failed filter shows them both.
        model.assign(
            1,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        );
        model.assign(
            2,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        );
        model.set_filter(FILTER_FAILED);
        assert_eq!(model.visible_rows(), [1, 2]);
        // A hidden row fails again with the same key: nothing on screen moves.
        assert_eq!(
            model.assign(0, ProbeState::Ok { ms: 40 }),
            AssignOutcome::RowChanged,
            "row 0 was hidden and stays hidden"
        );
        assert_eq!(model.visible_rows(), [1, 2]);
    }

    #[test]
    fn the_visible_order_is_a_pure_function_of_states_sort_and_filter() {
        let mirrors = ordered();
        let states = [
            ProbeState::Ok { ms: 30 },
            ProbeState::Idle,
            ProbeState::Fail {
                error: "timeout".into(),
            },
        ];
        assert_eq!(
            visible_order(&states, &mirrors, false, FILTER_ALL, ""),
            [0, 1, 2]
        );
        assert_eq!(visible_order(&states, &mirrors, false, FILTER_OK, ""), [0]);
        assert_eq!(
            visible_order(&states, &mirrors, false, FILTER_FAILED, ""),
            [2],
            "only the failed row passes; an idle row is not a failure"
        );
        assert_eq!(
            visible_order(&states, &mirrors, true, FILTER_ALL, ""),
            [0, 1, 2],
            "one measured row, the two unmeasured ones by name"
        );
    }

    #[test]
    fn the_filter_is_all_by_default_and_ignores_unknown_codes() {
        let mut model = MirrorModelRust::with_mirrors(&ordered());
        assert_eq!(model.filter_value(), FILTER_ALL);
        model.set_filter(9);
        assert_eq!(
            model.filter_value(),
            FILTER_ALL,
            "an unknown code means all"
        );
    }

    #[test]
    fn a_row_keeps_the_core_fields_of_its_mirror() {
        let model = MirrorModelRust::with_mirrors(&mirrors());
        let entry = model.row_at(1).unwrap();
        assert_eq!(entry.info.name, "Denmark");
        assert_eq!(entry.info.region.as_str(), "EU");
        assert_eq!(entry.info.country, "Denmark");
        assert!(entry.info.url.starts_with("http://"));
        assert_eq!(model.row_at(2), None);
    }
}
