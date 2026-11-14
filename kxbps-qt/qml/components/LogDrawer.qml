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
    /// The lines the update checks printed. A check is not a transaction and
    /// has no job, so its output arrives here; the drawer shows it until a
    /// transaction exists, because that is the newer news.
    property var checkLines
    property bool expanded: false
    // The height the user dragged to; collapsed and clamped by the properties
    // below. Not persisted in Task 10.
    property real expandedHeight: Kirigami.Units.gridUnit * 10

    readonly property bool hasJob: job !== null
    readonly property bool hasLines: hasJob
                                      || (checkLines !== null && checkLines.count > 0)
    readonly property real headerHeight: Kirigami.Units.gridUnit * 2
    // Collapsed, the drawer takes no room at all: the window's app bar carries
    // the toggle, so a second header row would only repeat it.
    readonly property real collapsedHeight: 0
    readonly property real maximumHeight: Math.min(Kirigami.Units.gridUnit * 18,
                                                   (Window.window ? Window.window.height : Kirigami.Units.gridUnit * 36) * 0.6)

    readonly property string stateWord: {
        if (!hasJob) {
            return i18nd("kxbps", "idle")
        }
        if (!job.done) {
            return i18nd("kxbps", "running")
        }
        if (job.failed) {
            return i18nd("kxbps", "failed")
        }
        return job.cancelled ? i18nd("kxbps", "cancelled") : i18nd("kxbps", "finished")
    }

    implicitHeight: expanded ? Math.max(headerHeight, Math.min(expandedHeight, maximumHeight))
                             : 0
    height: implicitHeight

    // Dragging to the minimum leaves `expandedHeight` at the header height.
    // Expanding again must restore a readable body, not just flip the flag.
    function expand() {
        if (expandedHeight <= collapsedHeight) {
            expandedHeight = Kirigami.Units.gridUnit * 10
        }
        expanded = true
    }

    function toggleExpanded() {
        if (expanded) {
            expanded = false
        } else {
            expand()
        }
    }

    QQC2.ItemDelegate {
        id: header
        anchors.left: parent.left
        anchors.right: parent.right
        // The drawer grows upward from the bottom edge, so its bar belongs at
        // that edge: the log opens above the control that opened it.
        anchors.bottom: parent.bottom
        height: drawer.headerHeight
        visible: drawer.expanded
        leftPadding: Kirigami.Units.largeSpacing
        rightPadding: Kirigami.Units.largeSpacing

        // A single click toggles after the double-click interval has passed, so
        // a double click does not toggle twice and land where it started.
        onClicked: toggleTimer.restart()
        onDoubleClicked: {
            toggleTimer.stop()
            drawer.toggleExpanded()
        }

        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                text: i18nd("kxbps", "Log")
                font.bold: true
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: drawer.stateWord
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                text: drawer.expanded ? i18nd("kxbps", "Hide") : i18nd("kxbps", "Show")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
        }
    }

    Timer {
        id: toggleTimer
        // The platform's own double-click window, so a single click does not
        // fire a toggle in the middle of a double click.
        interval: Qt.application.styleHints.mouseDoubleClickInterval
        onTriggered: drawer.toggleExpanded()
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
        visible: drawer.expanded
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

            // Scene coordinates: the handle itself moves up as the drawer grows,
            // so comparing its local `mouse.y` would move the edge at half the
            // cursor's speed and detach it from the pointer.
            property real startSceneY: 0
            property real startHeight: 0

            onPressed: (mouse) => {
                dragArea.startSceneY = dragArea.mapToItem(null, mouse.x, mouse.y).y
                dragArea.startHeight = drawer.expanded ? drawer.expandedHeight : drawer.collapsedHeight
            }
            onPositionChanged: (mouse) => {
                if (!dragArea.pressed) {
                    return
                }
                const delta = dragArea.startSceneY - dragArea.mapToItem(null, mouse.x, mouse.y).y
                // A click without movement must not open the drawer; only a
                // drag up from the collapsed state does.
                if (delta <= 0 && !drawer.expanded) {
                    return
                }
                const next = Math.max(drawer.collapsedHeight,
                                      Math.min(drawer.maximumHeight, dragArea.startHeight + delta))
                drawer.expandedHeight = next
                drawer.expanded = next > drawer.collapsedHeight
            }
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: header.top
        visible: drawer.expanded
        clip: true
        color: Kirigami.Theme.alternateBackgroundColor

        // The drawer opens before any transaction has run: a control that does
        // nothing when clicked reads as broken, so it says why it is empty.
        QQC2.Label {
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 2
            visible: !drawer.hasLines
            text: i18nd("kxbps", "No transaction has run in this session yet.")
            color: Kirigami.Theme.disabledTextColor
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
        }

        ListView {
            id: logList
            anchors.fill: parent
            anchors.margins: Kirigami.Units.smallSpacing
            visible: drawer.hasLines
            clip: true
            model: drawer.hasJob ? drawer.job.lines : drawer.checkLines
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
