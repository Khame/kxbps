// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// One mirror: the selection dot, server and country, region, latency and state.
//
// The row itself is the "Use" control: there is one source of truth for the
// selection (`mirrorBackend.currentMirror`) and no second checkbox that could
// disagree with it.
//
// Two different questions share this row, so they get two different badges:
// `Selected` is the app's own choice (the store's `mirror.json`) and `In use`
// is what xbps resolves from `/etc/xbps.d/00-repository-main.conf`. Both can be
// true, and when neither is the system mirror the list is `pending` an apply.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "../Tint.js" as Tint
import org.kde.ki18n 1.0

QQC2.ItemDelegate {
    id: delegate

    property var mirrorBackend
    /// The view, which owns the column widths; the heading and the rows read the
    /// same numbers, so a column can never be one width above and another below.
    property var columns

    readonly property bool selected: model.url === mirrorBackend.currentMirror
    // Both sides are base URLs ending in one `/`, the shape `mirror.json` and
    // `00-repository-main.conf` store, so equality is the same comparison the
    // backend's `pending()` makes.
    readonly property bool inUse: mirrorBackend.systemMirror.length > 0
                                  && model.url === mirrorBackend.systemMirror

    // The same row height as the package table: one line of text per row, so a
    // list of mirrors reads as a table and not as a stack of cards.
    height: Kirigami.Units.gridUnit * 2
    implicitHeight: height
    // A gutter as wide as the package table's checkbox cell: a mirror row has
    // no checkbox, so without it the server name would start at the window edge
    // and the two tables would not read as the same list.
    leftPadding: Kirigami.Units.gridUnit * 2
    rightPadding: Kirigami.Units.largeSpacing
    topPadding: 0
    bottomPadding: 0
    // No `highlighted`: the style paints that as a solid highlight band, which
    // turns one row into a bright stripe. The selection is the same 20 % tint
    // the sidebar uses, with an accent bar on the leading edge, so the list
    // reads as a table rather than as a warning.
    background: Rectangle {
        color: delegate.selected
               ? Tint.wash(Kirigami.Theme.highlightColor, Tint.selection)
               : (delegate.hovered
                  ? Tint.wash(Kirigami.Theme.highlightColor, Tint.hover)
                  : "transparent")

        // The accent of the chosen mirror, on the leading edge. It sits in the
        // background, not in the row's layout: a bar that appears and vanishes
        // with the selection would shift every column of the row.
        Rectangle {
            visible: delegate.selected
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: Kirigami.Units.smallSpacing / 2
            color: Kirigami.Theme.highlightColor
        }

        // The hairline between rows, from the theme, exactly as the package
        // table draws it.
        Kirigami.Separator {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
        }
    }

    Accessible.role: Accessible.RadioButton
    Accessible.name: model.country.length > 0
                     ? i18nd("kxbps", "%1, %2", model.name, model.country)
                     : model.name
    Accessible.checked: delegate.selected
    // The badges and the state are painted next to the row, so a screen reader
    // would read them as separate items with no owner. The description carries
    // them with the row instead.
    Accessible.description: {
        let parts = []
        if (delegate.selected) {
            parts.push(i18nd("kxbps", "Selected"))
        }
        if (delegate.inUse) {
            parts.push(i18nd("kxbps", "In use"))
        }
        parts.push(delegate.stateText(model.state, model.errorKey))
        return parts.join(", ")
    }

    // A row is a selection, not a transaction: the toolbar's `Apply to system`
    // is what changes the machine, so a stray click cannot point xbps at a
    // mirror on the other side of the world.
    onClicked: mirrorBackend.currentMirror = model.url

    function latencyText(state, ms) {
        return state === "ok" ? i18nd("kxbps", "%1 ms", ms) : ""
    }

    function stateText(state, errorKey) {
        switch (state) {
        case "idle":
            return i18nd("kxbps", "Not probed")
        case "running":
            return i18nd("kxbps", "Probing")
        case "ok":
            return i18nd("kxbps", "OK")
        }
        switch (errorKey) {
        case "plainHttp":
            return i18nd("kxbps", "Plain HTTP is not probed")
        case "timeout":
            return i18nd("kxbps", "Timed out")
        case "noResponse":
            return i18nd("kxbps", "No response")
        }
        return i18nd("kxbps", "Failed")
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing

        // Server and country on one line: the name takes what it needs and the
        // country follows it in the muted step, the way a package row puts its
        // description next to the name.
        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            // A nested layout can only grow as far as its children allow, and an
            // invisible child allows nothing: without this line a row with no
            // country could not widen this cell, so its address, region, tier and
            // state all jumped left by the width the cell failed to take.
            Layout.maximumWidth: Number.POSITIVE_INFINITY
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                text: model.name
                font.bold: true
                elide: Text.ElideRight
            }

            // The country sits right after the name, not against the column's
            // right edge: pushed right it read as the first half of the region
            // column, and the "Region" heading looked misaligned.
            QQC2.Label {
                Layout.fillWidth: true
                text: model.country
                visible: text.length > 0
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        // The address xbps would be pointed at. A place name does not identify a
        // mirror — two rows can say "Beijing" — so the row carries the URL too,
        // elided, with the whole string in a tooltip.
        QQC2.Label {
            Layout.preferredWidth: columns.addressWidth
            Layout.minimumWidth: columns.addressWidth
            Layout.maximumWidth: columns.addressWidth
            text: model.url
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight

            HoverHandler {
                id: addressHover
            }

            QQC2.ToolTip.visible: addressHover.hovered
            QQC2.ToolTip.text: model.url
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        }

        QQC2.Label {
            Layout.preferredWidth: columns.regionWidth
            Layout.minimumWidth: columns.regionWidth
            Layout.maximumWidth: columns.regionWidth
            // Region tokens are data, not UI copy.
            text: model.region
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        // `T1` / `T2`, the same token `xmirror` prints: tier 1 is run by the Void
        // infrastructure team, tier 2 is a community mirror.
        QQC2.Label {
            Layout.preferredWidth: columns.tierWidth
            Layout.minimumWidth: columns.tierWidth
            Layout.maximumWidth: columns.tierWidth
            text: "T" + model.tier
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        Item {
            Layout.preferredWidth: columns.stateWidth
            Layout.minimumWidth: columns.stateWidth
            Layout.maximumWidth: columns.stateWidth
            Layout.fillHeight: true
            clip: true

            RowLayout {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                spacing: Kirigami.Units.smallSpacing

                // The state first: "not probed" is what the row says about
                // itself, and the two badges are extra facts about it.
                Rectangle {
                    implicitWidth: stateLabel.implicitWidth + Kirigami.Units.smallSpacing * 2
                    implicitHeight: stateLabel.implicitHeight + Kirigami.Units.smallSpacing
                    radius: height / 2
                    color: Kirigami.Theme.alternateBackgroundColor
                    border.width: 1
                    border.color: model.state === "fail" ? Kirigami.Theme.negativeTextColor
                                                         : Kirigami.Theme.disabledTextColor

                    QQC2.Label {
                        id: stateLabel
                        anchors.centerIn: parent
                        text: delegate.stateText(model.state, model.errorKey)
                        font: Kirigami.Theme.smallFont
                        color: model.state === "fail" ? Kirigami.Theme.negativeTextColor
                                                      : (model.state === "ok" ? Kirigami.Theme.positiveTextColor
                                                                              : Kirigami.Theme.disabledTextColor)
                    }
                }

                QQC2.Label {
                    visible: delegate.selected
                    text: i18nd("kxbps", "Selected")
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.highlightColor
                }

                QQC2.Label {
                    visible: delegate.inUse
                    text: i18nd("kxbps", "In use")
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.positiveTextColor
                }
            }
        }

        // Last column: what the sweep measured, next to the state that says
        // whether the number means anything.
        QQC2.Label {
            // The same width the heading asks for, or the two columns drift.
            Layout.preferredWidth: columns.latencyWidth
            Layout.minimumWidth: columns.latencyWidth
            Layout.maximumWidth: columns.latencyWidth
            horizontalAlignment: Text.AlignRight
            text: delegate.latencyText(model.state, model.latencyMs)
            font.family: "monospace"
        }

    }
}
