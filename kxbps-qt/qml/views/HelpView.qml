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
                i18nd("kxbps", "KXbps is a window onto xbps, the Void Linux package manager. Every package, version, size and error comes from an xbps command."),
                i18nd("kxbps", "It installs, updates and removes packages, switches the mirror and adds repositories. It asks for authorisation through polkit before it changes anything.")
            ]
        },
        {
            heading: i18nd("kxbps", "Filters"),
            paragraphs: [
                i18nd("kxbps", "The sidebar chooses the set the table shows. The search field narrows that set.")
            ],
            bullets: [
                i18nd("kxbps", "Installed: packages in the system."),
                i18nd("kxbps", "Upgradable: installed packages with a newer version in the repository."),
                i18nd("kxbps", "Available: repository packages. Type a name and press Enter."),
                i18nd("kxbps", "Marked: packages in the queue."),
                i18nd("kxbps", "Explicit: packages installed on purpose, not as a dependency."),
                i18nd("kxbps", "Orphans: packages installed as a dependency that nothing needs now."),
                i18nd("kxbps", "On hold: packages xbps keeps at their current version.")
            ]
        },
        {
            heading: i18nd("kxbps", "The queue and the plan"),
            paragraphs: [
                i18nd("kxbps", "A click on a row adds the package to the queue; the same click removes it. Apply runs the queue."),
                i18nd("kxbps", "Before anything is written, xbps runs a dry run. The plan shows the packages, the actions and the exact bytes to download and install."),
                i18nd("kxbps", "If xbps refuses the plan, the reason appears and nothing runs. The status strip keeps the result until the queue changes.")
            ]
        },
        {
            heading: i18nd("kxbps", "The xbps log"),
            paragraphs: [
                i18nd("kxbps", "The log drawer at the bottom of the window shows the raw output of every xbps command. Lines are not translated."),
                i18nd("kxbps", "It opens when a transaction starts. The button in the status strip opens and closes it, and the top edge can be dragged to resize."),
                i18nd("kxbps", "If a command fails, the last lines carry the reason xbps printed.")
            ]
        },
        {
            heading: i18nd("kxbps", "Updates"),
            paragraphs: [
                i18nd("kxbps", "KXbps checks for updates on the interval set in Settings. The check also runs when the window opens after a pause."),
                i18nd("kxbps", "When updates wait, the tray icon shows an arrow and the open window's dock button carries their number."),
                i18nd("kxbps", "Notifications are optional. Settings can turn them off, and then only the tray icon reports.")
            ]
        },
        {
            heading: i18nd("kxbps", "Authorisation"),
            paragraphs: [
                i18nd("kxbps", "Reading package lists needs no root."),
                i18nd("kxbps", "Installing, removing, changing the mirror and adding a repository need authorisation through polkit. The password prompt belongs to the system."),
                i18nd("kxbps", "If the prompt is dismissed or refused, the status strip says so and nothing changes.")
            ]
        },
        {
            heading: i18nd("kxbps", "Settings"),
            paragraphs: [
                i18nd("kxbps", "Settings holds the update interval, the startup switch and the two notification switches."),
                i18nd("kxbps", "Start with the session writes an autostart entry. The other switches and the chosen mirror are saved in the state directory.")
            ]
        },
        {
            heading: i18nd("kxbps", "Where the state lives"),
            paragraphs: [
                i18nd("kxbps", "The queue, the preferences and the chosen mirror are kept in ~/.local/share/kxbps/."),
                i18nd("kxbps", "xbps reads its configuration from /etc/xbps.d.")
            ]
        },
        {
            heading: i18nd("kxbps", "More"),
            paragraphs: [
                i18nd("kxbps", "The manual page, man kxbps, covers the command line.")
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
                            textFormat: Text.PlainText
                            wrapMode: Text.WordWrap
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.smallSpacing
                        visible: modelData.bullets !== undefined && modelData.bullets.length > 0

                        Repeater {
                            model: modelData.bullets

                            delegate: RowLayout {
                                required property string modelData
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.smallSpacing

                                QQC2.Label {
                                    Layout.alignment: Qt.AlignTop
                                    text: "•"
                                    color: Kirigami.Theme.textColor
                                }

                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: modelData
                                    textFormat: Text.PlainText
                                    wrapMode: Text.WordWrap
                                }
                            }
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
