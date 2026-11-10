// SPDX-License-Identifier: GPL-2.0-or-later
//
// KI18n glue for QML.
//
// `import org.kde.ki18n` only registers the QML plugin; it does not define the
// `i18n*`/`i18nd*` functions. Those are `Q_INVOKABLE` members of the
// `KLocalizedContext` object, so they only become global functions in QML once
// that object is installed as the engine's root context object.
//
// Declared in the CXX bridge (`src/lib.rs`) as
//     fn setup_i18n(engine: Pin<&mut QQmlEngine>);
// which cxx lowers to the `QQmlEngine &` parameter below.

#include <QQmlContext>
#include <QQmlEngine>

#include <KLocalizedContext>
#include <KLocalizedString>

// `KLocalizedContext` is deprecated since KI18n 6.8 in favour of
// `KLocalization::setupLocalizedContext()`. It still ships and still provides
// `i18nd()`, which is what the QML layer calls; keep the deprecation note out of
// the build log until the i18n task moves to the newer context object.
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wdeprecated-declarations"
#endif

void khaxbps_setup_i18n(QQmlEngine &engine)
{
    KLocalizedString::setApplicationDomain("khaxbps");
    engine.rootContext()->setContextObject(new KLocalizedContext(&engine));
}

#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#endif
