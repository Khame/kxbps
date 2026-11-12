// One package row: name, version, size, state and the two queue actions in
// aligned columns.
//
// This is the mockup's update table, not its app cards: a package list is a
// tool surface, so the same fields line up from row to row and the eye can scan
// versions and sizes without reading every name. The widths are passed in by
// `PackagesView`, which draws the header with the exact same values.
//
// Nothing here starts a transaction: the queue is the only path to one.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.SwipeListItem {
    id: delegate

    property var installedBackend
    property real versionWidth
    property real sizeWidth
    property real stateWidth
    // Driven by the selected name, not `ListView.currentIndex`: a filter change
    // resets the model and would drop the current index, while the details pane
    // keeps showing the same package.
    property bool selected: false

    readonly property bool queued: model.queuedAction !== ""
    readonly property bool installed: model.installedVersion !== ""
    // What queueing this row means. An update is its own action: queueing an
    // installed package as "install" would plan a reinstall, not the update the
    // row announces.
    readonly property string queueToken: model.isUpdate ? "update" : (delegate.installed ? "remove" : "install")

    height: Kirigami.Units.gridUnit * 2.4
    implicitHeight: height
    leftPadding: Kirigami.Units.mediumSpacing
    rightPadding: Kirigami.Units.mediumSpacing
    topPadding: 0
    bottomPadding: 0
    // The current index covers keyboard navigation; the selected name keeps the
    // row marked after a filter change resets the model.
    highlighted: delegate.selected || ListView.isCurrentItem

    Accessible.role: Accessible.ListItem
    Accessible.name: model.name
    Accessible.selected: delegate.highlighted

    // Swipe reveals the same two actions the hover slot offers.
    swipe.enabled: true
    swipe.left: QQC2.ToolButton {
        icon.name: delegate.queued ? "list-remove"
                  : (model.isUpdate ? "view-refresh"
                                    : (delegate.installed ? "list-remove" : "list-add"))
        text: delegate.queueActionLabel(delegate.queued ? "unqueued" : delegate.queueToken)
        onClicked: {
            delegate.toggleQueue()
            delegate.swipe.close()
        }
    }
    swipe.right: QQC2.ToolButton {
        enabled: delegate.installed
        icon.name: model.held ? "object-unlocked" : "object-locked"
        text: model.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
        onClicked: {
            delegate.installedBackend.toggleHold(model.name)
            delegate.swipe.close()
        }
    }

    // One row, one queue decision: off if it is already queued, otherwise the
    // action that makes sense for the installed state.
    function toggleQueue() {
        if (delegate.queued) {
            delegate.installedBackend.removeQueued(model.name)
        } else {
            delegate.installedBackend.toggleQueue(model.name, delegate.queueToken)
        }
    }

    function queueActionLabel(action) {
        switch (action) {
        case "install":
            return i18nd("kxbps", "Install")
        case "remove":
            return i18nd("kxbps", "Remove")
        case "update":
            return i18nd("kxbps", "Update")
        case "unqueued":
            return i18nd("kxbps", "Unqueue")
        }
        return action
    }

    contentItem: RowLayout {
        // Tight gaps: at the 56 gridUnit default the name column has ~150 px
        // for the longest name, and largeSpacing would cost 12 px of it.
        spacing: Kirigami.Units.smallSpacing

        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                text: model.name
                font.bold: true
                elide: Text.ElideRight
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
                    visible: delegate.installed && !model.isUpdate
                    text: model.installedVersion + " ✓"
                    color: Kirigami.Theme.positiveTextColor
                    font.family: "monospace"
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    visible: delegate.installed && model.isUpdate
                    text: model.installedVersion
                    color: Kirigami.Theme.positiveTextColor
                    font.family: "monospace"
                }

                QQC2.Label {
                    visible: delegate.installed && model.isUpdate
                    text: "→"
                    color: Kirigami.Theme.neutralTextColor
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: delegate.installed && model.isUpdate
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
            text: model.isUpdate ? model.updateSize : model.size
            font.family: "monospace"
            elide: Text.ElideLeft

            QQC2.ToolTip.visible: sizeHover.containsMouse
            QQC2.ToolTip.text: model.isUpdate
                ? i18nd("kxbps", "Installed %1, download %2, new version %3",
                        model.size, model.download, model.updateSize)
                : i18nd("kxbps", "Installed %1", model.size)

            MouseArea {
                id: sizeHover
                anchors.fill: parent
                hoverEnabled: true
            }
        }

        Item {
            Layout.preferredWidth: delegate.stateWidth
            Layout.minimumWidth: delegate.stateWidth
            Layout.maximumWidth: delegate.stateWidth
            Layout.fillHeight: true
            clip: true

            RowLayout {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                spacing: Kirigami.Units.smallSpacing

                Pill {
                    id: heldPill
                    visible: model.held
                    text: i18nd("kxbps", "Held")
                }

                // The queue pill outranks the static Explicit label: it is the
                // state the user just changed. A pill only appears when it
                // fits; the row height is fixed, so the third one is hidden
                // instead of growing the row.
                Pill {
                    id: queuedPill
                    visible: delegate.queued
                             && (heldPill.visible ? heldPill.implicitWidth + Kirigami.Units.smallSpacing : 0)
                                + queuedPill.implicitWidth <= delegate.stateWidth
                    text: delegate.queueActionLabel(model.queuedAction)
                    textColor: Kirigami.Theme.highlightColor
                }

                Pill {
                    id: explicitPill
                    visible: model.explicit
                             && (heldPill.visible ? heldPill.implicitWidth + Kirigami.Units.smallSpacing : 0)
                                + (queuedPill.visible ? queuedPill.implicitWidth + Kirigami.Units.smallSpacing : 0)
                                + explicitPill.implicitWidth <= delegate.stateWidth
                    text: i18nd("kxbps", "Explicit")
                }
            }
        }

    }

    // The two row actions sit on top of the row's right edge instead of owning a
    // column: they are only useful under the pointer, and the name needs the
    // room. The solid background keeps the cells behind them from showing
    // through.
    Rectangle {
        anchors.right: parent.right
        anchors.rightMargin: delegate.rightPadding
        anchors.verticalCenter: parent.verticalCenter
        width: actionsRow.implicitWidth + Kirigami.Units.smallSpacing * 2
        height: delegate.height
        // Hover and keyboard focus only: the selected row already has its
        // details open, and its state pills must stay readable.
        visible: delegate.hovered || delegate.activeFocus
        z: 2
        color: delegate.highlighted ? Kirigami.Theme.highlightColor : Kirigami.Theme.backgroundColor

        RowLayout {
            id: actionsRow
            anchors.centerIn: parent
            spacing: Kirigami.Units.smallSpacing

            QQC2.ToolButton {
                display: QQC2.AbstractButton.IconOnly
                icon.name: delegate.queued ? "list-remove"
                          : (model.isUpdate ? "view-refresh"
                                            : (delegate.installed ? "list-remove" : "list-add"))
                text: delegate.queueActionLabel(delegate.queued ? "unqueued" : delegate.queueToken)
                Accessible.name: text
                onClicked: delegate.toggleQueue()
            }

            QQC2.ToolButton {
                display: QQC2.AbstractButton.IconOnly
                enabled: delegate.installed
                icon.name: model.held ? "object-unlocked" : "object-locked"
                text: model.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
                Accessible.name: text
                onClicked: delegate.installedBackend.toggleHold(model.name)
            }
        }
    }

    component Pill: Rectangle {
        id: pill

        property alias text: label.text
        property color textColor: Kirigami.Theme.textColor

        implicitWidth: label.implicitWidth + Kirigami.Units.smallSpacing * 2
        implicitHeight: label.implicitHeight + Kirigami.Units.smallSpacing
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
