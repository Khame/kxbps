// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Creates the tray declared in `tray_item.h` and hands it to QML.
//
// The item is a StatusNotifierItem, not a widget: the icon is a theme name, the
// menu is exported over `com.canonical.dbusmenu`, and nothing is drawn in this
// process. The two actions are the window's requests, and the tooltip carries
// the update count so a panel with no room for the window still says it.

#include "tray_shim.h"

#include "tray_item.h"

#include <QQmlContext>
#include <QQmlEngine>
#include <QVariant>

void kxbps_install_tray(QQmlEngine &engine)
{
    auto *tray = new KxbpsTray(&engine);
    engine.rootContext()->setContextProperty(QStringLiteral("trayBackend"),
                                             QVariant::fromValue(tray));
}
