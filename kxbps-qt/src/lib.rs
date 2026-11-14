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

use cxx_qt::CxxQtType;

pub mod autostart;
pub mod cli;
pub mod installed_backend;
pub mod job_backend;
pub mod log_model;
pub mod mirror_backend;
pub mod mirror_model;
pub mod mirror_system;
pub mod network;
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

/// The version every surface shows: the About dialog, `--version`, and the
/// header of the manual page. It comes from `Cargo.toml`, and the test below
/// pins the files that cannot read it at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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

        /// Implemented in `src/k18n_shim.cpp`; caps the application font at
        /// 10 pt before the QML engine starts.
        #[cxx_name = "kxbps_set_compact_font"]
        fn set_compact_font();

        /// Implemented in `src/style_shim.cpp`; stops the desktop style from
        /// underlining button mnemonics.
        include!("style_shim.h");

        #[cxx_name = "kxbps_set_plain_buttons"]
        fn set_plain_buttons();

        // The tray icon, implemented in `src/tray_shim.h`. A StatusNotifierItem
        // owns the menu over D-Bus, so no widget QMenu is created and Wayland
        // has nothing to grab. Installed as the `trayBackend` context property
        // before the QML file loads.
        include!("tray_shim.h");

        #[cxx_name = "kxbps_install_tray"]
        fn install_tray(engine: Pin<&mut QQmlEngine>);

        // Implemented in `src/backend_shim.cpp`; Qt Widgets owns the native
        // tray, so QML asks before it creates a `SystemTrayIcon`.
        include!("backend_shim.h");

        #[cxx_name = "kxbps_tray_available"]
        fn tray_available() -> bool;

        // `org.freedesktop.Notifications.Notify`, sent from `backend_shim.cpp`.
        // The notification module was the only user of `KNotification`, and it
        // silently dropped the event; the session bus does not.
        #[cxx_name = "kxbps_notify"]
        fn notify(
            summary: &QString,
            body: &QString,
            icon: &QString,
            action: &QString,
            owner: Pin<&mut QObject>,
        );

        // Connects this object to the notification server's `ActionInvoked`
        // signal once per process; the signal calls `onNotificationAction`.
        // Called right before the first `Notify`, when the service is there.
        #[cxx_name = "kxbps_notification_watch_actions"]
        fn notification_watch_actions(owner: Pin<&mut QObject>);

        // The dock badge, through stock Qt: `QGuiApplication::setBadgeNumber`
        // publishes the Unity `LauncherEntry.Update` signal that Plasma's task
        // manager draws as a number on the task button. Zero clears it.
        #[cxx_name = "kxbps_set_badge"]
        fn set_badge_on_dock(count: i64);
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
        fn notify(
            self: Pin<&mut AppBackend>,
            summary: &QString,
            body: &QString,
            icon: &QString,
            action: &QString,
        );

        /// Publishes the update count as the dock badge, or clears it when the
        /// count is zero. Called from QML whenever `updatesCount` moves.
        #[qinvokable]
        fn set_badge(self: &AppBackend, count: i32);

        // Called by `backend_shim.cpp`: the notification server answered a
        // popup with its id, or with `none` when the call failed. Invokable so
        // the watch of `ActionInvoked` can reach it: a D-Bus connection binds a
        // slot or an invokable, and a plain Rust method is neither.
        #[qinvokable]
        #[cxx_name = "rememberNotificationId"]
        fn remember_notification_id(self: Pin<&mut AppBackend>, id: &QString);

        // A D-Bus slot, called when the user clicks the popup's default
        // action. Only an id this process sent counts; anything else is a
        // notification from another app.
        #[qinvokable]
        #[cxx_name = "onNotificationAction"]
        fn on_notification_action(self: Pin<&mut AppBackend>, id: u32, key: &QString);

        /// The user clicked the update popup: the window should come back.
        #[qsignal]
        fn notification_activated(self: Pin<&mut AppBackend>);
    }
}

/// Rust side of [`qobject::AppBackend`].
pub struct AppBackendRust {
    update_count: i32,
    tray_available: bool,
    /// Ids the notification server handed out for this process's popups, newest
    /// last. An `ActionInvoked` for one of them is ours; an id from before a
    /// restart, or from another application, is not.
    notification_ids: Vec<u32>,
}

/// How many notification ids are remembered: enough for the popups a tray
/// session produces before the user acts, and bounded so a long-running
/// instance does not grow a list forever.
const REMEMBERED_NOTIFICATION_IDS: usize = 10;

/// The number the dock badge shows: the update count, never negative.
///
/// Qt takes a signed number and clears the badge on zero. A negative value is a
/// broken state rather than something to draw, so it is clamped instead of
/// reaching the task manager.
fn badge_value(count: i32) -> i64 {
    i64::from(count.max(0))
}

impl Default for AppBackendRust {
    /// The tray answer is fixed for the run, so it is read once, when QML
    /// instantiates the backend (the application already exists by then).
    fn default() -> Self {
        Self {
            update_count: 0,
            tray_available: qobject::tray_available(),
            notification_ids: Vec::new(),
        }
    }
}

impl qobject::AppBackend {
    /// Application version shown in the About dialog.
    ///
    /// It comes from the crate's own version, so the window, `--version` and
    /// the package cannot disagree about which build this is.
    fn version(&self) -> cxx_qt_lib::QString {
        cxx_qt_lib::QString::from(crate::VERSION)
    }

    /// Forwards one announcement to the desktop's notification service.
    ///
    /// The owner is `self`, not a copy: the async reply writes the id back into
    /// this object, and the `ActionInvoked` slot lives here as well.
    fn notify(
        self: Pin<&mut Self>,
        summary: &cxx_qt_lib::QString,
        body: &cxx_qt_lib::QString,
        icon: &cxx_qt_lib::QString,
        action: &cxx_qt_lib::QString,
    ) {
        // The C++ side needs the generated `QObject` itself, and it never
        // stores the pointer: the async reply writes the id back through
        // `rememberNotificationId`, which the parent keeps alive for the run.
        let this = cxx_qt::casting::Upcast::<cxx_qt::QObject>::upcast_pin(self);
        qobject::notify(summary, body, icon, action, this);
    }

    /// Publishes the update count as the dock badge, or clears it.
    ///
    /// The number is the only thing that travels: the badge is drawn by the
    /// task manager from the Unity signal Qt sends, so there is no text to
    /// translate and no icon to choose.
    fn set_badge(&self, count: i32) {
        qobject::set_badge_on_dock(badge_value(count));
    }

    /// Keeps the id the notification server answered with; `none` is a failed
    /// call and is dropped.
    fn remember_notification_id(mut self: Pin<&mut Self>, id: &cxx_qt_lib::QString) {
        let Some(id) = id.to_string().parse::<u32>().ok() else {
            return;
        };
        let data = self.as_mut().rust_mut().get_mut();
        if data.notification_ids.len() == REMEMBERED_NOTIFICATION_IDS {
            data.notification_ids.remove(0);
        }
        if !data.notification_ids.contains(&id) {
            data.notification_ids.push(id);
        }
    }

    /// A click on some popup: ours when the id matches one this process sent
    /// and the action is `default`.
    fn on_notification_action(mut self: Pin<&mut Self>, id: u32, key: &cxx_qt_lib::QString) {
        if key.to_string() != "default" {
            return;
        }
        let ours = {
            let data = self.as_mut().rust_mut().get_mut();
            match data.notification_ids.iter().position(|known| *known == id) {
                Some(at) => {
                    // One click per popup: a repeated action is ignored.
                    data.notification_ids.remove(at);
                    true
                }
                None => false,
            }
        };
        if ours {
            self.as_mut().notification_activated();
        }
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

/// Caps the application font at 10 pt.
///
/// Must run after the `QApplication` exists and before the QML engine loads:
/// Kirigami reads the font once to derive `gridUnit`, so every size in the
/// window follows this call. A system font smaller than 10 pt is left alone.
pub fn set_compact_font() {
    qobject::set_compact_font();
}

/// Stops the desktop style from underlining the keyboard mnemonic in a button
/// label.
///
/// Must run after the `QApplication` exists and before the QML engine loads:
/// the style decides how every button is painted, and the buttons are built
/// while the QML file loads. Alt shortcuts keep working; only the underline
/// goes away.
pub fn set_plain_buttons() {
    qobject::set_plain_buttons();
}

/// Creates the tray icon and installs it as the `trayBackend` context property.
///
/// Must run before `QQmlApplicationEngine::load`: `Main.qml` binds the update
/// count and connects the two tray signals while it is constructed. The object
/// is a `KStatusNotifierItem`, so the menu is exported over D-Bus instead of
/// being a `QMenu` window Wayland cannot grab.
pub fn install_tray(engine: Pin<&mut cxx_qt_lib::QQmlApplicationEngine>) {
    use cxx_qt::casting::Upcast;

    qobject::install_tray(Upcast::<cxx_qt_lib::QQmlEngine>::upcast_pin(engine));
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

/// The dock badge shows the update count, and zero means "no badge".
///
/// The clamp is the only decision here, but it is the one that reaches Qt: a
/// negative number would be a badge with no meaning, and the task manager has
/// no way to know it is broken.
#[cfg(test)]
mod badge_value_tests {
    use super::badge_value;

    #[test]
    fn a_count_is_passed_through() {
        assert_eq!(badge_value(0), 0);
        assert_eq!(badge_value(1), 1);
        assert_eq!(badge_value(15), 15);
        assert_eq!(badge_value(i32::MAX), i64::from(i32::MAX));
    }

    #[test]
    fn a_negative_count_clears_the_badge_instead_of_drawing_it() {
        assert_eq!(badge_value(-1), 0);
        assert_eq!(badge_value(i32::MIN), 0);
    }
}

/// The version lives in `Cargo.toml`, but two files cannot read it: the manual
/// page and the AppStream metadata are shipped as they are written. A stale
/// version there is invisible until someone opens the page or the software
/// centre, so the test pins both to [`VERSION`].
#[cfg(test)]
mod version_guard {
    use super::VERSION;

    #[test]
    fn the_manual_page_header_carries_the_version() {
        let man = include_str!("../../man/kxbps.1");
        assert!(
            man.contains(&format!("\"kxbps {VERSION}\"")),
            "man/kxbps.1 does not name kxbps {VERSION} in its .TH line"
        );
    }

    #[test]
    fn the_metainfo_lists_the_version() {
        let metainfo = include_str!("../../packaging/io.github.khame.kxbps.metainfo.xml");
        assert!(
            metainfo.contains(&format!("<release version=\"{VERSION}\"")),
            "the AppStream metadata has no release entry for {VERSION}"
        );
    }
}
