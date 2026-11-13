// Settings: the three preferences the store keeps, plus the About entry.
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

            QQC2.Button {
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
