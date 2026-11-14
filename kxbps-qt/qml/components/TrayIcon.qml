// The tray icon while the app runs.
//
// Loaded through a `Loader` in `Main.qml`: `Qt.labs.platform` is an optional
// module, and a missing one must cost the tray icon, not the window. The
// `Loader`'s error status is the only thing that sees the failure.
//
// The icon is the KDE symbolic name, not a qrc file: the StatusNotifierItem
// protocol carries an icon *name*, and a pixmap the host cannot resolve leaves an
// empty slot in the panel. The `-symbolic` suffix is what makes KIconLoader
// recolour the icon for the panel's palette; the plain names are installed for
// hosts that look them up literally. The tooltip carries the update count, which
// makes the icon useful on a panel with no room for the window.
//
// Activation is deliberately narrow: Trigger and DoubleClick raise the window.
// MiddleClick and Context do nothing here, because the context menu is the
// platform's own.

import QtQuick
import Qt.labs.platform
import org.kde.ki18n 1.0

SystemTrayIcon {
    id: tray

    // Asked to open the window and the Updates filter; `Main.qml` owns what
    // that means, so the same entry point serves the notification.
    signal showRequested()
    // The context menu's Quit: `Main.qml` decides whether closing is real.
    signal quitRequested()

    property var installedBackend

    readonly property int updateCount: installedBackend !== null ? installedBackend.updatesCount : 0

    // A theme name, not the qrc file: the StatusNotifierItem protocol carries
    // an icon *name*, and a pixmap the host cannot resolve leaves an empty
    // slot in the panel. The `-symbolic` name resolves through the symbolic
    // tray entries; while updates wait, the icon becomes the update glyph, so
    // the panel says what the tooltip says.
    icon.name: updateCount > 0 ? "kxbps-tray-updates-symbolic" : "kxbps-tray-symbolic"
    tooltip: updateCount > 0
             ? i18ndp("kxbps", "%1 package can be updated.", "%1 packages can be updated.", updateCount)
             : i18nd("kxbps", "KXbps")
    visible: true

    onActivated: function(reason) {
        if (reason === SystemTrayIcon.Trigger || reason === SystemTrayIcon.DoubleClick) {
            tray.showRequested()
        }
    }

    menu: Menu {
        MenuItem {
            text: i18nd("kxbps", "Open KXbps")
            onTriggered: tray.showRequested()
        }
        MenuItem {
            text: i18nd("kxbps", "Quit")
            onTriggered: tray.quitRequested()
        }
    }
}
