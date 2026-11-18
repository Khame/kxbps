// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// The user guide, in the window and in the user's language.
//
// The README covers installation and building, and `man kxbps` covers the
// command line; neither is translated. This page says what the program does
// with the system, which is the part a user needs while the window is open.
//
// The whole text lives in `sections` below. Adding a section is adding one
// object there, and `scripts/extract-i18n.sh` picks the new `i18nd` calls up
// from the catalogs: no other file decides what this page says.
//
// The layout is a reading column, not a table: a measure narrow enough for the
// eye to find the next line, a rule between sections, and a heading that stands
// off its own text. The bullet list bolds the term before the colon, because
// that term is what a reader scans for; the sentence after it stays ordinary
// text.

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
                i18nd("kxbps", "Available: repository packages, read fresh when the window starts. Typing filters them in place; a pattern with regex syntax is sent to xbps."),
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
                i18nd("kxbps", "The check fetches the repository indexes into memory, so it needs no password and writes nothing. The Available list is refreshed the same way on the same clock, which is how a package added to a repository shows up without a sync."),
                i18nd("kxbps", "When updates wait, the tray icon shows an arrow and the open window's dock button carries their number."),
                i18nd("kxbps", "The button on this page is the real sync: it runs xbps-install -S through polkit, writes the indexes to disk and asks for the password. Every later read then sees the new packages, in this window and in a terminal."),
                i18nd("kxbps", "Notifications are optional. Settings can turn them off, and then only the tray icon reports.")
            ]
        },
        {
            heading: i18nd("kxbps", "Authorisation"),
            paragraphs: [
                i18nd("kxbps", "Reading package lists needs no root."),
                i18nd("kxbps", "Syncing the repository indexes writes the index files, so it asks for the password as well."),
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

    // The measure. A line that runs the whole width of a maximized window is a
    // line the eye loses its place in; 34 grid units is about ninety characters
    // at the default font, and the page is centred in whatever room it has.
    readonly property real readingWidth:
        Math.min(parent.width - Kirigami.Units.largeSpacing * 2,
                 Kirigami.Units.gridUnit * 34)

    // A bullet is "Term: the sentence that explains it". The term is what a
    // reader scans for, so it is bold and keeps its colon; a bullet without one
    // is drawn as plain text. The split is display only: the translation owns
    // the whole sentence and may put the colon anywhere, or nowhere.
    function bulletHtml(text) {
        const at = text.indexOf(": ")
        if (at <= 0) {
            return escapeHtml(text)
        }
        return "<b>" + escapeHtml(text.slice(0, at)) + "</b>: " + escapeHtml(text.slice(at + 2))
    }

    // The strings are plain text, but `Text.StyledText` reads angle brackets as
    // markup: a translation carrying `~/.local/share` is safe, one carrying a
    // `<` is not, so everything that is not the bold tag is escaped.
    function escapeHtml(text) {
        return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    }

    QQC2.ScrollView {
        anchors.fill: parent
        clip: true
        // The page is a column of text: a horizontal scroll bar would mean a
        // line the user has to drag to read.
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            x: Math.max(Kirigami.Units.largeSpacing,
                        (parent.width - view.readingWidth) / 2)
            y: Kirigami.Units.largeSpacing
            width: view.readingWidth
            // The rhythm is the delegate's: a section owns the air above it, so
            // the rule between two sections sits at the same distance from both.
            spacing: 0

            Repeater {
                model: view.sections

                delegate: ColumnLayout {
                    id: section
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    spacing: 0

                    // The rule is what turns a wall of paragraphs into a page:
                    // it separates two sections without a box or a colour.
                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: section.index === 0 ? 0 : Kirigami.Units.gridUnit
                        Layout.bottomMargin: Kirigami.Units.gridUnit * 0.75
                        visible: section.index > 0
                    }

                    Kirigami.Heading {
                        Layout.fillWidth: true
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        level: 2
                        text: section.modelData.heading
                        wrapMode: Text.WordWrap
                    }

                    // Paragraphs are one block with air between them; the block
                    // itself starts tight under its heading, which is what makes
                    // the heading read as belonging to it.
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing * 2

                        Repeater {
                            model: section.modelData.paragraphs

                            delegate: QQC2.Label {
                                required property string modelData
                                Layout.fillWidth: true
                                text: modelData
                                textFormat: Text.PlainText
                                wrapMode: Text.WordWrap
                            }
                        }
                    }

                    // The list is indented from the paragraph it follows, and
                    // every bullet hangs off one lane so the terms line up.
                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing * 2
                        spacing: Kirigami.Units.smallSpacing
                        visible: section.modelData.bullets !== undefined
                                 && section.modelData.bullets.length > 0

                        Repeater {
                            model: section.modelData.bullets

                            delegate: RowLayout {
                                required property string modelData
                                Layout.fillWidth: true
                                spacing: Kirigami.Units.smallSpacing

                                QQC2.Label {
                                    Layout.alignment: Qt.AlignTop
                                    text: "•"
                                    color: Kirigami.Theme.disabledTextColor
                                }

                                QQC2.Label {
                                    Layout.fillWidth: true
                                    text: view.bulletHtml(modelData)
                                    textFormat: Text.StyledText
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }
                    }
                }
            }

            // The scroll ends where the content does, so the last line needs
            // air under it: a paragraph flush against the edge reads as cut off.
            Item {
                Layout.preferredHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
