//! QML-facing bridge for Khaxbps.
//!
//! The QML module `org.kde.khaxbps` is declared in `build.rs`; every `#[qml_element]`
//! QObject in this file is registered into that module by `qmltyperegistrar`, so QML
//! only needs `import org.kde.khaxbps`.

// Keep the Quick Controls extras crate in the link graph: its initializer archive is
// only pulled in by the linker when some Rust code references the crate.
#[allow(unused_imports)]
use cxx_qt_lib_extras as _;

use std::pin::Pin;

pub mod catalog_backend;
pub mod installed_backend;
pub mod job_backend;
pub mod log_model;
pub mod mirror_backend;
pub mod mirror_model;
pub mod package_model;
pub mod plan_backend;
pub mod queue_model;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "C++" {
        // Shared by every backend that needs the embedded catalog; cxx cannot
        // declare one extern function in two bridges, so the crate-root bridge
        // owns the only declaration and the others call it from Rust.
        include!("model_shim.h");

        #[cxx_name = "khaxbps_read_resource"]
        fn read_resource(path: &QString) -> QString;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlengine.h");
        type QQmlEngine = cxx_qt_lib::QQmlEngine;

        // Implemented in `src/k18n_shim.cpp`; `Pin<&mut T>` is passed as `T&`.
        // Without this the QML `i18nd()` helpers are undefined: they are
        // `Q_INVOKABLE` members of the `KLocalizedContext` root context object.
        include!("k18n_shim.h");

        #[cxx_name = "khaxbps_setup_i18n"]
        fn setup_i18n(engine: Pin<&mut QQmlEngine>);
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

/// Location of the generated package catalog inside the QML module's resources.
///
/// `build.rs` embeds `khaxbps-qt/resources/catalog.json` under the module's
/// import path, so the Qt layer can read it with `QFile` and hand the contents
/// to [`khaxbps_core::Catalog::load_from_str`].
pub const CATALOG_RESOURCE_PATH: &str = ":/qt/qml/org/kde/khaxbps/resources/catalog.json";

/// Registers the `org.kde.khaxbps` QML module (types and QML resources).
///
/// Must run before a `QQmlApplicationEngine` resolves any `qrc:/qt/qml/...` URL.
/// In CXX-Qt 0.10 this macro replaces the `qml_register_type` call of the 0.7-0.9 API.
pub fn init() {
    cxx_qt::init_qml_module!("org.kde.khaxbps");
}

/// Installs the KI18n `KLocalizedContext` on the engine's root context so QML
/// can call `i18nd()` / `i18n()`.
///
/// Must run before `QQmlApplicationEngine::load`, otherwise the QML file is
/// compiled against a root context without the translation helpers and every
/// `i18nd(...)` call raises `ReferenceError`.
pub fn setup_i18n(engine: Pin<&mut cxx_qt_lib::QQmlApplicationEngine>) {
    use cxx_qt::casting::Upcast;

    qobject::setup_i18n(Upcast::<cxx_qt_lib::QQmlEngine>::upcast_pin(engine));
}
