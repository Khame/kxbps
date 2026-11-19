// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// What this application is, in one screen and without a marketing page.
//
// The version comes from `AppBackend.version()` in Rust; Qt, KDE Frameworks
// and the platform come from `about_shim.cpp`, so the page reports the
// libraries this build actually links against.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../icons/IconNames.js" as Icons

Kirigami.Dialog {
    id: dialog

    property string version: ""
    property string qtVersion: ""
    property string kfVersion: ""
    property string platformName: ""
    // Built by `AppBackend.libraries_text()` in Rust, so the block that goes
    // to the clipboard is the one the tests cover.
    property string librariesText: ""

    readonly property string licenseName: "GPL-3.0-or-later"
    readonly property string licenseUrl: "https://www.gnu.org/licenses/gpl-3.0.html"
    readonly property string websiteUrl: "https://kxbps.khame.dev/"
    readonly property string projectUrl: "https://github.com/Khame/kxbps"
    readonly property string issuesUrl: "https://github.com/Khame/kxbps/issues"
    readonly property string donateUrl: "https://buymeacoffee.com/khame"

    title: i18nd("kxbps", "About KXbps")
    preferredWidth: Kirigami.Units.gridUnit * 50
    padding: Kirigami.Units.largeSpacing

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 48
        spacing: Kirigami.Units.smallSpacing

        // Takes the initial focus so the first action row does not open with a
        // focus ring. Tab still reaches every row in order.
        Item {
            focus: true
            Layout.preferredWidth: 0
            Layout.preferredHeight: 0
        }

        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 30
            clip: true
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            ColumnLayout {
                width: parent.width
                spacing: Kirigami.Units.largeSpacing

                Kirigami.AbstractCard {
                    Layout.fillWidth: true

                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing

                        Image {
                            source: "qrc:/qt/qml/org/kde/kxbps/icons/kxbps.svg"
                            sourceSize.width: Kirigami.Units.iconSizes.huge
                            sourceSize.height: Kirigami.Units.iconSizes.huge
                            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
                            fillMode: Image.PreserveAspectFit
                            Accessible.name: i18nd("kxbps", "KXbps")
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing

                            Kirigami.Heading {
                                Layout.fillWidth: true
                                level: 1
                                text: i18nd("kxbps", "KXbps")
                            }

                            QQC2.Label {
                                Layout.fillWidth: true
                                text: i18nd("kxbps", "Version %1", dialog.version)
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.fillWidth: true
                                text: i18nd("kxbps", "A graphical front end for xbps, the Void Linux package manager.")
                                wrapMode: Text.WordWrap
                            }
                        }
                    }
                }

                Kirigami.AbstractCard {
                    Layout.fillWidth: true

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 3
                            text: i18nd("kxbps", "Copyright")
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: "© 2026 Łukasz \"Khame\" Linhard"
                            wrapMode: Text.WordWrap
                        }
                    }
                }

                Kirigami.AbstractCard {
                    Layout.fillWidth: true

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        Kirigami.Heading {
                            Layout.fillWidth: true
                            level: 3
                            text: i18nd("kxbps", "License")
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: "<a href=\"" + dialog.licenseUrl + "\">" + dialog.licenseName + "</a>"
                            textFormat: Text.RichText
                            onLinkActivated: (link) => Qt.openUrlExternally(link)
                        }
                    }
                }

                AboutActionRow {
                    iconSource: "globe"
                    label: i18nd("kxbps", "Website")
                    target: dialog.websiteUrl
                }

                AboutActionRow {
                    iconSource: "qrc:/qt/qml/org/kde/kxbps/icons/actions/kxbps-" + Icons.coffee + "-symbolic.svg"
                    label: i18nd("kxbps", "Donate")
                    target: dialog.donateUrl
                }

                AboutActionRow {
                    iconSource: "document-edit"
                    label: i18nd("kxbps", "Contribute")
                    target: dialog.projectUrl
                }

                AboutActionRow {
                    iconSource: "tools-report-bug"
                    label: i18nd("kxbps", "Report a bug")
                    target: dialog.issuesUrl
                }

                Kirigami.AbstractCard {
                    Layout.fillWidth: true

                    contentItem: ColumnLayout {
                        spacing: Kirigami.Units.smallSpacing

                        RowLayout {
                            Layout.fillWidth: true

                            Kirigami.Heading {
                                Layout.fillWidth: true
                                level: 3
                                text: i18nd("kxbps", "Used libraries")
                            }

                            DenseButton {
                                text: i18nd("kxbps", "Copy to clipboard")
                                icon.name: "edit-copy-symbolic"
                                // Keep the initial focus off the button; Tab
                                // still reaches it when the user wants it.
                                focusPolicy: Qt.TabFocus
                                activeFocusOnTab: true
                                onClicked: {
                                    clipboard.selectAll()
                                    clipboard.copy()
                                }
                            }
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Platform: %1", dialog.platformName)
                            font.bold: true
                            wrapMode: Text.WordWrap
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "The system KXbps runs on.")
                            color: Kirigami.Theme.disabledTextColor
                            font: Kirigami.Theme.smallFont
                            wrapMode: Text.WordWrap
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "KDE Frameworks %1", dialog.kfVersion)
                            font.bold: true
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Libraries shared by KDE applications.")
                            color: Kirigami.Theme.disabledTextColor
                            font: Kirigami.Theme.smallFont
                            wrapMode: Text.WordWrap
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Qt %1", dialog.qtVersion)
                            font.bold: true
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "The application framework KXbps is built with.")
                            color: Kirigami.Theme.disabledTextColor
                            font: Kirigami.Theme.smallFont
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }

        TextEdit {
            id: clipboard
            visible: false
            text: dialog.librariesText
        }
    }

    // The dialog's own footer would use a style button, which is taller and
    // framed differently from every control in the window. This keeps the close
    // action a DenseButton like the rest of the tool.
    footer: QQC2.Control {
        padding: Kirigami.Units.largeSpacing

        contentItem: RowLayout {
            DenseButton {
                text: i18nd("kxbps", "Close")
                Layout.alignment: Qt.AlignRight
                onClicked: dialog.close()
            }
        }
    }
}
