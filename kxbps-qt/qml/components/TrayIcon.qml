// The tray icon while the app runs.
//
// Loaded through a `Loader` in `Main.qml`: `Qt.labs.platform` is an optional
// module, and a missing one must cost the tray icon, not the window. The
// `Loader`'s error status is the only thing that sees the failure.
//
// The icon is the application tile, not a symbolic outline: the user chose it,
// and it is the same file the desktop entry and the header use. `icon.source`
// takes the file; a name would resolve to whatever the theme thinks KXbps looks
// like. The tooltip carries the update count, which makes the icon useful on a
// panel with no room for the window.
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

    icon.source: "qrc:/qt/qml/org/kde/kxbps/icons/kxbps.svg"
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
