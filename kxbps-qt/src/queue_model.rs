// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pending transactions as a `QAbstractListModel`-shaped item model.
//!
//! CXX-Qt 0.10 has no `QAbstractListModel` wrapper, so this uses the same
//! `#[base = QAbstractItemModel]` + C++ shim recipe as [`crate::package_model`]:
//! a flat, single-column model with `Qt::UserRole`-based role ids.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
use kxbps_core::QueueAction;
use qobject::{QHash_i32_QByteArray, QObjectMutPtr};
use serde::{Deserialize, Serialize};

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

        include!("model_shim.h");
        type QAbstractItemModel;

        #[cxx_name = "kxbps_queue_model_begin_reset"]
        fn queue_model_begin_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_queue_model_end_reset"]
        fn queue_model_end_reset(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_queue_model_create_index"]
        fn queue_model_create_index(
            model: &QAbstractItemModel,
            row: i32,
            column: i32,
        ) -> QModelIndex;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractItemModel]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, count, READ, NOTIFY)]
        type QueueModel = super::QueueModelRust;

        #[qinvokable]
        #[cxx_override]
        fn row_count(self: &QueueModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn column_count(self: &QueueModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &QueueModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        fn role_names(self: &QueueModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        fn index(self: &QueueModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &QueueModel, index: &QModelIndex) -> QModelIndex;

        fn set_state(self: Pin<&mut QueueModel>, queue_json: &QString);
    }

    impl
        cxx_qt::Constructor<
            (QString, QObjectMutPtr),
            BaseArguments = (QObjectMutPtr,),
            NewArguments = (QString,),
        > for QueueModel
    {
    }
}

/// Role ids handed to `data()`; QML reads the names from [`ROLES`].
pub mod role {
    /// `Qt::UserRole`; ids below it belong to Qt.
    const USER_ROLE: i32 = 256;

    pub const NAME: i32 = USER_ROLE;
    pub const ACTION: i32 = USER_ROLE + 1;
    pub const VERSION: i32 = USER_ROLE + 2;
    pub const SIZE: i32 = USER_ROLE + 3;
}

/// Role ids and the names QML sees, in declaration order.
pub const ROLES: [(i32, &str); 4] = [
    (role::NAME, "name"),
    (role::ACTION, "action"),
    (role::VERSION, "version"),
    (role::SIZE, "size"),
];

/// One queue row as the backend writes it: the action plus what the read knows
/// about the version and the bytes it moves. A queued install of a package the
/// machine does not hold has neither, and the row shows a dash.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct QueueRow {
    pub name: String,
    pub action: QueueAction,
    pub version: String,
    pub size: String,
}

impl Default for QueueRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            action: QueueAction::Install,
            version: String::new(),
            size: String::new(),
        }
    }
}

/// Rust side of [`qobject::QueueModel`].
pub struct QueueModelRust {
    entries: Vec<QueueRow>,
    count: i32,
}

impl QueueModelRust {
    /// Parses the queue JSON written by [`kxbps_core::Store`].
    fn new(queue_json: &str) -> Self {
        let entries: Vec<QueueRow> = serde_json::from_str(queue_json).unwrap_or_default();
        Self::with_entries(entries)
    }

    fn with_entries(entries: Vec<QueueRow>) -> Self {
        Self {
            count: i32::try_from(entries.len()).unwrap_or(i32::MAX),
            entries,
        }
    }

    /// Whether `entries` would change the rows.
    fn changed(&self, entries: &[QueueRow]) -> bool {
        self.entries != entries
    }

    /// Commits a new queue, keeping `count` in step.
    fn assign(&mut self, entries: Vec<QueueRow>) {
        self.count = i32::try_from(entries.len()).unwrap_or(i32::MAX);
        self.entries = entries;
    }

    fn entry_at(&self, row: i32) -> Option<&QueueRow> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.entries.get(row))
    }
}

impl cxx_qt::Constructor<(QString, QObjectMutPtr)> for qobject::QueueModel {
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
        let (queue_json, parent) = arguments;
        ((queue_json,), (parent,), ())
    }

    fn new((queue_json,): Self::NewArguments) -> QueueModelRust {
        QueueModelRust::new(&queue_json.to_string())
    }
}

impl qobject::QueueModel {
    fn row_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { self.count }
    }

    fn column_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { 1 }
    }

    fn data(&self, index: &cxx_qt_lib::QModelIndex, role: i32) -> cxx_qt_lib::QVariant {
        let Some(entry) = self.entry_at(index.row()) else {
            return cxx_qt_lib::QVariant::default();
        };
        match role {
            role::NAME => QVariant::from(&QString::from(entry.name.as_str())),
            role::ACTION => QVariant::from(&QString::from(crate::package_model::queue_key(
                entry.action,
            ))),
            role::VERSION => QVariant::from(&QString::from(entry.version.as_str())),
            role::SIZE => QVariant::from(&QString::from(entry.size.as_str())),
            _ => QVariant::default(),
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
        let base = <qobject::QueueModel as Upcast<qobject::QAbstractItemModel>>::upcast(self);
        qobject::queue_model_create_index(base, row, column)
    }

    fn parent(&self, _index: &cxx_qt_lib::QModelIndex) -> cxx_qt_lib::QModelIndex {
        cxx_qt_lib::QModelIndex::default()
    }

    fn set_state(mut self: Pin<&mut Self>, queue_json: &cxx_qt_lib::QString) {
        let entries: Vec<QueueRow> =
            serde_json::from_str(&queue_json.to_string()).unwrap_or_default();
        let changed = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.changed(&entries)
        };
        if !changed {
            return;
        }
        qobject::queue_model_begin_reset(base(self.as_mut()));
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.assign(entries);
        }
        qobject::queue_model_end_reset(base(self.as_mut()));
        self.as_mut().count_changed();
    }
}

fn base(model: Pin<&mut qobject::QueueModel>) -> Pin<&mut qobject::QAbstractItemModel> {
    <qobject::QueueModel as Upcast<qobject::QAbstractItemModel>>::upcast_pin(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kxbps_core::QueueAction;

    fn entries() -> Vec<QueueRow> {
        vec![
            QueueRow {
                name: "vim".into(),
                action: QueueAction::Install,
                version: String::new(),
                size: String::new(),
            },
            QueueRow {
                name: "ark".into(),
                action: QueueAction::Remove,
                version: "1.0_1".into(),
                size: "1.2 MiB".into(),
            },
        ]
    }

    #[test]
    fn snapshot_reports_change_only_when_the_queue_moves() {
        let mut model = QueueModelRust::with_entries(Vec::new());
        assert_eq!(model.count, 0);
        assert!(model.changed(&entries()));
        model.assign(entries());
        assert_eq!(model.count, 2);
        assert!(!model.changed(&entries()), "same queue, no reset");
        assert!(model.changed(&entries()[..1]), "a shorter queue differs");
        model.assign(entries().split_off(1));
        assert_eq!(model.count, 1);
    }

    #[test]
    fn role_table_covers_the_row() {
        assert_eq!(
            ROLES,
            [
                (256, "name"),
                (257, "action"),
                (258, "version"),
                (259, "size")
            ]
        );
    }

    /// A plain `Vec<QueueItem>` JSON (the store's own shape) still parses: the
    /// version and size default to empty, so an older file cannot break the
    /// list.
    #[test]
    fn a_queue_without_versions_still_parses() {
        let model = QueueModelRust::new(r#"[{"name":"vim","action":"update"}]"#);
        assert_eq!(model.count, 1);
        let entry = model.entry_at(0).expect("one row");
        assert_eq!(entry.name, "vim");
        assert_eq!(entry.action, QueueAction::Update);
        assert_eq!(entry.version, "");
        assert_eq!(entry.size, "");
    }

    #[test]
    fn malformed_json_yields_an_empty_model() {
        let model = QueueModelRust::new("{ not json");
        assert_eq!(model.count, 0);
        assert!(model.entry_at(0).is_none());
    }
}
