// One package row: name and description, version, size, and the one action the
// row exists for.
//
// The action is a visible button, not a hover overlay: the list is a work
// surface, and a control that only appears under the pointer cannot be found by
// someone scanning the screen. Clicking it asks the window for a plan, which is
// a dry run — nothing is installed until the plan dialog's Apply, so a stray
// click costs a dialog, never a package.
//
// The widths come from `PackagesView`, which draws the header with the same
// values, so no label can push the next column out of line.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

QQC2.ItemDelegate {
    id: delegate

    property var installedBackend
    property real versionWidth
    property real sizeWidth
    property real actionWidth
    // Driven by the selected name, not `ListView.currentIndex`: a filter change
    // resets the model and would drop the current index, while the details pane
    // keeps showing the same package.
    property bool selected: false

    // The page asks the window for a transaction; the row never runs anything.
    signal planRequested(string name, string token)

    readonly property bool installed: model.installedVersion !== ""
    readonly property bool isUpdate: model.isUpdate === true
    readonly property bool queued: model.queuedAction !== ""
    // An update outranks "installed": planning an update as an install would
    // reinstall the old version instead of moving to the new one.
    readonly property string actionToken: delegate.isUpdate ? "update"
                                          : (delegate.installed ? "remove" : "install")
    readonly property string actionLabel: delegate.isUpdate ? i18nd("kxbps", "Update")
                                         : (delegate.installed ? i18nd("kxbps", "Remove")
                                                               : i18nd("kxbps", "Install"))
    readonly property string actionIcon: delegate.isUpdate ? "system-software-update"
                                        : (delegate.installed ? "edit-delete" : "document-save")

    implicitHeight: Kirigami.Units.gridUnit * 2.6
    height: implicitHeight
    leftPadding: Kirigami.Units.mediumSpacing
    rightPadding: Kirigami.Units.mediumSpacing
    topPadding: 0
    bottomPadding: 0
    // The current index covers keyboard navigation; the selected name keeps the
    // row marked after a filter change resets the model.
    highlighted: delegate.selected || ListView.isCurrentItem

    Accessible.role: Accessible.ListItem
    Accessible.name: model.name
    Accessible.description: model.short
    Accessible.selected: delegate.highlighted

    contentItem: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        // Name over description: the row is a table cell, so the second line is
        // what tells two similar packages apart (`neovim` against `neovim-qt`).
        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: model.name
                font.bold: true
                elide: Text.ElideRight
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: Kirigami.Units.smallSpacing

                // Only the states that change what the package does. "Explicit"
                // is the normal case and would be a pill on every row.
                Pill {
                    visible: model.held
                    text: i18nd("kxbps", "Held")
                }

                Pill {
                    visible: delegate.queued
                    text: i18nd("kxbps", "Queued")
                    textColor: Kirigami.Theme.highlightColor
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    visible: text.length > 0
                    text: model.short
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.disabledTextColor
                    elide: Text.ElideRight
                }
            }
        }

        // Version: one column with three states, so an update reads as
        // "installed → available" and a fresh install as xbps' version.
        Item {
            Layout.preferredWidth: delegate.versionWidth
            Layout.minimumWidth: delegate.versionWidth
            Layout.maximumWidth: delegate.versionWidth
            Layout.fillHeight: true

            RowLayout {
                anchors.fill: parent
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: !delegate.installed
                    text: model.version
                    font.family: "monospace"
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: delegate.installed && !delegate.isUpdate
                    text: model.installedVersion + " ✓"
                    color: Kirigami.Theme.positiveTextColor
                    font.family: "monospace"
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    visible: delegate.installed && delegate.isUpdate
                    text: model.installedVersion
                    color: Kirigami.Theme.positiveTextColor
                    font.family: "monospace"
                }

                QQC2.Label {
                    visible: delegate.installed && delegate.isUpdate
                    text: "→"
                    color: Kirigami.Theme.disabledTextColor
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: delegate.installed && delegate.isUpdate
                    text: model.updateVersion
                    font.family: "monospace"
                    elide: Text.ElideRight
                }
            }
        }

        // Installed size, or the new version's size on an update row. The
        // tooltip carries the other two numbers, so one narrow column still
        // tells the whole story.
        QQC2.Label {
            Layout.preferredWidth: delegate.sizeWidth
            Layout.minimumWidth: delegate.sizeWidth
            Layout.maximumWidth: delegate.sizeWidth
            horizontalAlignment: Text.AlignRight
            text: delegate.isUpdate ? model.updateSize : model.size
            font.family: "monospace"
            elide: Text.ElideLeft

            QQC2.ToolTip.visible: sizeHover.containsMouse
            QQC2.ToolTip.text: delegate.isUpdate
                ? i18nd("kxbps", "Installed %1, download %2, new version %3",
                        model.size, model.download, model.updateSize)
                : i18nd("kxbps", "Installed %1", model.size)

            MouseArea {
                id: sizeHover
                anchors.fill: parent
                hoverEnabled: true
            }
        }

        QQC2.Button {
            Layout.preferredWidth: delegate.actionWidth
            Layout.minimumWidth: delegate.actionWidth
            Layout.maximumWidth: delegate.actionWidth
            text: delegate.actionLabel
            // Install and update carry their icon; a removal is the quiet,
            // text-only option, because a list where every installed row offers
            // a red button reads as a warning. None of them is the accent
            // button: a column of identical highlighted buttons pulls the eye
            // off the package names.
            icon.name: delegate.actionToken === "remove" ? "" : delegate.actionIcon
            flat: delegate.actionToken === "remove"
            font: Kirigami.Theme.smallFont
            implicitHeight: Kirigami.Units.gridUnit * 1.6
            Accessible.name: delegate.actionLabel + " " + model.name
            onClicked: delegate.planRequested(model.name, delegate.actionToken)
        }
    }

    // A hairline between rows: at 43 px the rows are close enough that the eye
    // needs the same help the mockup's table gives it, without a full-opacity
    // line that would read as a border.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: 1
        color: Qt.rgba(Kirigami.Theme.textColor.r,
                       Kirigami.Theme.textColor.g,
                       Kirigami.Theme.textColor.b, 0.08)
    }

    component Pill: Rectangle {
        id: pill

        property alias text: label.text
        property color textColor: Kirigami.Theme.textColor

        implicitWidth: label.implicitWidth + Kirigami.Units.smallSpacing * 1.5
        implicitHeight: label.implicitHeight
        radius: height / 2
        color: Kirigami.Theme.alternateBackgroundColor
        border.width: 1
        border.color: Kirigami.Theme.disabledTextColor

        QQC2.Label {
            id: label
            anchors.centerIn: parent
            font: Kirigami.Theme.smallFont
            color: pill.textColor
        }
    }
}
