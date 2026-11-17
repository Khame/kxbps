// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// One external action in the About page.
//
// Each action is its own rounded card rather than a row inside a shared frame:
// the page groups everything in cards, and three rows stacked with separators
// inside one card read as a cramped list instead of three separate actions. The
// whole card is the button, so the target area matches what the eye sees.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Kirigami.AbstractCard {
    id: card

    property string iconSource: ""
    property string label: ""
    property url target

    Layout.fillWidth: true
    showClickFeedback: true
    Accessible.role: Accessible.Button
    Accessible.name: label
    // The dialog takes the initial focus itself; Tab still reaches every card.
    focusPolicy: Qt.TabFocus
    activeFocusOnTab: true
    onClicked: Qt.openUrlExternally(target)

    contentItem: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            source: card.iconSource
            isMask: true
            color: Kirigami.Theme.textColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: card.label
        }

        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            source: "external-link"
            isMask: true
            color: Kirigami.Theme.disabledTextColor
        }
    }
}
