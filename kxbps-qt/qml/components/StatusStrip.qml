// The thin bar at the bottom: what the tool is doing, and the way into the
// xbps log. Counts live in the page footer, where the table that produced them
// is, so this bar answers one question only.
//
// A transaction owns the bar while it runs: progress, its own sentence and a
// cancel that exists only while xbps has not started. Afterwards the bar keeps
// the result until the next transaction, because the exit code is what the
// user wants to read.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Rectangle {
    id: bar

    property var job
    property string jobLabel
    // False once the queue is used again: the last transaction's result must
    // not outlive the state it describes.
    property bool showResult: false
    // Why a transaction never started: a dry run xbps refused, or one that
    // found nothing to do.
    property string message: ""
    property bool messageIsError: false
    // The last update check's sentence. It sits below a running job and below
    // a dry-run message, because the user's own action is the more recent
    // fact; it replaces "Ready" so a quiet strip still says how old the list is.
    property string checkMessage: ""
    property bool checkIsError: false
    // Whether the log drawer below is open.
    property bool logOpen: false

    signal cancelRequested()
    signal logToggleRequested()

    readonly property bool running: job !== null && !job.done
    readonly property bool resultVisible: !running && showResult && job !== null && job.done

    // One sentence, by precedence: the job, its result, the reason nothing
    // started, the last update check, or "Ready".
    readonly property string statusText: {
        if (running || resultVisible) {
            return bar.jobLabel
        }
        if (message.length > 0) {
            return message
        }
        if (checkMessage.length > 0) {
            return checkMessage
        }
        return i18nd("kxbps", "Ready")
    }

    readonly property bool statusIsError: {
        if (running || resultVisible) {
            return job !== null && job.failed === true
        }
        if (message.length > 0) {
            return messageIsError
        }
        return checkMessage.length > 0 && checkIsError
    }

    color: Kirigami.Theme.alternateBackgroundColor

    Kirigami.Separator {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.largeSpacing

        QQC2.ProgressBar {
            visible: bar.running
            Layout.preferredWidth: Kirigami.Units.gridUnit * 8
            from: 0
            to: 100
            value: bar.job !== null ? bar.job.progress : 0
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: bar.statusText
            font: Kirigami.Theme.smallFont
            color: bar.statusIsError ? Kirigami.Theme.negativeTextColor
                                     : Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        QQC2.Button {
            // A cancel is only possible before xbps exists; hiding the control
            // once it runs avoids a click that would do nothing.
            visible: bar.running && (bar.job === null || bar.job.cancellable)
            text: i18nd("kxbps", "Cancel")
            icon.name: "dialog-cancel"
            font: Kirigami.Theme.smallFont
            onClicked: bar.cancelRequested()
        }

        QQC2.Button {
            text: i18nd("kxbps", "Log")
            icon.name: "utilities-terminal"
            font: Kirigami.Theme.smallFont
            checkable: true
            checked: bar.logOpen
            onClicked: bar.logToggleRequested()
        }
    }
}
