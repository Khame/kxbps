//! QML-facing bridge for Khaxbps.
//!
//! The QML module `org.kde.kxbps` is declared in `build.rs`; every `#[qml_element]`
//! QObject in this file is registered into that module by `qmltyperegistrar`, so QML
//! only needs `import org.kde.kxbps`.

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
pub mod prefs_backend;
pub mod queue_model;
pub mod xbps_runner;

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

        #[cxx_name = "kxbps_read_resource"]
        fn read_resource(path: &QString) -> QString;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlengine.h");
        type QQmlEngine = cxx_qt_lib::QQmlEngine;

        // Implemented in `src/k18n_shim.cpp`; `Pin<&mut T>` is passed as `T&`.
        // Without this the QML `i18nd()` helpers are undefined: they are
        // `Q_INVOKABLE` members of the `KLocalizedContext` root context object.
        include!("k18n_shim.h");

        #[cxx_name = "kxbps_setup_i18n"]
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
/// `build.rs` embeds `kxbps-qt/resources/catalog.json` under the module's
/// import path, so the Qt layer can read it with `QFile` and hand the contents
/// to [`kxbps_core::Catalog::load_from_str`].
pub const CATALOG_RESOURCE_PATH: &str = ":/qt/qml/org/kde/kxbps/resources/catalog.json";

/// Registers the `org.kde.kxbps` QML module (types and QML resources).
///
/// Must run before a `QQmlApplicationEngine` resolves any `qrc:/qt/qml/...` URL.
/// In CXX-Qt 0.10 this macro replaces the `qml_register_type` call of the 0.7-0.9 API.
pub fn init() {
    cxx_qt::init_qml_module!("org.kde.kxbps");
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

/// The module URI is spelled out in five independent literals that no compiler
/// check connects: `build.rs` (`QmlModule::new`), this file
/// (`init_qml_module!`, `CATALOG_RESOURCE_PATH`), `main.rs` (the `qrc:` URL it
/// loads) and `Main.qml` (the `import`). A disagreement fails silently — the
/// engine resolves no module and the app exits with an empty window — so this
/// test pins all five to one URI.
#[cfg(test)]
mod uri_guard {
    use super::CATALOG_RESOURCE_PATH;

    /// The double-quoted literal that follows `marker` in `source`.
    fn quoted_after(source: &str, marker: &str) -> String {
        let start = source
            .find(marker)
            .unwrap_or_else(|| panic!("`{marker}` not found"))
            + marker.len();
        let end = start
            + source[start..]
                .find('"')
                .unwrap_or_else(|| panic!("unterminated literal after `{marker}`"));
        source[start..end].to_string()
    }

    #[test]
    fn module_uri_is_the_same_in_every_literal() {
        let build_rs = include_str!("../build.rs");
        let lib_rs = include_str!("lib.rs");
        let main_rs = include_str!("main.rs");
        let main_qml = include_str!("../qml/Main.qml");

        let declared = quoted_after(build_rs, "QmlModule::new(\"");
        let registered = quoted_after(lib_rs, "init_qml_module!(\"");
        assert_eq!(declared, "org.kde.kxbps");
        assert_eq!(
            registered, declared,
            "the URI `init()` registers differs from the one `build.rs` declares"
        );

        // `cxx-qt-build` serves the module's resources below the URI with dots
        // turned into separators, so the catalog path is derived from it.
        let uri_path = declared.replace('.', "/");
        assert_eq!(
            CATALOG_RESOURCE_PATH,
            format!(":/qt/qml/{uri_path}/resources/catalog.json")
        );
        assert_eq!(
            quoted_after(main_rs, "QUrl::from(\""),
            format!("qrc:/qt/qml/{uri_path}/qml/Main.qml"),
            "the URL `main.rs` loads is not inside the registered module"
        );

        let import = main_qml
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("import {declared}")))
            .unwrap_or_else(|| panic!("Main.qml does not import {declared}"));
        assert_eq!(
            import.trim(),
            format!("import {declared}"),
            "Main.qml imports a different module than the one `init()` registers"
        );
    }
}
