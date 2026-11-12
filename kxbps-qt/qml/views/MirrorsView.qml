// The mirror list and the repository state.
//
// Mirrors are the embedded 34-entry snapshot; refreshing the list from
// `void-linux/xmirror` and editing repositories are later tasks. The
// Repositories tab is therefore read-only: it reports what the store implies
// and says so instead of offering checkboxes that would do nothing.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var mirrorBackend
    property var catalogBackend
    property var installedBackend

    readonly property real selectionWidth: Kirigami.Units.gridUnit * 2
    readonly property real regionWidth: Kirigami.Units.gridUnit * 6
    readonly property real latencyWidth: Kirigami.Units.gridUnit * 5
    readonly property real stateWidth: Kirigami.Units.gridUnit * 12

    // A repository is enabled when its `void-repo-*` package is installed,
    // which is exactly what `repos_from_installed` reads in the core. `get()`
    // has no notify signal, so the counter is read to make the binding
    // re-evaluate after a transaction.
    function repoEnabled(repoPackage) {
        view.installedBackend.installedCount
        if (repoPackage.length === 0) {
            // `current` has no toggle package.
            return true
        }
        const info = view.catalogBackend.get(repoPackage)
        return info.installedVersion !== undefined && info.installedVersion.length > 0
    }

    QQC2.TabBar {
        id: tabs
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top

        QQC2.TabButton {
            text: i18nd("kxbps", "Mirrors")
        }
        QQC2.TabButton {
            text: i18nd("kxbps", "Repositories")
        }
    }

    StackLayout {
        anchors.top: tabs.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        currentIndex: tabs.currentIndex

        ColumnLayout {
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.topMargin: Kirigami.Units.smallSpacing
                visible: view.mirrorBackend.probing
                text: i18nd("kxbps", "%1 of %2 mirrors probed",
                            view.mirrorBackend.finishedCount, view.mirrorBackend.model.count)
                color: Kirigami.Theme.disabledTextColor
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ListView {
                    id: mirrorList
                    anchors.fill: parent
                    clip: true
                    model: view.mirrorBackend.model
                    headerPositioning: ListView.OverlayHeader
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

                    header: Item {
                        width: mirrorList.width
                        height: Kirigami.Units.gridUnit * 2

                        Rectangle {
                            anchors.fill: parent
                            color: Kirigami.Theme.alternateBackgroundColor
                        }

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 1
                            color: Kirigami.Theme.disabledTextColor
                        }

                        RowLayout {
                            anchors.fill: parent
                            anchors.leftMargin: Kirigami.Units.largeSpacing
                            anchors.rightMargin: Kirigami.Units.largeSpacing
                            spacing: Kirigami.Units.largeSpacing

                            Item {
                                Layout.preferredWidth: view.selectionWidth
                                Layout.minimumWidth: view.selectionWidth
                                Layout.maximumWidth: view.selectionWidth
                            }

                            QQC2.Label {
                                Layout.fillWidth: true
                                text: i18nd("kxbps", "Server")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.regionWidth
                                Layout.minimumWidth: view.regionWidth
                                Layout.maximumWidth: view.regionWidth
                                text: i18nd("kxbps", "Region")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.latencyWidth
                                Layout.minimumWidth: view.latencyWidth
                                Layout.maximumWidth: view.latencyWidth
                                horizontalAlignment: Text.AlignRight
                                text: i18nd("kxbps", "Latency")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.stateWidth
                                Layout.minimumWidth: view.stateWidth
                                Layout.maximumWidth: view.stateWidth
                                text: i18nd("kxbps", "State")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }
                        }
                    }

                    delegate: MirrorDelegate {
                        width: mirrorList.width
                        mirrorBackend: view.mirrorBackend
                    }
                }

                QQC2.Label {
                    anchors.centerIn: parent
                    visible: view.mirrorBackend.model.count === 0
                    text: i18nd("kxbps", "No mirrors.")
                    color: Kirigami.Theme.disabledTextColor
                }
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                text: i18nd("kxbps", "Saved in %1", "~/.local/share/kxbps/mirror.json")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                text: i18nd("kxbps", "KXbps keeps the selection in its own store; system configuration is not modified.")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
            }
        }

        ColumnLayout {
            spacing: Kirigami.Units.smallSpacing

            Repeater {
                model: [
                    { name: "current", repoPackage: "" },
                    { name: "nonfree", repoPackage: "void-repo-nonfree" },
                    { name: "multilib", repoPackage: "void-repo-multilib" }
                ]

                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.fillWidth: true
                        // Raw repository key, not UI copy.
                        text: modelData.name
                        font.family: "monospace"
                    }

                    QQC2.Label {
                        text: view.repoEnabled(modelData.repoPackage) ? i18nd("kxbps", "Enabled")
                                                                     : i18nd("kxbps", "Disabled")
                        color: view.repoEnabled(modelData.repoPackage) ? Kirigami.Theme.positiveTextColor
                                                                       : Kirigami.Theme.disabledTextColor
                    }
                }
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.topMargin: Kirigami.Units.smallSpacing
                text: i18nd("kxbps", "Enabling a repository installs its void-repo package; that is a later task.")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
            }

            Item {
                Layout.fillHeight: true
            }
        }
    }
}
