// One entry of the navigation sidebar.
//
// A plain `QQC2.ItemDelegate`: Kirigami 6.11 no longer ships
// `Kirigami.AbstractListItem`, and the delegate's own `highlighted` background
// already uses the theme's highlight colour, so selection follows the system
// palette in both light and dark mode.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

QQC2.ItemDelegate {
    id: item

    property string label
    // The counters are only interesting when non-zero, so an empty string
    // hides the badge instead of reserving room for a "0".
    property string badgeText: ""
    property bool badgeAlert: false

    implicitHeight: Kirigami.Units.gridUnit * 2
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing

    Accessible.role: Accessible.PageTab
    Accessible.name: item.label
    Accessible.selected: item.highlighted

    contentItem: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        QQC2.Label {
            Layout.fillWidth: true
            text: item.label
            elide: Text.ElideRight
            color: item.highlighted ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
        }

        QQC2.Label {
            visible: item.badgeText.length > 0
            text: item.badgeText
            font: Kirigami.Theme.smallFont
            color: item.badgeAlert ? Kirigami.Theme.negativeTextColor : Kirigami.Theme.disabledTextColor
        }
    }
}
