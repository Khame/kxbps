//! Append-only log lines for the job dialog.
//!
//! One role, `line`, plus a `count` property for the header. Rows are appended
//! incrementally (`beginInsertRows` / `endInsertRows`), so the dialog keeps its
//! scroll position and only the new line is materialised.

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt::casting::Upcast;
use cxx_qt_lib::{QByteArray, QString, QVariant};
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

        include!("model_shim.h");
        type QAbstractItemModel;

        #[cxx_name = "kxbps_log_model_begin_insert_rows"]
        fn log_model_begin_insert_rows(model: Pin<&mut QAbstractItemModel>, first: i32, last: i32);

        #[cxx_name = "kxbps_log_model_end_insert_rows"]
        fn log_model_end_insert_rows(model: Pin<&mut QAbstractItemModel>);

        #[cxx_name = "kxbps_log_model_create_index"]
        fn log_model_create_index(model: &QAbstractItemModel, row: i32, column: i32)
        -> QModelIndex;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractItemModel]
        #[qml_element]
        #[qml_uncreatable]
        #[qproperty(i32, count, READ, NOTIFY)]
        type LogModel = super::LogModelRust;

        #[qinvokable]
        #[cxx_override]
        fn row_count(self: &LogModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn column_count(self: &LogModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &LogModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        fn role_names(self: &LogModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        fn index(self: &LogModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &LogModel, index: &QModelIndex) -> QModelIndex;

        fn append(self: Pin<&mut LogModel>, line: &QString);

        /// Every line, newline-separated, for the clipboard. The log is read
        /// as text by people, and a list of rows cannot be selected.
        #[qinvokable]
        fn joined_text(self: &LogModel) -> QString;
    }

    impl cxx_qt::Constructor<()> for LogModel {}
}

/// Role ids handed to `data()`; QML reads the name from [`ROLES`].
pub mod role {
    /// `Qt::UserRole`; ids below it belong to Qt.
    const USER_ROLE: i32 = 256;

    pub const LINE: i32 = USER_ROLE;
}

/// Role ids and the names QML sees.
pub const ROLES: [(i32, &str); 1] = [(role::LINE, "line")];

impl cxx_qt::Constructor<()> for qobject::LogModel {
    type BaseArguments = ();
    type NewArguments = ();
    type InitializeArguments = ();

    fn route_arguments(
        _: (),
    ) -> (
        Self::NewArguments,
        Self::BaseArguments,
        Self::InitializeArguments,
    ) {
        ((), (), ())
    }

    fn new(_: Self::NewArguments) -> LogModelRust {
        LogModelRust::default()
    }
}

/// Rust side of [`qobject::LogModel`].
#[derive(Default)]
pub struct LogModelRust {
    lines: Vec<String>,
    count: i32,
}

impl LogModelRust {
    /// Appends one line and returns its row.
    fn push(&mut self, line: &str) -> usize {
        self.lines.push(line.to_string());
        self.count = i32::try_from(self.lines.len()).unwrap_or(i32::MAX);
        self.lines.len() - 1
    }

    /// Every line, newline-separated: what "copy" hands to the clipboard.
    fn joined(&self) -> String {
        self.lines.join("\n")
    }

    fn line_at(&self, row: i32) -> Option<&str> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.lines.get(row))
            .map(String::as_str)
    }
}

impl qobject::LogModel {
    fn row_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { self.count }
    }

    fn column_count(&self, parent: &cxx_qt_lib::QModelIndex) -> i32 {
        if parent.is_valid() { 0 } else { 1 }
    }

    fn data(&self, index: &cxx_qt_lib::QModelIndex, role: i32) -> cxx_qt_lib::QVariant {
        if role != role::LINE {
            return cxx_qt_lib::QVariant::default();
        }
        match self.line_at(index.row()) {
            Some(line) => QVariant::from(&QString::from(line)),
            None => QVariant::default(),
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
        let base = <qobject::LogModel as Upcast<qobject::QAbstractItemModel>>::upcast(self);
        qobject::log_model_create_index(base, row, column)
    }

    fn parent(&self, _index: &cxx_qt_lib::QModelIndex) -> cxx_qt_lib::QModelIndex {
        cxx_qt_lib::QModelIndex::default()
    }

    fn joined_text(&self) -> cxx_qt_lib::QString {
        cxx_qt_lib::QString::from(&self.joined())
    }

    fn append(mut self: Pin<&mut Self>, line: &cxx_qt_lib::QString) {
        let text = line.to_string();
        let row = {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            i32::try_from(data.lines.len()).unwrap_or(i32::MAX)
        };
        qobject::log_model_begin_insert_rows(base(self.as_mut()), row, row);
        {
            let data = Pin::get_mut(self.as_mut().rust_mut());
            data.push(&text);
        }
        qobject::log_model_end_insert_rows(base(self.as_mut()));
        self.as_mut().count_changed();
    }
}

fn base(model: Pin<&mut qobject::LogModel>) -> Pin<&mut qobject::QAbstractItemModel> {
    <qobject::LogModel as Upcast<qobject::QAbstractItemModel>>::upcast_pin(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_appends_and_counts() {
        let mut model = LogModelRust::default();
        assert_eq!(model.count, 0);
        assert_eq!(model.push("[*] Verifying package integrity..."), 0);
        assert_eq!(model.push("vim-9.1.1700_1: installed successfully."), 1);
        assert_eq!(model.count, 2);
        assert_eq!(
            model.line_at(1),
            Some("vim-9.1.1700_1: installed successfully.")
        );
        assert_eq!(model.line_at(2), None);
    }

    #[test]
    fn joined_text_is_every_line_newline_separated() {
        let mut model = LogModelRust::default();
        assert_eq!(model.joined(), "", "an empty log copies as nothing");
        model.push("$ xbps-install -Su");
        assert_eq!(model.joined(), "$ xbps-install -Su");
        model.push("2 packages will be installed.");
        assert_eq!(
            model.joined(),
            "$ xbps-install -Su\n2 packages will be installed."
        );
    }

    #[test]
    fn role_table_has_the_line_role_at_user_role() {
        assert_eq!(ROLES, [(256, "line")]);
    }
}
