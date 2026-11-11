//! The mirror list as a flat `QAbstractItemModel`.
//!
//! CXX-Qt 0.10 has no `QAbstractListModel` wrapper, so this follows the same
//! `#[base = QAbstractItemModel]` + C++ shim recipe as [`crate::package_model`].
//!
//! Rows are built once from [`MIRRORS`] and never re-created: the row set is the
//! static list, so there is no reset path at all. A probe result rewrites the
//! `state`, `latencyMs` and `errorKey` of one row and emits `dataChanged` for
//! that row, which is what keeps the view's scroll position and selection alive
//! while a batch runs (Global Constraints: no needless copying, no needless
//! resets).
//!
//! Row `i` is `MIRRORS[i]` — [`crate::mirror_backend`] probes by index and the
//! model answers by index, so the two agree without passing URLs back and forth.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
use khaxbps_core::{MIRRORS, MirrorInfo, ProbeState};
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
        #[cxx_name = "khaxbps_mirror_model_create_index"]
        fn mirror_model_create_index(
            model: &QAbstractItemModel,
            row: i32,
            column: i32,
        ) -> QModelIndex;

        // `dataChanged` is a signal and cannot be emitted from Rust.
        #[cxx_name = "khaxbps_mirror_model_data_changed"]
        fn mirror_model_data_changed(model: Pin<&mut QAbstractItemModel>, first: i32, last: i32);
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractItemModel]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, count, READ, NOTIFY)]
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

        // Called by the mirror backend through `khaxbps_mirror_model_*`; not
        // invokable, because only a probe batch may move a row's state.
        fn set_all_running(self: Pin<&mut MirrorModel>);

        fn set_result(
            self: Pin<&mut MirrorModel>,
            row: i32,
            state_key: &QString,
            latency_ms: u32,
            error_key: &QString,
        );
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
}

/// Role ids and the names QML sees, in declaration order.
pub const ROLES: [(i32, &str); 7] = [
    (role::NAME, "name"),
    (role::URL, "url"),
    (role::REGION, "region"),
    (role::COUNTRY, "country"),
    (role::LATENCY_MS, "latencyMs"),
    (role::STATE, "state"),
    (role::ERROR_KEY, "errorKey"),
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

/// One mirror and the result of its last probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorRow {
    pub info: MirrorInfo,
    pub state: ProbeState,
}

/// Rust side of [`qobject::MirrorModel`].
pub struct MirrorModelRust {
    rows: Vec<MirrorRow>,
    count: i32,
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
        Self {
            count: i32::try_from(rows.len()).unwrap_or(i32::MAX),
            rows,
        }
    }

    fn row_at(&self, row: i32) -> Option<&MirrorRow> {
        usize::try_from(row).ok().and_then(|row| self.rows.get(row))
    }

    /// Records one row's probe result; `false` when the row already said that.
    fn assign(&mut self, row: i32, state: ProbeState) -> bool {
        let Some(entry) = usize::try_from(row)
            .ok()
            .and_then(|row| self.rows.get_mut(row))
        else {
            return false;
        };
        if entry.state == state {
            return false;
        }
        entry.state = state;
        true
    }

    /// Puts every row into `running`; `false` when they all already were.
    fn mark_all_running(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.rows {
            if entry.state != ProbeState::Running {
                entry.state = ProbeState::Running;
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
        if parent.is_valid() { 0 } else { self.count }
    }

    fn column_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { 1 }
    }

    fn data(&self, index: &cxx_qt_lib::QModelIndex, role: i32) -> cxx_qt_lib::QVariant {
        let Some(row) = self.row_at(index.row()) else {
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
        if parent.is_valid() || column != 0 || row < 0 || row >= self.count {
            return cxx_qt_lib::QModelIndex::default();
        }
        let base = <qobject::MirrorModel as Upcast<qobject::QAbstractItemModel>>::upcast(self);
        qobject::mirror_model_create_index(base, row, column)
    }

    fn parent(&self, _index: &cxx_qt_lib::QModelIndex) -> cxx_qt_lib::QModelIndex {
        cxx_qt_lib::QModelIndex::default()
    }

    /// Every row `running`, announced as one range so nothing resets.
    fn set_all_running(mut self: Pin<&mut Self>) {
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.mark_all_running()
        };
        let count = self.count;
        if changed && count > 0 {
            qobject::mirror_model_data_changed(base(self.as_mut()), 0, count - 1);
        }
    }

    /// One row's result, announced for that row only.
    fn set_result(
        mut self: Pin<&mut Self>,
        row: i32,
        state_key: &cxx_qt_lib::QString,
        latency_ms: u32,
        error_key: &cxx_qt_lib::QString,
    ) {
        let state = parse_state(&state_key.to_string(), latency_ms, &error_key.to_string());
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.assign(row, state)
        };
        if changed {
            qobject::mirror_model_data_changed(base(self.as_mut()), row, row);
        }
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
    use khaxbps_core::{DEFAULT_MIRROR, Region};

    fn mirrors() -> Vec<MirrorInfo> {
        vec![
            MirrorInfo {
                name: "Default redirect",
                url: DEFAULT_MIRROR,
                region: Region::World,
                country: "",
            },
            MirrorInfo {
                name: "Denmark",
                url: "http://ftp.dk.xemacs.org/voidlinux/",
                region: Region::Eu,
                country: "Denmark",
            },
        ]
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
    fn a_probe_result_only_touches_its_row() {
        let mut model = MirrorModelRust::new();
        assert!(model.assign(3, ProbeState::Ok { ms: 42 }));
        assert_eq!(model.row_at(3).unwrap().state, ProbeState::Ok { ms: 42 });
        assert_eq!(model.row_at(2).unwrap().state, ProbeState::Idle);
        assert_eq!(model.row_at(4).unwrap().state, ProbeState::Idle);
        assert!(
            !model.assign(3, ProbeState::Ok { ms: 42 }),
            "the same result is not a change"
        );
        assert!(model.assign(3, ProbeState::Running));
    }

    #[test]
    fn out_of_range_rows_are_ignored() {
        let mut model = MirrorModelRust::new();
        assert!(!model.assign(-1, ProbeState::Running));
        assert!(!model.assign(34, ProbeState::Running));
        assert!(!model.assign(i32::MAX, ProbeState::Running));
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
