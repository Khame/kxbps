// The confirmation surface for a resolved transaction plan.
//
// The plan is a C++-parented child of `InstalledBackend`: this dialog owns the
// reference while it is open and releases it on close. Applying transfers the
// job to `JobDialog`; a blocked plan transfers ownership to `BlockedDialog`.
// Nothing here starts a transaction without a second click on Apply.
//
// A plan that carries only a notice or an error has no rows and no Apply: the
// footer shrinks to a single Close button instead of offering a dead action.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property var installedBackend
    property var plan: null
    // A dry run is a process; until it answers the dialog says so instead of
    // showing an empty plan.
    property bool loading: false
    property string loadingText: ""

    signal jobStarted(var startedJob)
    signal blockedRequested(var blockedPlan)

    readonly property bool hasRows: plan !== null && !plan.isBlocked
                                    && plan.noticeKey.length === 0
                                    && plan.errorKey.length === 0
                                    && plan.count > 0
    // A plan with steps is what turns Apply on; while a dry run is in flight
    // there is nothing to apply and nothing to say yet.
    readonly property bool waiting: plan === null && loading

    function clearLoading() {
        loading = false
    }

    // Apply exists only when there is something to apply; a blocked plan is
    // handled by the button in the body, a notice or error by Close.
    readonly property bool canApply: hasRows
    readonly property bool showApplyFooter: hasRows || (plan !== null && plan.isBlocked)

    title: dialog.plan === null && dialog.loading ? dialog.loadingText : dialog.titleText()
    preferredWidth: Kirigami.Units.gridUnit * 46
    // The list is the only thing that may scroll; the dialog never grows past
    // 70 % of the window.
    maximumHeight: (parent ? parent.height : Kirigami.Units.gridUnit * 36) * 0.7
    padding: Kirigami.Units.largeSpacing

    onClosed: dialog.releasePlan()

    function showPlan(value) {
        if (value === null) {
            return
        }
        loading = false
        if (value !== plan) {
            dialog.releasePlan()
        }
        plan = value
        open()
    }

    function releasePlan() {
        loading = false
        if (plan === null) {
            return
        }
        const old = plan
        plan = null
        installedBackend.release(old)
    }

    function apply() {
        if (!canApply) {
            return
        }
        const started = installedBackend.applyPlan(plan)
        const finished = plan
        plan = null
        installedBackend.release(finished)
        close()
        if (started !== null) {
            jobStarted(started)
        }
    }

    // Hand the blocked plan to the BlockedDialog without releasing it here:
    // that dialog owns it from now on and releases it on close.
    function requestBlocked() {
        if (plan === null || !plan.isBlocked) {
            return
        }
        const blocked = plan
        plan = null
        close()
        blockedRequested(blocked)
    }

    function actionText(key) {
        switch (key) {
        case "install":
            return i18nd("kxbps", "Install")
        case "remove":
            return i18nd("kxbps", "Remove")
        case "update":
            return i18nd("kxbps", "Update")
        case "reinstall":
            return i18nd("kxbps", "Reinstall")
        }
        // A key the QML layer does not know yet must still name the action.
        return key
    }

    function reasonText(key) {
        switch (key) {
        case "requested":
            return i18nd("kxbps", "Requested")
        case "dependency":
            return i18nd("kxbps", "Dependency")
        case "reverse":
            return i18nd("kxbps", "Reverse dependency")
        case "orphan":
            return i18nd("kxbps", "Orphan")
        }
        return key
    }

    function versionText(item) {
        if (item.from.length === 0 && item.to.length === 0) {
            return "—"
        }
        if (item.from.length === 0) {
            return "— → " + item.to
        }
        if (item.to.length === 0) {
            return item.from + " → —"
        }
        return item.from + " → " + item.to
    }

    function titleText() {
        if (plan === null) {
            return ""
        }
        switch (plan.titleKey) {
        case "planQueue":
            return i18nd("kxbps", "Apply queued changes")
        case "planSystem":
            return i18nd("kxbps", "System upgrade")
        case "planInstall":
            return i18nd("kxbps", "Install packages")
        case "planRemove":
            return i18nd("kxbps", "Remove %1", plan.titleArgs[0] || "")
        case "planCascade":
            return i18nd("kxbps", "Remove %1 and its dependents", plan.titleArgs[0] || "")
        }
        // Notices and errors carry no title key of their own.
        return plan.titleKey.length > 0 ? plan.titleKey
                                        : i18nd("kxbps", "Transaction plan")
    }

    function noticeText() {
        if (plan === null) {
            return ""
        }
        switch (plan.noticeKey) {
        case "emptyQueue":
            return i18nd("kxbps", "The queue does not change the system.")
        case "emptyInstall":
            return i18nd("kxbps", "Nothing to install.")
        case "emptySystem":
            return i18nd("kxbps", "The system is up to date.")
        case "emptyRemove":
            return i18nd("kxbps", "Nothing to remove.")
        }
        return plan.noticeKey
    }

    function errorText() {
        if (plan === null) {
            return ""
        }
        switch (plan.errorKey) {
        case "errorUnknownPackage":
            return i18nd("kxbps", "xbps does not know %1.", plan.errorArgs[0] || "")
        case "errorNotInstalled":
            return i18nd("kxbps", "Not installed: %1", plan.errorArgs[0] || "")
        case "errorXbpsStart":
            return i18nd("kxbps", "%1 could not be started: %2", plan.errorArgs[0] || "",
                         plan.errorArgs[1] || "")
        case "errorXbpsTimeout":
            return i18nd("kxbps", "%1 did not answer within %2 s.", plan.errorArgs[0] || "",
                         plan.errorArgs[1] || "")
        case "errorXbpsExit":
            return i18nd("kxbps", "%1 exited with %2: %3", plan.errorArgs[0] || "",
                         plan.errorArgs[1] || "", plan.errorArgs[2] || "")
        case "errorXbpsOutput":
            return i18nd("kxbps", "%1 printed a line xbps cannot be read from: %3 (line %2)",
                         plan.errorArgs[0] || "", plan.errorArgs[1] || "",
                         plan.errorArgs[2] || "")
        }
        return plan.errorKey
    }

    function noteText() {
        if (plan === null || plan.noteKey.length === 0) {
            return ""
        }
        if (plan.noteKey === "noteHeld") {
            return i18nd("kxbps", "Held packages skipped: %1", plan.noteArgs[0] || "")
        }
        return plan.noteKey
    }

    // One full sentence per sign: translated fragments must not be glued
    // together, because the sign and the wording around it differ per language.
    function diskText() {
        if (plan === null) {
            return ""
        }
        if (plan.diskDelta > 0) {
            return i18nd("kxbps", "Disk change: +%1", plan.diskDeltaText)
        }
        if (plan.diskDelta < 0) {
            return i18nd("kxbps", "Disk change: −%1", plan.diskDeltaText)
        }
        return i18nd("kxbps", "Disk change: 0")
    }

    ColumnLayout {
        id: content

        implicitWidth: Kirigami.Units.gridUnit * 44
        spacing: 0
        focus: true
        Keys.onReturnPressed: if (dialog.canApply) dialog.apply()
        Keys.onEnterPressed: if (dialog.canApply) dialog.apply()

        // The footer actions are referenced here so they exist before the
        // conditional list below is evaluated.
        Kirigami.Action {
            id: cancelAction
            text: i18nd("kxbps", "Cancel")
            onTriggered: dialog.reject()
        }

        Kirigami.Action {
            id: applyAction
            text: i18nd("kxbps", "Apply")
            enabled: dialog.canApply
            onTriggered: dialog.apply()
        }

        Kirigami.Action {
            id: closeAction
            text: i18nd("kxbps", "Close")
            onTriggered: dialog.close()
        }

        // One dry run answers in a moment; a block of text is enough, and the
        // Close action stays available if xbps is slow.
        QQC2.BusyIndicator {
            visible: dialog.waiting
            running: dialog.waiting
            Layout.alignment: Qt.AlignHCenter
        }

        QQC2.Label {
            visible: dialog.waiting
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: dialog.loadingText
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.disabledTextColor
        }

        // Column header, repeated by the row delegate below.
        RowLayout {
            visible: dialog.hasRows
            Layout.fillWidth: true
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "Package")
                font.bold: true
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            QQC2.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                Layout.minimumWidth: Kirigami.Units.gridUnit * 8
                Layout.maximumWidth: Kirigami.Units.gridUnit * 8
                text: i18nd("kxbps", "Action")
                font.bold: true
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                Layout.minimumWidth: Kirigami.Units.gridUnit * 14
                Layout.maximumWidth: Kirigami.Units.gridUnit * 14
                text: i18nd("kxbps", "Version")
                font.bold: true
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                Layout.minimumWidth: Kirigami.Units.gridUnit * 6
                Layout.maximumWidth: Kirigami.Units.gridUnit * 6
                horizontalAlignment: Text.AlignRight
                text: i18nd("kxbps", "Size")
                font.bold: true
                color: Kirigami.Theme.disabledTextColor
            }
        }

        Repeater {
            model: dialog.hasRows ? dialog.plan.items : []

            delegate: RowLayout {
                required property var modelData
                Layout.fillWidth: true
                Layout.preferredHeight: Kirigami.Units.gridUnit * 2.2
                spacing: Kirigami.Units.smallSpacing

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    spacing: 0

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: modelData.name
                        font.bold: true
                        elide: Text.ElideRight
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: dialog.reasonText(modelData.reason)
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.disabledTextColor
                        elide: Text.ElideRight
                    }
                }

                QQC2.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                    Layout.minimumWidth: Kirigami.Units.gridUnit * 8
                    Layout.maximumWidth: Kirigami.Units.gridUnit * 8
                    text: dialog.actionText(modelData.action)
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                    Layout.minimumWidth: Kirigami.Units.gridUnit * 14
                    Layout.maximumWidth: Kirigami.Units.gridUnit * 14
                    text: dialog.versionText(modelData)
                    font.family: "monospace"
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                    Layout.minimumWidth: Kirigami.Units.gridUnit * 6
                    Layout.maximumWidth: Kirigami.Units.gridUnit * 6
                    horizontalAlignment: Text.AlignRight
                    text: modelData.sizeText
                    font.family: "monospace"
                    elide: Text.ElideLeft
                }
            }
        }

        // A blocked plan has no steps: show what stands in the way, and offer
        // the confirmation surface that can turn it into a cascade.
        ColumnLayout {
            visible: dialog.plan !== null && dialog.plan.isBlocked
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            Repeater {
                model: dialog.plan !== null && dialog.plan.isBlocked ? dialog.plan.blocked : []

                delegate: ColumnLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 0

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: modelData.name
                        font.bold: true
                        elide: Text.ElideRight
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        text: i18nd("kxbps", "Needed by: %1", modelData.neededBy.join(", "))
                        wrapMode: Text.WordWrap
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
            }

            QQC2.Button {
                text: i18nd("kxbps", "Review blocked packages")
                onClicked: dialog.requestBlocked()
            }
        }

        QQC2.Label {
            visible: dialog.plan !== null && dialog.plan.noticeKey.length > 0
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: dialog.noticeText()
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            visible: dialog.plan !== null && dialog.plan.errorKey.length > 0
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: dialog.errorText()
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            visible: dialog.plan !== null && dialog.plan.noteKey.length > 0
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: dialog.noteText()
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.neutralTextColor
        }
    }

    footerLeadingComponent: Component {
        ColumnLayout {
            visible: dialog.hasRows
            spacing: 0

            QQC2.Label {
                text: i18ndp("kxbps", "%1 package", "%1 packages",
                             dialog.plan !== null ? dialog.plan.count : 0)
                font: Kirigami.Theme.smallFont
            }

            QQC2.Label {
                text: i18nd("kxbps", "Download: %1",
                            dialog.plan !== null ? dialog.plan.downloadText : "")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                text: dialog.diskText()
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }
        }
    }

    customFooterActions: dialog.showApplyFooter ? [cancelAction, applyAction]
                                                : [closeAction]

    onOpened: content.forceActiveFocus()
}
