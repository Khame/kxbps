// One entry of the navigation sidebar.
//
// A plain `QQC2.ItemDelegate`: Kirigami 6.11 no longer ships
// `Kirigami.AbstractListItem`, and the delegate's own `highlighted` background
// already uses the theme's highlight colour, so selection follows the system
// palette in both light and dark mode.
//
// The sidebar is navigation, not decoration, so the row carries the three parts
// a navigation row needs and nothing else: a Breeze icon, the label, and the
// counter that makes the entry worth clicking. `compact` is the variant without
// an icon, used for the informational repository list.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

QQC2.ItemDelegate {
    id: item

    property string label
    // A KXbps symbolic icon name, without the `kxbps-` prefix and the
    // `-symbolic.svg` suffix. Empty draws no icon and the label takes the space.
    property string iconName: ""
    // The counters are only interesting when non-zero, so an empty string
    // hides the badge instead of reserving room for a "0".
    property string badgeText: ""
    property bool badgeAlert: false
    // A flat row: no icon, small font, for a value the sidebar only reports.
    property bool compact: false
    // A row that reports state instead of navigating: no hover, no accent.
    property bool muted: false

    implicitHeight: item.compact ? Kirigami.Units.gridUnit * 1.5
                                : Kirigami.Units.gridUnit * 2
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing

    Accessible.role: Accessible.PageTab
    Accessible.name: item.label
    Accessible.selected: item.highlighted

    // The default delegate background is the style's button colour, which reads
    // as a raised control; a navigation entry is a flat bar with an accent when
    // current, like every Plasma sidebar.
    background: Rectangle {
        color: item.highlighted && !item.muted
               ? Qt.rgba(Kirigami.Theme.highlightColor.r,
                         Kirigami.Theme.highlightColor.g,
                         Kirigami.Theme.highlightColor.b, 0.2)
               : (item.hovered && !item.muted ? Qt.rgba(Kirigami.Theme.highlightColor.r,
                                                       Kirigami.Theme.highlightColor.g,
                                                       Kirigami.Theme.highlightColor.b, 0.1)
                                              : "transparent")
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        // The accent bar of the current entry, on the window edge. It is the
        // same signal the highlight colour gives, for a palette where the
        // highlight tint is faint.
        Rectangle {
            visible: item.highlighted && !item.muted
            Layout.preferredWidth: Math.round(Kirigami.Units.smallSpacing / 2)
            Layout.fillHeight: true
            Layout.leftMargin: -item.leftPadding
            color: Kirigami.Theme.highlightColor
        }

        KxbpsIcon {
            visible: !item.compact && item.iconName.length > 0
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            symbolicName: item.iconName
            // The selected row is a 20% tint, not a full highlight: normal text
            // colour is the one that keeps its contrast.
            highlighted: false
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: item.label
            elide: Text.ElideRight
            font: item.compact ? Kirigami.Theme.smallFont : Kirigami.Theme.defaultFont
            color: item.muted ? Kirigami.Theme.disabledTextColor
                              : Kirigami.Theme.textColor
        }

        QQC2.Label {
            visible: item.badgeText.length > 0
            text: item.badgeText
            font: Kirigami.Theme.smallFont
            color: item.badgeAlert ? Kirigami.Theme.negativeTextColor
                                   : Kirigami.Theme.disabledTextColor
        }
    }
}
