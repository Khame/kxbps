// One row of the queue: what Apply will do to one package.
//
// The row states the action, the version and the size it moves, so the queue
// can be reviewed without opening anything. The × is the only control: it takes
// the package out of the queue, the same gesture as unchecking it in the table.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "../Tint.js" as Tint
import org.kde.ki18n 1.0
import "../icons/IconNames.js" as Icons

QQC2.ItemDelegate {
    id: row

    property var installedBackend

    // The row's own identity, as real properties: `model` is a context property
    // injected into this file's bindings only.
    readonly property string packageName: model.name === undefined ? "" : model.name
    // The queue model's own role is `action`, not the package table's
    // `queuedAction`: the two models describe the same store, not the same row.
    readonly property string packageQueuedAction: model.action === undefined ? "" : model.action
    readonly property bool removal: row.packageQueuedAction === "remove"
    // The backend fills these from the read: the version the update would
    // install, or the installed one a removal takes away, and the bytes the
    // transaction moves. Empty means the read does not know, never zero.
    readonly property string versionText: model.version !== undefined && model.version !== ""
                                           ? model.version : row.dash
    readonly property string sizeText: model.size !== undefined && model.size !== ""
                                       ? model.size : row.dash
    readonly property string actionWord: {
        switch (row.packageQueuedAction) {
        case "remove":
            return i18nd("kxbps", "Removal")
        case "update":
            return i18nd("kxbps", "Update")
        }
        return i18nd("kxbps", "Installation")
    }
    // A minus for what leaves the system, a plus for what arrives.
    readonly property string sign: row.removal ? "−" : "+"
    readonly property string dash: "—"

    implicitHeight: Kirigami.Units.gridUnit * 2
    height: implicitHeight
    leftPadding: Kirigami.Units.mediumSpacing
    rightPadding: Kirigami.Units.mediumSpacing
    topPadding: 0
    bottomPadding: 0

    Accessible.role: Accessible.ListItem
    Accessible.name: row.packageName
    Accessible.description: row.actionWord + " · " + row.versionText + " · " + row.sizeText
    background: Rectangle {
        color: row.hovered
               ? Tint.wash(Kirigami.Theme.highlightColor, Tint.hover)
               : "transparent"

        Kirigami.Separator {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
        }
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            // Fixed lane so every sign sits over the next.
            Layout.preferredWidth: Kirigami.Units.gridUnit
            text: row.sign
            font.bold: true
            color: row.removal ? Kirigami.Theme.negativeTextColor
                               : Kirigami.Theme.positiveTextColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: row.packageName
            font.bold: true
            elide: Text.ElideRight
        }

        QQC2.Label {
            text: row.actionWord
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            Layout.preferredWidth: Kirigami.Units.gridUnit * 9
            horizontalAlignment: Text.AlignRight
            text: row.versionText
            font.family: "monospace"
            elide: Text.ElideLeft
        }

        QQC2.Label {
            Layout.preferredWidth: Kirigami.Units.gridUnit * 6
            horizontalAlignment: Text.AlignRight
            text: row.sizeText
            font.family: "monospace"
            elide: Text.ElideLeft
        }

        QQC2.ToolButton {
            icon.name: Icons.clear
            text: i18nd("kxbps", "Remove from the queue")
            display: QQC2.AbstractButton.IconOnly
            Accessible.name: i18nd("kxbps", "Remove %1 from the queue", row.packageName)
            onClicked: {
                if (row.installedBackend !== null && row.installedBackend !== undefined) {
                    row.installedBackend.removeQueued(row.packageName)
                }
            }
        }
    }
}
