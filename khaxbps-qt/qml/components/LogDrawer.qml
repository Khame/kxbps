// The collapsible, resizable `xbps` log below the status strip.
//
// Lines are raw command output and stay untranslated; only the header and the
// state word are copy. The drawer is closed by default and opens itself when a
// transaction starts, because that is the moment the output becomes useful.
//
// A plain Item and a MouseArea rather than `SplitView`: there is exactly one
// split here, and the handle has to clamp to the window so the package list
// never disappears behind the log.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Item {
    id: drawer

    property var job
    property bool expanded: false
    // The height the user dragged to; collapsed and clamped by the properties
    // below. Not persisted in Task 10.
    property real expandedHeight: Kirigami.Units.gridUnit * 10

    readonly property bool hasJob: job !== null
    readonly property real headerHeight: Kirigami.Units.gridUnit * 2
    readonly property real collapsedHeight: headerHeight
    readonly property real maximumHeight: Math.min(Kirigami.Units.gridUnit * 18,
                                                   (Window.window ? Window.window.height : Kirigami.Units.gridUnit * 36) * 0.6)

    readonly property string stateWord: {
        if (!hasJob) {
            return i18nd("khaxbps", "idle")
        }
        if (!job.done) {
            return i18nd("khaxbps", "running")
        }
        return job.cancelled ? i18nd("khaxbps", "cancelled") : i18nd("khaxbps", "finished")
    }

    implicitHeight: expanded && hasJob ? Math.max(collapsedHeight, Math.min(expandedHeight, maximumHeight))
                                       : collapsedHeight
    height: implicitHeight

    QQC2.ItemDelegate {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: drawer.headerHeight
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing

        // A single click toggles after the double-click interval has passed, so
        // a double click does not toggle twice and land where it started.
        onClicked: toggleTimer.restart()
        onDoubleClicked: {
            toggleTimer.stop()
            drawer.expanded = !drawer.expanded
        }

        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                text: i18nd("khaxbps", "xbps log")
                font.bold: true
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: drawer.stateWord
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                text: drawer.expanded ? i18nd("khaxbps", "Hide") : i18nd("khaxbps", "Show")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
        }
    }

    Timer {
        id: toggleTimer
        interval: 200
        onTriggered: drawer.expanded = !drawer.expanded
    }

    // The resize handle on the top edge: dragging up shows more log, dragging
    // down hides it. Double-clicking the header is the keyboard-free shortcut.
    Rectangle {
        id: handle
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: Kirigami.Units.smallSpacing
        color: Kirigami.Theme.alternateBackgroundColor
        z: 2

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            height: 1
            color: Kirigami.Theme.disabledTextColor
        }

        MouseArea {
            id: dragArea
            anchors.fill: parent
            cursorShape: Qt.SizeVerCursor
            preventStealing: true

            property real startY: 0
            property real startHeight: 0

            onPressed: (mouse) => {
                dragArea.startY = mouse.y
                dragArea.startHeight = drawer.expanded ? drawer.expandedHeight : drawer.collapsedHeight
                drawer.expanded = true
                drawer.expandedHeight = dragArea.startHeight
            }
            onPositionChanged: (mouse) => {
                if (!dragArea.pressed) {
                    return
                }
                const delta = dragArea.startY - mouse.y
                drawer.expandedHeight = Math.max(drawer.collapsedHeight,
                                                 Math.min(drawer.maximumHeight,
                                                          dragArea.startHeight + delta))
            }
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        visible: drawer.expanded && drawer.hasJob
        clip: true
        color: Kirigami.Theme.alternateBackgroundColor

        ListView {
            id: logList
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing
            clip: true
            model: drawer.hasJob ? drawer.job.lines : null
            onCountChanged: positionViewAtEnd()

            delegate: QQC2.Label {
                width: logList.width
                text: model.line
                font.family: "monospace"
                elide: Text.ElideRight
            }
        }
    }
}
