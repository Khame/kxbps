// The status bar under the table: the counts, and what a running job is doing.
//
// It answers three questions without opening anything: how big the system is,
// how much of it the queue holds, and how the last transaction ended. While a
// job runs the strip belongs to the job, because progress is the only thing
// that matters then; the counts come back when it has finished.

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
    // Why a transaction never started: a dry run xbps refused, or one that
    // found nothing to do. The strip is the only place the silent queue path
    // can say it.
    property string message: ""
    property bool messageIsError: false
    property string currentMirror
    // The counts of the whole system, not of the visible filter: the strip is a
    // fact about the machine, and a filter is a view of it.
    property int totalCount: 0
    property int installedCount: 0

    signal cancelRequested()

    readonly property bool running: job !== null && !job.done

    color: Kirigami.Theme.alternateBackgroundColor

    Kirigami.Separator {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
    }

    // The one sentence of the strip, with positional arguments so a translator
    // can reorder the numbers. Void ships no catalogue, so there is no total
    // "packages" number to print: the installed set is the only whole set the
    // tool knows.
    function summaryText() {
        // Label-and-number, not a sentence: a count in a sentence needs the
        // language's plural form for each of the three numbers, and one msgid
        // cannot carry three of them. Every language reads this as a legend.
        return i18nd("kxbps", "Installed: %1 · Upgradable: %2 · Marked: %3",
                     strip.installedCount,
                     strip.installedBackend.updatesCount,
                     strip.installedBackend.queueCount)
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        // While a transaction runs the left half belongs to the job.
        RowLayout {
            visible: strip.running
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: Kirigami.Units.largeSpacing

            QQC2.ProgressBar {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                from: 0
                to: 100
                value: strip.job !== null ? strip.job.progress : 0
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: strip.jobLabel
                font: Kirigami.Theme.smallFont
                elide: Text.ElideRight
            }

            QQC2.Button {
                text: i18nd("kxbps", "Cancel")
                icon.name: "dialog-cancel"
                font: Kirigami.Theme.smallFont
                // A cancel is only possible before xbps exists; hiding the
                // control once it runs avoids a click that would do nothing.
                visible: strip.job === null || strip.job.cancellable
                onClicked: strip.cancelRequested()
            }
        }

        QQC2.Label {
            visible: !strip.running
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: strip.summaryText()
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        // Why a click on Apply started nothing. A refusal is not a count, so
        // it reads in the warning colour instead of the muted one.
        QQC2.Label {
            visible: !strip.running && strip.message.length > 0
            Layout.maximumWidth: Kirigami.Units.gridUnit * 30
            text: strip.message
            font: Kirigami.Theme.smallFont
            color: strip.messageIsError ? Kirigami.Theme.negativeTextColor
                                        : Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        // The result of the last transaction, beside the counts it changed.
        QQC2.Label {
            visible: !strip.running && strip.job !== null && strip.job.done && strip.showResult
            Layout.maximumWidth: Kirigami.Units.gridUnit * 24
            text: strip.jobLabel
            font: Kirigami.Theme.smallFont
            // A failed transaction must not read like a finished one.
            color: strip.job !== null && strip.job.failed ? Kirigami.Theme.negativeTextColor
                                                          : Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 20
            text: strip.currentMirror
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideMiddle
            // The repository in use is a URL, so it stays untranslated.
        }
    }
}
