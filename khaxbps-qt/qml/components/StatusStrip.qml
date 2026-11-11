// The permanent strip between the page and the log.
//
// It answers three questions without opening anything: what is queued, what the
// queue would cost, and which mirror a transaction would use. While a job runs
// the strip belongs to the job, because progress is the only thing that matters
// then.
//
// Task 11: the plan summary becomes a dialog and this strip goes back to being
// a one-line status bar.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Rectangle {
    id: strip

    property var installedBackend
    property var plan
    property var job
    property string jobLabel
    property string currentMirror

    signal reviewRequested()
    signal cancelRequested()

    readonly property bool running: job !== null && !job.done

    implicitHeight: Kirigami.Units.gridUnit * 2
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
            return i18nd("khaxbps", "Queue is empty")
        }
        return i18ndp("khaxbps", "%1 package queued", "%1 packages queued", count)
    }

    function planText() {
        if (!plan) {
            return ""
        }
        if (plan.errorKey.length > 0) {
            return i18nd("khaxbps", "The plan could not be built.")
        }
        if (plan.isBlocked) {
            return i18nd("khaxbps", "The change is blocked.")
        }
        if (plan.count === 0) {
            return i18nd("khaxbps", "Nothing to do.")
        }
        return i18nd("khaxbps", "%1 to download · %2 disk", plan.downloadText, plan.diskDeltaText)
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.smallSpacing
        anchors.rightMargin: Kirigami.Units.smallSpacing
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
                text: i18nd("khaxbps", "Cancel")
                onClicked: strip.cancelRequested()
            }
        }

        RowLayout {
            visible: !strip.running
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing

            QQC2.Label {
                text: strip.queueText()
                elide: Text.ElideRight
            }

            QQC2.Button {
                text: strip.plan ? i18nd("khaxbps", "Apply") : i18nd("khaxbps", "Review changes")
                enabled: strip.installedBackend.queueCount > 0 || strip.plan !== null
                onClicked: strip.reviewRequested()
            }

            QQC2.Label {
                visible: strip.job !== null && strip.job.done
                text: strip.jobLabel
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Label {
                visible: strip.plan !== null
                Layout.fillWidth: true
                text: strip.planText()
                color: strip.plan !== null && (strip.plan.isBlocked || strip.plan.errorKey.length > 0)
                       ? Kirigami.Theme.negativeTextColor
                       : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Label {
                visible: strip.installedBackend.updatesCount > 0
                text: i18ndp("khaxbps", "%1 update", "%1 updates", strip.installedBackend.updatesCount)
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
