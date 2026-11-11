// One `xbps-query` hit: the package row without the queue actions.
//
// Query results are `QVariantMap`s handed over by the catalog backend, so the
// fields arrive as `modelData`, not as model roles. Widths come from the view
// so the header and every result line up.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

QQC2.ItemDelegate {
    id: delegate

    property real versionWidth
    property real sizeWidth

    height: Kirigami.Units.gridUnit * 2.4
    implicitHeight: height
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing
    topPadding: 0
    bottomPadding: 0

    Accessible.role: Accessible.ListItem
    Accessible.name: modelData.name

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing

        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: 0

            QQC2.Label {
                Layout.fillWidth: true
                text: modelData.name
                font.bold: true
                elide: Text.ElideRight
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: modelData.repo + " · " + modelData.category
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        QQC2.Label {
            Layout.preferredWidth: delegate.versionWidth
            Layout.minimumWidth: delegate.versionWidth
            Layout.maximumWidth: delegate.versionWidth
            text: modelData.installedVersion !== ""
                  ? modelData.installedVersion + " ✓"
                  : modelData.version
            color: modelData.installedVersion !== "" ? Kirigami.Theme.positiveTextColor
                                                     : Kirigami.Theme.textColor
            font.family: "monospace"
            elide: Text.ElideRight
        }

        QQC2.Label {
            Layout.preferredWidth: delegate.sizeWidth
            Layout.minimumWidth: delegate.sizeWidth
            Layout.maximumWidth: delegate.sizeWidth
            horizontalAlignment: Text.AlignRight
            text: modelData.size
            font.family: "monospace"
            elide: Text.ElideLeft
        }
    }
}
