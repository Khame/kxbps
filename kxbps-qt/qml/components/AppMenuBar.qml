// The application menu bar: File, Package, View, Settings.
//
// `Kirigami.ApplicationWindow` has no `menuBar` property (Kirigami 6.11), so
// this is a `QQC2.MenuBar` placed by `Main.qml` as the first row of the
// right-hand column. That makes it an **in-window** bar: it belongs to this
// window and scrolls with nothing, but it is not exported to the Plasma global
// menu, and a user who hides all in-window decoration will not see it.
//
// The bar owns no state. It reads the window's selection tuple and the
// preferences object, and reports every action through a signal; `Main.qml`
// decides what to call. No item ever acts on an empty selection: each one is
// disabled until the field it needs is present.
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
    property string packagePkgver: ""
    property bool packageInstalled: false
    property bool packageIsUpdate: false
    property bool packageHeld: false
    property string packageQueuedAction: ""

    // The preferences object from `Main.qml`; null until it is attached.
    property var prefsBackend

    // Whether the store owner is attached. `Main.qml` passes
    // `installedBackend !== null`; a reset before that would rewrite a store the
    // app has not read yet.
    property bool storeReady: false

    // Section and filter state, so File/Package/View can be disabled when they
    // have nothing to say (`Remove…` needs an install, Update all needs updates).
    property int updatesCount: 0
    property string section: "packages"
    property string filter: ""

    signal resetRequested()
    signal aboutRequested()
    signal queueToggleRequested()
    signal holdToggleRequested()
    signal removeRequested()
    signal updateAllRequested()
    signal filterSelected(string token)
    signal sectionSelected(string token)
    signal showInstalledInQueryRequested()

    // True once a row is selected. Every Package item needs it.
    readonly property bool hasSelection: menuBar.packageName.length > 0

    // Whether a package-list filter is the one on screen. A function, not a
    // property: the `Binding` elements call it, and the condition stays in one
    // place instead of six.
    function isFilter(token) {
        return menuBar.section === "packages" && menuBar.filter === token
    }

    // The style sizes the bar itself; a pixel height here would break scaling.
    // The style also draws no background, and the line under the bar belongs to
    // the header below, so nothing is painted here.
    Accessible.role: Accessible.MenuBar
    Accessible.name: i18nd("kxbps", "Application menu")

    QQC2.Menu {
        title: i18nd("kxbps", "File")

        QQC2.MenuItem {
            text: i18nd("kxbps", "Restore base image")
            // The reset dialog calls `installedBackend.reset()`, so the item
            // waits for the store owner to be attached.
            enabled: menuBar.storeReady
            onTriggered: menuBar.resetRequested()
        }
    }

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

    QQC2.Menu {
        title: i18nd("kxbps", "View")

        // The four filter entries are marks on the window state, not switches:
        // a click only asks for a new state. `MenuItem` assigns `checked`
        // itself on activation, which kills a plain binding, so the value is
        // bound with a `Binding` element: it survives that assignment and
        // renders correctly before the first click too. Without it the menu
        // stops following the window and the sidebar leaves the mark lying.
        QQC2.MenuItem {
            text: i18nd("kxbps", "Installed")
            checkable: true
            Binding on checked {
                value: menuBar.isFilter("installed")
            }
            onTriggered: menuBar.filterSelected("installed")
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Updates")
            checkable: true
            Binding on checked {
                value: menuBar.isFilter("updates")
            }
            onTriggered: menuBar.filterSelected("updates")
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Available")
            checkable: true
            Binding on checked {
                value: menuBar.isFilter("all")
            }
            onTriggered: menuBar.filterSelected("all")
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Queue")
            checkable: true
            Binding on checked {
                value: menuBar.isFilter("queued")
            }
            onTriggered: menuBar.filterSelected("queued")
        }

        QQC2.MenuSeparator {}

        QQC2.MenuItem {
            text: i18nd("kxbps", "Update all")
            enabled: menuBar.updatesCount > 0
            onTriggered: menuBar.updateAllRequested()
        }

        QQC2.MenuSeparator {}

        QQC2.MenuItem {
            text: i18nd("kxbps", "Query")
            checkable: true
            Binding on checked {
                value: menuBar.section === "query"
            }
            onTriggered: menuBar.sectionSelected("query")
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Mirrors")
            checkable: true
            Binding on checked {
                value: menuBar.section === "mirrors"
            }
            onTriggered: menuBar.sectionSelected("mirrors")
        }

        QQC2.MenuSeparator {}

        // `-L` is an xbps flag, not a word: it stays untranslated inside the
        // sentence, and a translation may move it anywhere in the line.
        QQC2.MenuItem {
            text: i18nd("kxbps", "Show installed in xbps-query -L")
            onTriggered: menuBar.showInstalledInQueryRequested()
        }
    }

    QQC2.Menu {
        title: i18nd("kxbps", "Settings")

        // The three switches. `MenuItem` assigns `checked` itself when
        // activated, which kills a plain binding, so the value comes from a
        // `Binding` element — it survives the assignment and is correct before
        // the first click. The handler only tells the backend; the backend's
        // property is what the checkbox shows, so a change made elsewhere (a
        // reset, another window) reaches the menu too. The `null` guard is part
        // of the bound expression: without a backend the item is disabled and
        // shows unchecked rather than a guessed value.
        QQC2.MenuItem {
            text: i18nd("kxbps", "Notify on startup")
            checkable: true
            enabled: menuBar.prefsBackend !== null
            Binding on checked {
                value: menuBar.prefsBackend !== null
                       && menuBar.prefsBackend.notifyOnStart
            }
            onToggled: {
                if (menuBar.prefsBackend !== null) {
                    menuBar.prefsBackend.notifyOnStart = checked
                }
            }
        }

        QQC2.MenuItem {
            text: i18nd("kxbps", "Desktop notifications")
            checkable: true
            enabled: menuBar.prefsBackend !== null
            Binding on checked {
                value: menuBar.prefsBackend !== null
                       && menuBar.prefsBackend.desktopNotifications
            }
            onToggled: {
                if (menuBar.prefsBackend !== null) {
                    menuBar.prefsBackend.desktopNotifications = checked
                }
            }
        }

        // The autostart entry follows this switch through `PrefsBackend`:
        // `Main.qml` has a `Connections` block on `onStartWithSessionChanged`
        // that calls `syncAutostart()`, so moving the toggle here keeps the
        // `~/.config/autostart` file in step without a second code path.
        QQC2.MenuItem {
            text: i18nd("kxbps", "Start with session")
            checkable: true
            enabled: menuBar.prefsBackend !== null
            Binding on checked {
                value: menuBar.prefsBackend !== null
                       && menuBar.prefsBackend.startWithSession
            }
            onToggled: {
                if (menuBar.prefsBackend !== null) {
                    menuBar.prefsBackend.startWithSession = checked
                }
            }
        }

        QQC2.MenuSeparator {}

        QQC2.MenuItem {
            text: i18nd("kxbps", "About KXbps")
            onTriggered: menuBar.aboutRequested()
        }
    }
}
