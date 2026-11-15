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
import org.kde.ki18n 1.0

QQC2.ItemDelegate {
    id: delegate

    property var mirrorBackend

    readonly property bool selected: model.url === mirrorBackend.currentMirror
    // Both sides are base URLs ending in one `/`, the shape `mirror.json` and
    // `00-repository-main.conf` store, so equality is the same comparison the
    // backend's `pending()` makes.
    readonly property bool inUse: mirrorBackend.systemMirror.length > 0
                                  && model.url === mirrorBackend.systemMirror

    height: Kirigami.Units.gridUnit * 2.4
    implicitHeight: height
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing
    topPadding: 0
    bottomPadding: 0
    // No `highlighted`: the style paints that as a solid highlight band, which
    // turns one row into a bright stripe. The selection is the same 20 % tint
    // the sidebar uses, with an accent bar on the leading edge, so the list
    // reads as a table rather than as a warning.
    background: Rectangle {
        color: delegate.selected
               ? Qt.rgba(Kirigami.Theme.highlightColor.r,
                         Kirigami.Theme.highlightColor.g,
                         Kirigami.Theme.highlightColor.b, 0.2)
               : (delegate.hovered ? Qt.rgba(Kirigami.Theme.highlightColor.r,
                                             Kirigami.Theme.highlightColor.g,
                                             Kirigami.Theme.highlightColor.b, 0.1)
                                   : "transparent")
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

        // The accent of the chosen mirror, on the leading edge. It is the same
        // signal the sidebar gives for the current page.
        Rectangle {
            visible: delegate.selected
            Layout.preferredWidth: Math.round(Kirigami.Units.smallSpacing / 2)
            Layout.fillHeight: true
            Layout.leftMargin: -delegate.leftPadding
            color: Kirigami.Theme.highlightColor
        }

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

            QQC2.Label {
                Layout.fillWidth: true
                text: model.country.length > 0 ? model.country : model.region
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        QQC2.Label {
            Layout.preferredWidth: Kirigami.Units.gridUnit * 6
            Layout.minimumWidth: Kirigami.Units.gridUnit * 6
            Layout.maximumWidth: Kirigami.Units.gridUnit * 6
            // Region tokens are data, not UI copy.
            text: model.region
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            // The same width the heading asks for, or the two columns drift.
            Layout.preferredWidth: Kirigami.Units.gridUnit * 7
            Layout.minimumWidth: Kirigami.Units.gridUnit * 7
            Layout.maximumWidth: Kirigami.Units.gridUnit * 7
            horizontalAlignment: Text.AlignRight
            text: delegate.latencyText(model.state, model.latencyMs)
            font.family: "monospace"
        }

        Item {
            Layout.preferredWidth: Kirigami.Units.gridUnit * 16
            Layout.minimumWidth: Kirigami.Units.gridUnit * 16
            Layout.maximumWidth: Kirigami.Units.gridUnit * 16
            Layout.fillHeight: true
            clip: true

            RowLayout {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                spacing: Kirigami.Units.smallSpacing

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
            }
        }

    }
}
