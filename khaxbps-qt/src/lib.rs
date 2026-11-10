//! QML-facing bridge for Khaxbps.
//!
//! The QML module `org.kde.khaxbps` is declared in `build.rs`; every `#[qml_element]`
//! QObject in this file is registered into that module by `qmltyperegistrar`, so QML
//! only needs `import org.kde.khaxbps`.

// Keep the Quick Controls extras crate in the link graph: its initializer archive is
// only pulled in by the linker when some Rust code references the crate.
#[allow(unused_imports)]
use cxx_qt_lib_extras as _;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    // `auto_cxx_name` turns `update_count` into the QML-facing `updateCount`;
    // without it CXX-Qt exposes the raw snake_case name to QML.
    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, update_count)]
        type AppBackend = super::AppBackendRust;

        #[qinvokable]
        fn version(self: &AppBackend) -> QString;
    }
}

/// Rust side of [`qobject::AppBackend`].
#[derive(Default)]
pub struct AppBackendRust {
    update_count: i32,
}

impl qobject::AppBackend {
    /// Application version shown in the UI; proves Rust -> QML calls work.
    fn version(&self) -> cxx_qt_lib::QString {
        cxx_qt_lib::QString::from("0.1.0")
    }
}

/// Registers the `org.kde.khaxbps` QML module (types and QML resources).
///
/// Must run before a `QQmlApplicationEngine` resolves any `qrc:/qt/qml/...` URL.
/// In CXX-Qt 0.10 this macro replaces the `qml_register_type` call of the 0.7-0.9 API.
pub fn init() {
    cxx_qt::init_qml_module!("org.kde.khaxbps");
}
