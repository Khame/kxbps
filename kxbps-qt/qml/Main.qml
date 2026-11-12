import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import org.kde.kxbps
import "components"
import "views"

Kirigami.ApplicationWindow {
    id: root

    width: Kirigami.Units.gridUnit * 64
    height: Kirigami.Units.gridUnit * 36
    minimumWidth: Kirigami.Units.gridUnit * 48
    minimumHeight: Kirigami.Units.gridUnit * 26
    // QQmlApplicationEngine does not show the root window on its own.
    visible: true
    title: i18nd("kxbps", "KXbps")

    // The window draws its own header; Kirigami's application toolbar would put
    // the page title above it a second time.
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None

    // Which sidebar entry is active. The four package sections share one view
    // and differ only by the model's filter; only Query and Mirrors are pages of
    // their own.
    property string section: "packages"

    // The current or last finished job. It is kept until the next transaction
    // starts, so the status strip and the log drawer stay readable after the
    // JobDialog closes; the dialog releases the previous job when it takes the
    // next one.
    property var job: null
    property bool showJobResult: false

    readonly property string jobLabel: {
        if (job === null) {
            return ""
        }
        switch (job.labelKey) {
        case "jobSync":
            return i18nd("kxbps", "Synchronising the repository index…")
        case "jobDownload":
            return job.labelArg.length > 0 ? i18nd("kxbps", "Downloading %1…", job.labelArg)
                                           : i18nd("kxbps", "Downloading packages…")
        case "jobUnpack":
            return job.labelArg.length > 0 ? i18nd("kxbps", "Unpacking %1…", job.labelArg)
                                           : i18nd("kxbps", "Unpacking packages…")
        case "jobRemove":
            return i18nd("kxbps", "Removing %1…", job.labelArg)
        case "jobDone":
            return i18nd("kxbps", "Finished.")
        case "jobCancelled":
            return i18nd("kxbps", "Cancelled.")
        case "jobFailed":
            return i18nd("kxbps", "Transaction failed")
        }
        return ""
    }

    // Query helpers over the catalog (`search`, `get`).
    CatalogBackend {
        id: catalogBackend
    }

    // Owns the store: the package model, the queue, planning and applying.
    InstalledBackend {
        id: installedBackend
        // The query façade answers `search`/`get` from the same snapshot; without
        // this wiring its results would report every package as not installed.
        queryBackend: catalogBackend
    }

    // The version string the About dialog shows.
    AppBackend {
        id: appBackend
    }

    // The mirror list, its probe batch and the persisted selection. `owner` is
    // the store owner, so the chosen mirror is saved through it.
    MirrorBackend {
        id: mirrorBackend
        owner: installedBackend
    }

    function selectFilter(token) {
        section = "packages"
        installedBackend.packages.filter = token
    }

    // The header follows the filter, not the sidebar entry: clicking "Orphans"
    // must not keep saying "Installed".
    function headerTitle() {
        if (section === "query") {
            return i18nd("kxbps", "Query")
        }
        if (section === "mirrors") {
            return i18nd("kxbps", "Mirrors")
        }
        switch (installedBackend.packages.filter) {
        case "installed":
            return i18nd("kxbps", "Installed")
        case "updates":
            return i18nd("kxbps", "Updates")
        case "all":
            return i18nd("kxbps", "Available")
        case "queued":
            return i18nd("kxbps", "Queue")
        case "explicit":
            return i18nd("kxbps", "Explicit")
        case "orphans":
            return i18nd("kxbps", "Orphans")
        case "held":
            return i18nd("kxbps", "Held")
        }
        return i18nd("kxbps", "Packages")
    }

    function headerSubtitle() {
        if (section === "query") {
            return i18ndp("kxbps", "%1 result", "%1 results", queryView.resultCount)
        }
        if (section === "mirrors") {
            if (mirrorBackend.probing) {
                return i18nd("kxbps", "%1 of %2 mirrors probed",
                             mirrorBackend.finishedCount, mirrorBackend.model.count)
            }
            return i18ndp("kxbps", "%1 mirror", "%1 mirrors", mirrorBackend.model.count)
        }
        const filter = installedBackend.packages.filter
        const count = installedBackend.packages.count
        if (filter === "installed" || filter === "updates" || filter === "all") {
            // The list is the installed set xbps reported; a repository count
            // would need a second read this header does not make.
            return i18ndp("kxbps", "%1 package", "%1 packages", count)
        }
        return i18ndp("kxbps", "%1 package", "%1 packages", count)
    }

    // Plan and job objects are C++-parented children of the backend, so the
    // dialog that opens one releases it. `startJob` hands the job to JobDialog,
    // which releases the previous job when it takes the new one; the strip and
    // the drawer keep reading `root.job` in between.
    function openQueuePlan() {
        planDialog.showPlan(installedBackend.openQueuePlan())
    }

    function openSystemPlan() {
        planDialog.showPlan(installedBackend.openSystemPlan())
    }

    function startJob(started) {
        if (started === null) {
            return
        }
        // Assign before the dialog releases the previous job, so no binding
        // ever reads a job that is already scheduled for deletion.
        job = started
        showJobResult = false
        jobDialog.showJob(started)
        logDrawer.expand()
    }

    Connections {
        target: installedBackend
        function onQueueCountChanged() {
            // A queue change invalidates the plan a dialog is showing; the
            // dialog drops the stale snapshot and releases the backend child.
            if (planDialog.opened) {
                planDialog.close()
            }
            if (installedBackend.queueCount > 0) {
                root.showJobResult = false
            }
        }
        // One revision per store mutation, including the hold toggle the
        // counters cannot see. The query façade receives the same snapshot, so
        // nothing else has to be refreshed here.
        function onStateRevisionChanged() {
        }
    }

    Connections {
        target: root.job
        function onDoneChanged() {
            if (root.job !== null && root.job.done) {
                root.showJobResult = true
            }
        }
    }

    // The confirm surface: queue plan, system plan, named install and the
    // cascade plan BlockedDialog produces all arrive here.
    PlanDialog {
        id: planDialog
        installedBackend: installedBackend
        onJobStarted: (started) => root.startJob(started)
        onBlockedRequested: (blockedPlan) => blockedDialog.showPlan(blockedPlan)
    }

    JobDialog {
        id: jobDialog
        installedBackend: installedBackend
        labelText: root.jobLabel
    }

    BlockedDialog {
        id: blockedDialog
        installedBackend: installedBackend
        onCascadeRequested: (cascadePlan) => planDialog.showPlan(cascadePlan)
    }

    ResetDialog {
        id: resetDialog
        installedBackend: installedBackend
        // Restore rewrites the whole store, so the last transaction's result no
        // longer describes it. Only an actual restore emits this signal; Cancel
        // leaves the result alone.
        onRestored: root.showJobResult = false
    }

    AboutDialog {
        id: aboutDialog
        version: appBackend.version()
    }

    Component.onCompleted: {
        // The window opens on the installed set, read from the system: the
        // worker thread fills the list a moment later, and until it answers the
        // catalog fills the first frame rather than an empty window.
        installedBackend.packages.filter = "installed"
        installedBackend.reload()
    }

    pageStack.initialPage: Kirigami.Page {
        padding: 0

        RowLayout {
            anchors.fill: parent
            spacing: 0

            // Navigation. The mockup's sidebar, with the counters the tool can
            // actually keep: installed, updates, the full catalog and the queue.
            Rectangle {
                Layout.preferredWidth: Kirigami.Units.gridUnit * 12.5
                Layout.fillHeight: true
                color: Kirigami.Theme.alternateBackgroundColor

                Rectangle {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: 1
                    color: Kirigami.Theme.disabledTextColor
                }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.topMargin: Kirigami.Units.smallSpacing
                    spacing: 0

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Installed")
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "installed"
                        onClicked: root.selectFilter("installed")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Updates")
                        badgeText: installedBackend.updatesCount > 0 ? String(installedBackend.updatesCount) : ""
                        badgeAlert: true
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "updates"
                        onClicked: root.selectFilter("updates")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Available")
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "all"
                        onClicked: root.selectFilter("all")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Queue")
                        badgeText: installedBackend.queueCount > 0 ? String(installedBackend.queueCount) : ""
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "queued"
                        onClicked: root.selectFilter("queued")
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        Layout.rightMargin: Kirigami.Units.smallSpacing
                        height: 1
                        color: Kirigami.Theme.disabledTextColor
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        text: i18nd("kxbps", "Manage")
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.disabledTextColor
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Query")
                        highlighted: root.section === "query"
                        onClicked: root.section = "query"
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Mirrors")
                        highlighted: root.section === "mirrors"
                        onClicked: root.section = "mirrors"
                    }

                    Item {
                        Layout.fillHeight: true
                    }

                    // The architecture is not exposed by any backend yet;
                    // claiming `x86_64` here would be a guess, so the foot only
                    // names the system it manages.
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.margins: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Rectangle {
                            width: Kirigami.Units.gridUnit * 0.5
                            height: width
                            radius: width / 2
                            color: installedBackend.error.length > 0 ? Kirigami.Theme.negativeTextColor
                                                                     : Kirigami.Theme.positiveTextColor
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Void Linux")
                            font: Kirigami.Theme.smallFont
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // One title, one muted line with the live numbers, and the
                // current view's actions on the right.
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: toolbarLayout.implicitHeight + Kirigami.Units.largeSpacing * 2
                    color: Kirigami.Theme.backgroundColor

                    RowLayout {
                        id: toolbarLayout
                        anchors.fill: parent
                        anchors.margins: Kirigami.Units.largeSpacing
                        spacing: Kirigami.Units.largeSpacing

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0

                            Kirigami.Heading {
                                Layout.fillWidth: true
                                level: 3
                                text: root.headerTitle()
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.fillWidth: true
                                text: root.headerSubtitle()
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                            }
                        }

                        QQC2.Button {
                            visible: root.section === "mirrors"
                            text: i18nd("kxbps", "Probe all")
                            icon.name: "network-connect"
                            enabled: !mirrorBackend.probing
                            onClicked: mirrorBackend.probeAll()
                        }

                        // Task 13 replaces this with the menu bar; until then
                        // the two global actions live behind one toolbar menu.
                        QQC2.ToolButton {
                            id: menuButton
                            icon.name: "application-menu"
                            Accessible.name: i18nd("kxbps", "Application menu")
                            onClicked: menu.open()

                            QQC2.Menu {
                                id: menu
                                y: menuButton.height
                                width: Kirigami.Units.gridUnit * 14

                                QQC2.MenuItem {
                                    text: i18nd("kxbps", "About KXbps")
                                    onTriggered: aboutDialog.open()
                                }

                                QQC2.MenuItem {
                                    text: i18nd("kxbps", "Restore base image")
                                    onTriggered: resetDialog.open()
                                }
                            }
                        }
                    }

                    Rectangle {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        height: 1
                        color: Kirigami.Theme.disabledTextColor
                    }
                }

                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: root.section === "packages" ? 0 : (root.section === "query" ? 1 : 2)

                    PackagesView {
                        id: packagesView
                        packages: installedBackend.packages
                        installedBackend: installedBackend
                        catalogBackend: catalogBackend
                        job: root.job
                        onUpdateAllRequested: root.openSystemPlan()
                    }

                    QueryView {
                        id: queryView
                        catalogBackend: catalogBackend
                    }

                    MirrorsView {
                        mirrorBackend: mirrorBackend
                        installedBackend: installedBackend
                    }
                }

                StatusStrip {
                    Layout.fillWidth: true
                    installedBackend: installedBackend
                    job: root.job
                    jobLabel: root.jobLabel
                    showResult: root.showJobResult
                    currentMirror: mirrorBackend.currentMirror
                    onReviewRequested: root.openQueuePlan()
                    onCancelRequested: {
                        if (root.job !== null) {
                            root.job.cancel()
                        }
                    }
                }

                LogDrawer {
                    id: logDrawer
                    Layout.fillWidth: true
                    job: root.job
                }
            }
        }
    }
}
