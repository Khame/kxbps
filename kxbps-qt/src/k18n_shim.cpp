// SPDX-License-Identifier: GPL-3.0-or-later
//
// KI18n glue for QML.
//
// `import org.kde.ki18n` only registers the QML plugin; it does not define the
// `i18n*`/`i18nd*` functions. Those are members of the localized context
// object, so they only become available in QML once that object is installed in
// the engine's root context.
//
// `KLocalizedContext` did that until KI18n 6.8 deprecated it in favour of
// `KLocalization::setupLocalizedContext()`, which creates a
// `KLocalizedQmlContext` and installs it. It also re-evaluates bindings when the
// application language changes, which the old object did not.
//
// Declared in the CXX bridge (`src/lib.rs`) as
//     fn setup_i18n(engine: Pin<&mut QQmlEngine>);
// which cxx lowers to the `QQmlEngine &` parameter below.

#include <QQmlEngine>

#include <QFont>
#include <QGuiApplication>

#include <KLocalizedQmlContext>
#include <KLocalizedString>

void kxbps_setup_i18n(QQmlEngine &engine)
{
    KLocalizedString::setApplicationDomain("kxbps");
    KLocalization::setupLocalizedContext(&engine);
}

void kxbps_set_compact_font()
{
    QFont font = QGuiApplication::font();
    if (font.pointSize() > 10) {
        font.setPointSize(10);
        QGuiApplication::setFont(font);
    }
}
