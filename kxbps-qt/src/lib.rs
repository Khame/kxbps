//! QML-facing bridge for KXbps.
//!
//! The QML module `org.kde.kxbps` is declared in `build.rs`; every `#[qml_element]`
//! QObject in this file is registered into that module by `qmltyperegistrar`, so QML
//! only needs `import org.kde.kxbps`.

// Keep the Quick Controls extras crate in the link graph: its initializer archive is
// only pulled in by the linker when some Rust code references the crate.
#[allow(unused_imports)]
use cxx_qt_lib_extras as _;

use std::pin::Pin;

pub mod autostart;
pub mod installed_backend;
pub mod job_backend;
pub mod log_model;
pub mod mirror_backend;
pub mod mirror_model;
pub mod mirror_system;
pub mod notification;
pub mod package_model;
pub mod plan_backend;
pub mod prefs_backend;
pub mod query_backend;
pub mod queue_model;
pub mod xbps_bridge;
pub mod xbps_plan;
pub mod xbps_runner;
pub mod xbps_snapshot;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
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

        // Implemented in `src/backend_shim.cpp`; Qt Widgets owns the native
        // tray, so QML asks before it creates a `SystemTrayIcon`.
        include!("backend_shim.h");

        #[cxx_name = "kxbps_tray_available"]
        fn tray_available() -> bool;

        // `org.freedesktop.Notifications.Notify`, sent from `backend_shim.cpp`.
        // The notification module was the only user of `KNotification`, and it
        // silently dropped the event; the session bus does not.
        #[cxx_name = "kxbps_notify"]
        fn notify(summary: &QString, body: &QString, icon: &QString);
    }

    // `auto_cxx_name` turns `update_count` into the QML-facing `updateCount`;
    // without it CXX-Qt exposes the raw snake_case name to QML.
    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(i32, update_count)]
        /// Whether this session has a tray host; constant for the run.
        #[qproperty(bool, tray_available, READ, CONSTANT)]
        type AppBackend = super::AppBackendRust;

        #[qinvokable]
        fn version(self: &AppBackend) -> QString;

        /// Announces a set of available updates. The strings arrive already
        /// translated; this only forwards them to the session bus.
        #[qinvokable]
        fn notify(self: &AppBackend, summary: &QString, body: &QString, icon: &QString);
    }
}

/// Rust side of [`qobject::AppBackend`].
pub struct AppBackendRust {
    update_count: i32,
    tray_available: bool,
}

impl Default for AppBackendRust {
    /// The tray answer is fixed for the run, so it is read once, when QML
    /// instantiates the backend (the application already exists by then).
    fn default() -> Self {
        Self {
            update_count: 0,
            tray_available: qobject::tray_available(),
        }
    }
}

impl qobject::AppBackend {
    /// Application version shown in the UI; proves Rust -> QML calls work.
    fn version(&self) -> cxx_qt_lib::QString {
        cxx_qt_lib::QString::from("0.1.0")
    }

    /// Forwards one announcement to the desktop's notification service.
    fn notify(
        &self,
        summary: &cxx_qt_lib::QString,
        body: &cxx_qt_lib::QString,
        icon: &cxx_qt_lib::QString,
    ) {
        qobject::notify(summary, body, icon);
    }
}

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

/// The module URI is spelled out in four independent literals that no compiler
/// check connects: `build.rs` (`QmlModule::new`), this file
/// (`init_qml_module!`), `main.rs` (the `qrc:` URL it loads) and `Main.qml`
/// (the `import`). A disagreement fails silently — the engine resolves no
/// module and the app exits with an empty window — so this test pins all four
/// to one URI.
#[cfg(test)]
mod uri_guard {
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
        // turned into separators.
        let uri_path = declared.replace('.', "/");
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
