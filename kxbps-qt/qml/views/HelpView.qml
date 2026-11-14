// The user guide, in the window and in the user's language.
//
// The README covers installation and building, and `man kxbps` covers the
// command line; neither is translated. This page says what the program does
// with the system, which is the part a user needs while the window is open.
//
// The whole text lives in `sections` below. Adding a section is adding one
// object there, and `scripts/extract-i18n.sh` picks the new `i18nd` calls up
// from the catalogs: no other file decides what this page says.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Item {
    id: view

    readonly property var sections: [
        {
            heading: i18nd("kxbps", "What KXbps does"),
            paragraphs: [
                i18nd("kxbps", "KXbps is a window onto xbps, the Void Linux package manager. Every list, version, size and error comes from an xbps command; nothing is invented."),
                i18nd("kxbps", "It installs, updates and removes packages, switches the mirror and adds repositories, and asks for authorisation through polkit before it changes anything.")
            ]
        },
        {
            heading: i18nd("kxbps", "The queue"),
            paragraphs: [
                i18nd("kxbps", "Marking a package adds it to the queue; the Apply button runs the queue."),
                i18nd("kxbps", "Apply asks xbps for a dry run first, so the plan and the exact byte count appear before anything is written. A plan xbps refuses is shown with its reason, and nothing runs."),
                i18nd("kxbps", "The xbps log drawer at the bottom of the window carries the raw output of every command, and opens by itself when a transaction starts.")
            ]
        },
        {
            heading: i18nd("kxbps", "Updates"),
            paragraphs: [
                i18nd("kxbps", "The update check follows the interval set in Settings, and runs again when the window is opened after a long pause."),
                i18nd("kxbps", "When updates are waiting, KXbps announces them once, reminds you after three hours, and after that only the tray icon keeps reporting."),
                i18nd("kxbps", "The tray icon shows an arrow while updates wait, and the dock button of the open window carries their number.")
            ]
        },
        {
            heading: i18nd("kxbps", "Authorisation"),
            paragraphs: [
                i18nd("kxbps", "Reading the package lists needs no root."),
                i18nd("kxbps", "Installing, removing and changing the system need authorisation through polkit. The password prompt belongs to the system, not to this window."),
                i18nd("kxbps", "If the prompt is dismissed or refused, the status bar says so and nothing changes.")
            ]
        },
        {
            heading: i18nd("kxbps", "Where the state lives"),
            paragraphs: [
                i18nd("kxbps", "The queue, the preferences and the chosen mirror are kept in ~/.local/share/kxbps/."),
                i18nd("kxbps", "The xbps configuration files are under /etc/xbps.d.")
            ]
        },
        {
            heading: i18nd("kxbps", "More"),
            paragraphs: [
                i18nd("kxbps", "The README covers installation and building. The manual page, man kxbps, covers the command line.")
            ]
        }
    ]

    QQC2.ScrollView {
        anchors.fill: parent
        clip: true
        // The page is a column of text: a horizontal scroll bar would mean a
        // line the user has to drag to read.
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            x: Kirigami.Units.largeSpacing
            y: Kirigami.Units.largeSpacing
            width: Math.min(parent.width - Kirigami.Units.largeSpacing * 2,
                            Kirigami.Units.gridUnit * 46)
            spacing: Kirigami.Units.largeSpacing

            Repeater {
                model: view.sections

                delegate: ColumnLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    Kirigami.Heading {
                        Layout.fillWidth: true
                        level: 2
                        text: modelData.heading
                        wrapMode: Text.WordWrap
                    }

                    Repeater {
                        model: modelData.paragraphs

                        delegate: QQC2.Label {
                            required property string modelData
                            Layout.fillWidth: true
                            text: modelData
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }

            Item {
                Layout.fillHeight: true
            }
        }
    }
}
