// The permanent strip between the page and the log.
//
// It answers three questions without opening anything: what is queued, which
// mirror a transaction would use, and how the last one ended. While a job runs
// the strip belongs to the job, because progress is the only thing that matters
// then.
//
// Task 11: the plan summary moved into `PlanDialog`; this strip is a one-line
// status bar again.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Rectangle {
    id: strip

    property var installedBackend
    property var job
    property string jobLabel
    // False once the queue is used again: the last transaction's result must
    // not outlive the state it describes.
    property bool showResult: false
    property string currentMirror

    signal reviewRequested()
    signal cancelRequested()

    readonly property bool running: job !== null && !job.done

    implicitHeight: Kirigami.Units.gridUnit * 2.5
    color: Kirigami.Theme.alternateBackgroundColor

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        height: 1
        color: Kirigami.Theme.disabledTextColor
    }

    // What the queue holds. The finished job's result is a separate label: it
    // must not hide the live queue count.
    function queueText() {
        const count = strip.installedBackend.queueCount
        if (count === 0) {
            return i18nd("kxbps", "Queue is empty")
        }
        return i18ndp("kxbps", "%1 package queued", "%1 packages queued", count)
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        // While a transaction runs, items 1-4 belong to the job.
        RowLayout {
            visible: strip.running
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            QQC2.ProgressBar {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                from: 0
                to: 100
                value: strip.job !== null ? strip.job.progress : 0
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: strip.jobLabel
                elide: Text.ElideRight
            }

            QQC2.Button {
                text: i18nd("kxbps", "Cancel")
                icon.name: "dialog-cancel"
                // A cancel is only possible before xbps exists; hiding the
                // control once it runs avoids a click that would do nothing.
                visible: strip.job === null || strip.job.cancellable
                onClicked: strip.cancelRequested()
            }
        }

        RowLayout {
            visible: !strip.running
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            QQC2.Label {
                text: strip.queueText()
                color: strip.installedBackend.queueCount > 0 ? Kirigami.Theme.textColor
                                                             : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Button {
                text: i18nd("kxbps", "Review changes")
                icon.name: "dialog-ok-apply"
                enabled: strip.installedBackend.queueCount > 0
                onClicked: strip.reviewRequested()
            }

            QQC2.Label {
                visible: strip.job !== null && strip.job.done && strip.showResult
                text: strip.jobLabel
                // A failed transaction must not read like a finished one.
                color: strip.job !== null && strip.job.failed ? Kirigami.Theme.negativeTextColor
                                                              : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Label {
                visible: strip.installedBackend.updatesCount > 0
                text: i18ndp("kxbps", "%1 update", "%1 updates", strip.installedBackend.updatesCount)
                color: Kirigami.Theme.negativeTextColor
            }

            Item {
                Layout.fillWidth: true
            }
        }

        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 18
            text: strip.currentMirror
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideMiddle
        }
    }
}
