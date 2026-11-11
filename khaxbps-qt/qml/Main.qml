import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import org.kde.khaxbps
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
    title: i18nd("khaxbps", "Khaxbps")

    // The window draws its own header; Kirigami's application toolbar would put
    // the page title above it a second time.
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None

    // Which sidebar entry is active. The four package sections share one view
    // and differ only by the model's filter; only Query and Mirrors are pages of
    // their own.
    property string section: "packages"

    // The whole catalog, read through the query façade because QML cannot index
    // the package model by row. The details pane uses it for reverse
    // dependencies and the query view narrows the argument-less modes with it.
    // Refreshed after every store change: a map carries the installed and queue
    // snapshot it was built from.
    property var catalogIndex: []

    // Task 11: replace with the plan/job dialogs.
    property var plan: null
    // The finished job is kept until the next transaction starts, so its log
    // stays readable after it ends. Its result line leaves the strip as soon as
    // the queue is used again.
    property var job: null
    property bool showJobResult: false

    readonly property string jobLabel: {
        if (job === null) {
            return ""
        }
        switch (job.labelKey) {
        case "jobSync":
            return i18nd("khaxbps", "Synchronising the repository index…")
        case "jobDownload":
            return job.labelArg.length > 0 ? i18nd("khaxbps", "Downloading %1…", job.labelArg)
                                           : i18nd("khaxbps", "Downloading packages…")
        case "jobUnpack":
            return job.labelArg.length > 0 ? i18nd("khaxbps", "Unpacking %1…", job.labelArg)
                                           : i18nd("khaxbps", "Unpacking packages…")
        case "jobRemove":
            return i18nd("khaxbps", "Removing %1…", job.labelArg)
        case "jobDone":
            return i18nd("khaxbps", "Finished.")
        case "jobCancelled":
            return i18nd("khaxbps", "Cancelled.")
        case "jobFailed":
            return i18nd("khaxbps", "Transaction failed")
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

    // The mirror list, its probe batch and the persisted selection. `owner` is
    // the store owner, so the chosen mirror is saved through it.
    MirrorBackend {
        id: mirrorBackend
        owner: installedBackend
    }

    function refreshCatalogIndex() {
        catalogIndex = catalogBackend.search("", "L", "")
    }

    function selectFilter(token) {
        section = "packages"
        installedBackend.packages.filter = token
    }

    // The header follows the filter, not the sidebar entry: clicking "Orphans"
    // must not keep saying "Installed".
    function headerTitle() {
        if (section === "query") {
            return i18nd("khaxbps", "Query")
        }
        if (section === "mirrors") {
            return i18nd("khaxbps", "Mirrors")
        }
        switch (installedBackend.packages.filter) {
        case "installed":
            return i18nd("khaxbps", "Installed")
        case "updates":
            return i18nd("khaxbps", "Updates")
        case "all":
            return i18nd("khaxbps", "Available")
        case "queued":
            return i18nd("khaxbps", "Queue")
        case "explicit":
            return i18nd("khaxbps", "Explicit")
        case "orphans":
            return i18nd("khaxbps", "Orphans")
        case "held":
            return i18nd("khaxbps", "Held")
        }
        return i18nd("khaxbps", "Packages")
    }

    function headerSubtitle() {
        if (section === "query") {
            return i18ndp("khaxbps", "%1 result", "%1 results", queryView.resultCount)
        }
        if (section === "mirrors") {
            if (mirrorBackend.probing) {
                return i18nd("khaxbps", "%1 of %2 mirrors probed",
                             mirrorBackend.finishedCount, mirrorBackend.model.count)
            }
            return i18ndp("khaxbps", "%1 mirror", "%1 mirrors", mirrorBackend.model.count)
        }
        const filter = installedBackend.packages.filter
        const count = installedBackend.packages.count
        if (filter === "installed" || filter === "updates" || filter === "all") {
            return i18ndp("khaxbps", "%1 package · %2 in repository",
                          "%1 packages · %2 in repository", count, catalogIndex.length)
        }
        return i18ndp("khaxbps", "%1 package", "%1 packages", count)
    }

    // Task 11: replace with the plan/job dialogs.
    //
    // First press opens the queue plan and the strip shows what it would cost;
    // the second press applies it. Plan and job objects are C++-parented
    // children of the backend and must be released, never garbage-collected.
    function reviewOrApply() {
        if (plan !== null) {
            const opened = plan
            const started = installedBackend.applyPlan(opened)
            plan = null
            installedBackend.release(opened)
            if (job !== null) {
                const previous = job
                job = null
                installedBackend.release(previous)
            }
            showJobResult = false
            job = started
            if (job !== null) {
                logDrawer.expand()
            }
            return
        }
        plan = installedBackend.openQueuePlan()
    }

    function closePlan() {
        if (plan === null) {
            return
        }
        const opened = plan
        plan = null
        installedBackend.release(opened)
    }

    Connections {
        target: installedBackend
        function onQueueCountChanged() {
            // A queue change invalidates the plan the strip was showing.
            root.closePlan()
            if (installedBackend.queueCount > 0) {
                root.showJobResult = false
            }
        }
        // One revision per store mutation, including the hold toggle the
        // counters cannot see. The catalog snapshot carries installed and held
        // flags, so it has to be re-read.
        function onStateRevisionChanged() {
            root.refreshCatalogIndex()
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

    Component.onCompleted: {
        root.refreshCatalogIndex()
        // The window opens on the installed set, not on all 115 packages.
        installedBackend.packages.filter = "installed"
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
                        label: i18nd("khaxbps", "Installed")
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "installed"
                        onClicked: root.selectFilter("installed")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("khaxbps", "Updates")
                        badgeText: installedBackend.updatesCount > 0 ? String(installedBackend.updatesCount) : ""
                        badgeAlert: true
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "updates"
                        onClicked: root.selectFilter("updates")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("khaxbps", "Available")
                        highlighted: root.section === "packages"
                                     && installedBackend.packages.filter === "all"
                        onClicked: root.selectFilter("all")
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("khaxbps", "Queue")
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
                        text: i18nd("khaxbps", "Manage")
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.disabledTextColor
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("khaxbps", "Query")
                        highlighted: root.section === "query"
                        onClicked: root.section = "query"
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("khaxbps", "Mirrors")
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
                            text: i18nd("khaxbps", "Void Linux")
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
                            text: i18nd("khaxbps", "Probe all")
                            icon.name: "network-connect"
                            enabled: !mirrorBackend.probing
                            onClicked: mirrorBackend.probeAll()
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
                        catalogIndex: root.catalogIndex
                    }

                    QueryView {
                        id: queryView
                        catalogBackend: catalogBackend
                        catalogIndex: root.catalogIndex
                    }

                    MirrorsView {
                        mirrorBackend: mirrorBackend
                        catalogBackend: catalogBackend
                        installedBackend: installedBackend
                    }
                }

                StatusStrip {
                    Layout.fillWidth: true
                    installedBackend: installedBackend
                    plan: root.plan
                    job: root.job
                    jobLabel: root.jobLabel
                    showResult: root.showJobResult
                    currentMirror: mirrorBackend.currentMirror
                    onReviewRequested: root.reviewOrApply()
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
