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

    // A `-Rs` hit is installed when the snapshot knows its version; the search
    // line itself only carries the `[*]`/`[-]` tag.
    readonly property bool installed: modelData.installedVersion !== undefined
                                        && modelData.installedVersion !== ""

    highlighted: delegate.current

    // The row is two lines for a plain hit and three for an installed one
    // (name, the Installed tag, the description); a fixed height clipped the
    // labels into each other once every hit carried a description.
    height: Math.max(Kirigami.Units.gridUnit * 2.4,
                     row.implicitHeight + topPadding + bottomPadding)
    implicitHeight: height
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing
    topPadding: 0
    bottomPadding: 0

    Accessible.role: Accessible.ListItem
    Accessible.name: modelData.name

    contentItem: RowLayout {
        id: row
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
                text: delegate.installed ? i18nd("kxbps", "Installed") : ""
                font: Kirigami.Theme.smallFont
                color: delegate.installed ? Kirigami.Theme.positiveTextColor
                                           : Kirigami.Theme.disabledTextColor
                visible: delegate.installed
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
            color: delegate.installed ? Kirigami.Theme.positiveTextColor
                                       : Kirigami.Theme.textColor
        }
    }
}
