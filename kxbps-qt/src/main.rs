use cxx_qt_lib::{QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use cxx_qt_lib_extras::QApplication;
use std::env;

fn main() {
    // `QApplication`, not `QGuiApplication`: `Qt.labs.platform`'s
    // `SystemTrayIcon` is backed by `QSystemTrayIcon` from Qt Widgets, and
    // without a `QApplication` it aborts the process with
    // "QWidget: Cannot create a QWidget without QApplication".
    let mut app = QApplication::new();
    cxx_qt_lib::QGuiApplication::set_desktop_file_name(&QString::from("io.github.khame.kxbps"));
    if env::var("QT_QUICK_CONTROLS_STYLE").is_err() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }

    kxbps_qt::init();

    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        // Root context must carry KI18n's `KLocalizedContext` before the QML file
        // is compiled, or every `i18nd()` call is a `ReferenceError`.
        kxbps_qt::setup_i18n(engine.as_mut());
        engine.load(&QUrl::from("qrc:/qt/qml/org/kde/kxbps/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
