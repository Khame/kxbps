// The application menu bar: the actions for the row the list has selected.
//
// `Kirigami.ApplicationWindow` has no `menuBar` property (Kirigami 6.11), so
// this is a `QQC2.MenuBar` placed by `Main.qml` as the first row of the
// right-hand column. That makes it an **in-window** bar: it belongs to this
// window and scrolls with nothing, but it is not exported to the Plasma global
// menu, and a user who hides all in-window decoration will not see it.
//
// The bar is shown only on the package list, because every item here acts on
// that list's selection. Query, Mirrors and Settings have no menu bar: their
// actions live in their own headers, next to the content they change.
//
// The bar owns no state. It reads the window's selection tuple and reports
// every action through a signal; `Main.qml` decides what to call. No item ever
// acts on an empty selection: each one is disabled until the field it needs is
// present.
//
// Mnemonics: no `&` is added to any label. The `&` would become part of the
// English msgid and translators would have to keep it in the translated
// string, which a translation can silently drop. The bar is reachable with
// Alt/F10 and the arrow keys instead.

import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

QQC2.MenuBar {
    id: menuBar

    // The window's selection, passed in whole. A name alone would let an item
    // read a `held`/`queued` flag from a row that has since changed.
    property string packageName: ""
    property bool packageInstalled: false
    property bool packageIsUpdate: false
    property bool packageHeld: false
    property string packageQueuedAction: ""

    signal queueToggleRequested()
    signal holdToggleRequested()
    signal removeRequested()

    // True once a row is selected. Every item needs it.
    readonly property bool hasSelection: menuBar.packageName.length > 0

    // The style sizes the bar itself; a pixel height here would break scaling.
    // The style also draws no background, and the line under the bar belongs to
    // the header below, so nothing is painted here.
    Accessible.role: Accessible.MenuBar
    Accessible.name: i18nd("kxbps", "Application menu")

    QQC2.Menu {
        title: i18nd("kxbps", "Package")

        QQC2.MenuItem {
            // One item, one gesture: the label and the token follow the row.
            // "Add to queue" would be a lie on an installed package, and an
            // update queues its own token rather than a reinstall.
            text: menuBar.packageQueuedAction !== "" ? i18nd("kxbps", "Remove from queue")
                  : menuBar.packageIsUpdate ? i18nd("kxbps", "Queue update")
                  : menuBar.packageInstalled ? i18nd("kxbps", "Queue removal")
                                             : i18nd("kxbps", "Add to queue")
            enabled: menuBar.hasSelection
            onTriggered: menuBar.queueToggleRequested()
        }

        QQC2.MenuSeparator {}

        QQC2.MenuItem {
            text: menuBar.packageHeld ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
            // Only an installed package can be held.
            enabled: menuBar.hasSelection && menuBar.packageInstalled
            onTriggered: menuBar.holdToggleRequested()
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Remove…")
            enabled: menuBar.hasSelection && menuBar.packageInstalled
            onTriggered: menuBar.removeRequested()
        }
    }
}
