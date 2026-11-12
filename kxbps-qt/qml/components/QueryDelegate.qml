// One `xbps-query` hit: the package row without the queue actions.
//
// Query results are `QVariantMap`s handed over by the query backend, so the
// fields arrive as `modelData`, not as model roles. A `-Rs` line carries the
// package name, its version and its description, and nothing else: the
// installed state is the tag, so a hit is either "Installed" or plain, never a
// repository or a category xbps did not print. Widths come from the view so
// the header and every result line up.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

QQC2.ItemDelegate {
    id: delegate

    property real versionWidth
    // The row whose facts the pane is showing; the view tracks it by pkgver.
    property bool current: false

    highlighted: delegate.current

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
                text: modelData.installed ? i18nd("kxbps", "Installed") : ""
                font: Kirigami.Theme.smallFont
                color: modelData.installed ? Kirigami.Theme.positiveTextColor
                                           : Kirigami.Theme.disabledTextColor
                visible: modelData.installed
                elide: Text.ElideRight
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: modelData.short
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        QQC2.Label {
            Layout.preferredWidth: delegate.versionWidth
            Layout.minimumWidth: delegate.versionWidth
            Layout.maximumWidth: delegate.versionWidth
            text: modelData.version
            font.family: "monospace"
            elide: Text.ElideRight
            color: modelData.installed ? Kirigami.Theme.positiveTextColor
                                       : Kirigami.Theme.textColor
        }
    }
}
