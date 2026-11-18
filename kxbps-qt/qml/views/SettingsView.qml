// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Settings: the preferences the store keeps, plus the About entry.
//
// The switches write through `PrefsBackend`, which owns no store of its own:
// the installed backend is the only `Store` owner, and `PrefsBackend` forwards
// every write to it. The autostart file follows `startWithSession` through
// `Main.qml`, so this page never touches `~/.config/autostart`.
//
// `checked` is bound with a `Binding` element rather than a plain expression:
// a `Switch` assigns `checked` itself on a click, which kills the binding. The
// `Binding` survives that assignment and shows the store's value when the page
// is opened again.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var prefsBackend

    signal aboutRequested()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        Kirigami.FormLayout {
            Layout.fillWidth: true

            QQC2.Switch {
                id: notifySwitch
                Kirigami.FormData.label: i18nd("kxbps", "Notifications:")
                text: i18nd("kxbps", "Announce available updates on startup")
                enabled: view.prefsBackend !== null
                onToggled: {
                    if (view.prefsBackend !== null) {
                        view.prefsBackend.notifyOnStart = checked
                    }
                }
                Binding {
                    target: notifySwitch
                    property: "checked"
                    value: view.prefsBackend !== null && view.prefsBackend.notifyOnStart
                    restoreMode: Binding.RestoreNone
                }
            }

            QQC2.Switch {
                id: desktopSwitch
                text: i18nd("kxbps", "Use desktop notifications")
                QQC2.ToolTip.text: i18nd("kxbps", "When off, only the tray icon shows available updates.")
                QQC2.ToolTip.visible: hovered
                QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                enabled: view.prefsBackend !== null
                onToggled: {
                    if (view.prefsBackend !== null) {
                        view.prefsBackend.desktopNotifications = checked
                    }
                }
                Binding {
                    target: desktopSwitch
                    property: "checked"
                    value: view.prefsBackend !== null && view.prefsBackend.desktopNotifications
                    restoreMode: Binding.RestoreNone
                }
            }

            QQC2.Switch {
                id: sessionSwitch
                Kirigami.FormData.label: i18nd("kxbps", "Startup:")
                text: i18nd("kxbps", "Start with the session")
                enabled: view.prefsBackend !== null
                onToggled: {
                    if (view.prefsBackend !== null) {
                        view.prefsBackend.startWithSession = checked
                    }
                }
                Binding {
                    target: sessionSwitch
                    property: "checked"
                    value: view.prefsBackend !== null && view.prefsBackend.startWithSession
                    restoreMode: Binding.RestoreNone
                }
            }

            QQC2.ComboBox {
                id: intervalCombo
                Kirigami.FormData.label: i18nd("kxbps", "Check for updates:")
                enabled: view.prefsBackend !== null
                // The minutes the store accepts, in the order the list shows
                // them; the keys are compared with `updateCheckMinutes`, which
                // the store already normalized to one of these.
                readonly property var minutes: [30, 60, 180, 360, 720, 1440]
                // A sentence per entry, each its own `i18nd` call: gettext
                // cannot follow a string built from a fragment, and the plural
                // forms differ per language, so the labels are not derived from
                // the number.
                function labelText(index) {
                    switch (index) {
                    case 0:
                        return i18nd("kxbps", "Every 30 minutes")
                    case 1:
                        return i18nd("kxbps", "Every hour")
                    case 2:
                        return i18nd("kxbps", "Every 3 hours")
                    case 3:
                        return i18nd("kxbps", "Every 6 hours")
                    case 4:
                        return i18nd("kxbps", "Every 12 hours")
                    case 5:
                        return i18nd("kxbps", "Every 24 hours")
                    default:
                        return ""
                    }
                }
                model: [labelText(0), labelText(1), labelText(2), labelText(3), labelText(4), labelText(5)]
                textRole: "modelData"
                // Written on `activated` only: a binding that wrote back would
                // save on every keyboard step through the list, and a click
                // assigns `currentIndex` itself, which kills a plain binding.
                onActivated: {
                    if (view.prefsBackend !== null) {
                        view.prefsBackend.updateCheckMinutes = minutes[currentIndex]
                    }
                }
                Binding {
                    target: intervalCombo
                    property: "currentIndex"
                    value: {
                        if (view.prefsBackend === null) {
                            return -1
                        }
                        const at = intervalCombo.minutes.indexOf(view.prefsBackend.updateCheckMinutes)
                        return at >= 0 ? at : -1
                    }
                    restoreMode: Binding.RestoreNone
                }
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "The switches are saved to the application's state directory and survive a restart.")
            color: Kirigami.Theme.disabledTextColor
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "KXbps is a native KDE package manager for Void Linux. It reads and changes the system through xbps.")
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
            }

            DenseButton {
                text: i18nd("kxbps", "About KXbps")
                icon.name: "help-about"
                onClicked: view.aboutRequested()
            }
        }

        Item {
            Layout.fillHeight: true
        }
    }
}
