// The running transaction, shown as the same log the drawer shows below.
//
// Job retention contract: closing this dialog *hides* it, it does not release
// the job. `Main.qml` keeps the job in `root.job` after Close so the status strip
// and the log drawer stay readable, and ownership moves back here only when the
// next transaction starts (`showJob`), which is the point the job backend promises
// to retain the job until. Releasing on close would empty the drawer the moment
// the user closes the dialog, so there is deliberately no `onClosed` release.
// The next `showJob` releases the previous job when it takes the next one.
//
// The header label is passed in already translated by `Main.qml`, which owns the
// one `jobLabel` switch the strip and the dialog share.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property var installedBackend
    property var job: null
    // The current step, translated by `Main.jobLabel`.
    property string labelText: ""

    readonly property bool running: job !== null && !job.done
    readonly property bool failed: job !== null && job.failed

    title: i18nd("khaxbps", "Transaction")
    preferredWidth: Kirigami.Units.gridUnit * 46
    maximumHeight: (parent ? parent.height : Kirigami.Units.gridUnit * 36) * 0.7
    padding: Kirigami.Units.largeSpacing

    // Takes over a job and releases the one shown before it. Called once per
    // `applyPlan`, so the previous job lives exactly as long as its log is the
    // last transaction's log.
    function showJob(value) {
        if (value === null) {
            return
        }
        if (job !== null && job !== value) {
            const previous = job
            job = null
            installedBackend.release(previous)
        }
        job = value
        open()
        Qt.callLater(logList.positionViewAtEnd)
    }

    function resultText() {
        if (job === null || !job.done) {
            return ""
        }
        if (job.failed) {
            return i18nd("khaxbps", "Transaction failed (exit %1)", job.exitCode)
        }
        if (job.cancelled) {
            return i18nd("khaxbps", "Transaction cancelled")
        }
        return i18nd("khaxbps", "Transaction finished")
    }

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 44
        spacing: Kirigami.Units.smallSpacing

        // Referenced by the conditional footer list below.
        Kirigami.Action {
            id: cancelAction
            text: i18nd("khaxbps", "Cancel")
            onTriggered: if (dialog.job !== null) dialog.job.cancel()
        }

        // Closing hides the dialog but does not release the job — see the
        // retention contract at the top of this file.
        Kirigami.Action {
            id: closeAction
            text: i18nd("khaxbps", "Close")
            onTriggered: dialog.close()
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: dialog.labelText
            elide: Text.ElideRight
        }

        QQC2.ProgressBar {
            Layout.fillWidth: true
            visible: dialog.running
            from: 0
            to: 100
            value: dialog.job !== null ? dialog.job.progress : 0
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Kirigami.Units.gridUnit * 16
            clip: true
            color: Kirigami.Theme.alternateBackgroundColor

            ListView {
                id: logList
                anchors.fill: parent
                anchors.margins: Kirigami.Units.smallSpacing
                clip: true
                model: dialog.job !== null ? dialog.job.lines : null
                // While the job runs the newest line is the interesting one;
                // after it ends the user scrolls back through the log.
                onCountChanged: if (dialog.running) positionViewAtEnd()

                delegate: QQC2.Label {
                    width: logList.width
                    text: model.line
                    font.family: "monospace"
                    elide: Text.ElideRight
                }
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: dialog.job !== null && dialog.job.done
            text: dialog.resultText()
            color: dialog.failed ? Kirigami.Theme.negativeTextColor
                                 : Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }
    }

    // Cancel while it runs, Close once there is nothing left to cancel. The
    // Cancel action does not close the dialog: the worker needs a moment to
    // reach its final state, and the log should show how it ended.
    customFooterActions: dialog.running ? [cancelAction] : [closeAction]
}
