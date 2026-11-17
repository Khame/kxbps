// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The tray icon, as a StatusNotifierItem.
//
// `Qt.labs.platform`'s `SystemTrayIcon` gives the QML menu to Qt Widgets, which
// on Wayland creates a `QMenu` window that cannot grab and prints
// `Failed to create grabbing popup` plus three `QPainter` errors on every
// start. KStatusNotifierItem exports the same menu over `com.canonical.dbusmenu`
// and never shows a widget, so the warnings are gone and the host draws the
// menu itself.
//
// This header is included by the CXX bridge (`lib.rs`) and by nothing else, so
// it holds only the one function Rust calls. The QObject lives in
// `tray_item.h`, which moc reads and the bridge never sees: a header included
// both by the generated bridge code and by its own moc translation unit is
// included twice under two paths, and `#pragma once` does not stop that.

#pragma once

class QQmlEngine;

/// Creates the tray and installs it as the `trayBackend` context property.
///
/// Called once, before the QML file is loaded: the window binds its count and
/// its two slots to this object at construction. A session without a status
/// notifier host still gets the object — it simply never appears — so QML never
/// has to guard a missing target.
void kxbps_install_tray(QQmlEngine &engine);
