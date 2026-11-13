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

    // The selected row, as the whole tuple the menu bar acts on. It reaches the
    // window through `PackagesView.selectionChanged`; an empty `selectedName`
    // means nothing is selected and every Package item is disabled.
    property string selectedName: ""
    property string selectedPkgver: ""
    property bool selectedInstalled: false
    property bool selectedIsUpdate: false
    property bool selectedHeld: false
    property string selectedQueuedAction: ""

    // True once a row is selected: the one condition every Package item needs.
    readonly property bool hasSelection: selectedName.length > 0

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
        case "jobStart":
            return i18nd("kxbps", "Waiting for authentication…")
        case "jobWorking":
            return i18nd("kxbps", "Running xbps…")
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
        case "jobAuthDismissed":
            return i18nd("kxbps", "Authentication was dismissed")
        case "jobAuthDenied":
            return i18nd("kxbps", "Authentication was denied")
        case "jobPkexecMissing":
            return i18nd("kxbps", "pkexec is missing or not executable")
        case "jobXbpsFailed":
            return job.labelArg.length > 0 ? i18nd("kxbps", "xbps failed with exit code %1", job.labelArg)
                                           : i18nd("kxbps", "xbps failed")
        case "jobBusy":
            return i18nd("kxbps", "Another transaction is running")
        case "jobFailed":
            return i18nd("kxbps", "Transaction failed")
        // The helper's success verdict carries no argument: the URL is the one
        // the dialog named a moment ago, so repeating it adds nothing.
        case "mirrorApplyDone":
            return i18nd("kxbps", "Mirror applied and the index synchronised.")
        // The helper's failures. `labelArg` carries only the first verdict
        // argument, so a `%2` the key would like (the `-S` stderr tail) is not
        // available to QML; the argument that is there is the one that names
        // what went wrong.
        case "errorMirrorUrlRefused":
            return i18nd("kxbps", "The helper refused the mirror URL.")
        case "errorMirrorWrite":
            return i18nd("kxbps", "Could not write the xbps configuration: %1", job.labelArg)
        case "errorMirrorSync":
            return i18nd("kxbps", "The repository index could not be synchronised (exit %1)", job.labelArg)
        case "errorMirrorRevert":
            return i18nd("kxbps", "Synchronisation failed and the previous configuration could not be restored.")
        case "errorMirrorHelper":
            return job.labelArg.length > 0
                   ? i18nd("kxbps", "The mirror helper failed: %1", job.labelArg)
                   : i18nd("kxbps", "The mirror helper failed")
        }
        return ""
    }

    // Query helpers over xbps (`search`, `get`).
    QueryBackend {
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

    // The user's preferences. `owner` comes first and nothing else is set in
    // this literal: a write before the owner is attached would be overwritten
    // by the block the store hands over on attach (module docs, attach order).
    PrefsBackend {
        id: prefsBackend
        owner: installedBackend
        // Once, after the owner's block is adopted: the file on disk is made to
        // match the saved switch, so a fresh profile gets its entry and a user
        // who turned it off does not. Toggles below keep the two in step.
        Component.onCompleted: syncAutostart()
    }

    Connections {
        target: prefsBackend
        function onStartWithSessionChanged() {
            prefsBackend.syncAutostart()
        }
    }

    // The tray icon, if the session has a tray host. A missing
    // `Qt.labs.platform` leaves `item` null and every use below guards for it.
    Loader {
        id: trayLoader
        // Qt Widgets backs the tray; on a platform without one (offscreen, a
        // plain WM) creating the icon only prints Qt warnings.
        active: appBackend.trayAvailable
        source: "components/TrayIcon.qml"
        onLoaded: item.installedBackend = installedBackend
    }

    // The desktop notification, if `org.kde.notification` is installed. The
    // notifier itself never fails to load; its inner popup does, silently.
    Loader {
        id: notifierLoader
        source: "components/Notifier.qml"
        asynchronous: true
        onLoaded: {
            item.installedBackend = installedBackend
            item.prefsBackend = prefsBackend
            root.updateStartNotified()
        }
    }

    // `--tray` starts with the window hidden: that is what the autostart entry
    // runs, and the argv is read here so the Rust side needs no argument parser.
    Component.onCompleted: {
        if (Qt.application.arguments.indexOf("--tray") >= 0) {
            visible = false
        }
        // The window opens on the installed set, read from the system: the
        // worker thread fills the list a moment later, and until it answers the
        // catalog fills the first frame rather than an empty window.
        installedBackend.packages.filter = "installed"
        installedBackend.reload()
    }

    // Whether closing the window should hand it to the tray. Bound, not copied:
    // the tray Loader fills (or fails) after this object is complete.
    readonly property bool trayAvailable: trayLoader.item !== null

    // The window's `visible` has to describe the real state, or a close that the
    // tray intercepted still counts as a close. `quitting` is set by the tray's
    // Quit, so the closing handler knows which is which.
    property bool quitting: false

    // One entry point for the tray icon and the notification: bring the window
    // back, raise it, switch to what the user was told about. The filter
    // switches before activation so focus lands on the list.
    function showUpdates() {
        visible = true
        selectFilter("updates")
        raise()
        requestActivate()
    }

    // The announcement was delivered, seen or ignored: the signature goes into
    // the marker so the next start stays quiet. The signature is the one
    // `apply_plan` writes when nothing is left, so both describe one set.
    function markDelivered(signature) {
        if (signature.length > 0) {
            prefsBackend.dismissedVersion = signature
        }
    }

    onClosing: function(close) {
        if (!root.quitting && root.trayAvailable) {
            close.accepted = false
            root.visible = false
        }
    }

    function selectFilter(token) {
        section = "packages"
        // The filter narrows the installed set; a repository search belongs to
        // the search field, so switching filters drops both.
        packagesView.clearSearch()
        installedBackend.packages.filter = token
    }

    // A re-check while the app sits in the tray: a long-running instance must
    // still notice new updates. Six hours is the interval the plan asked for,
    // and only a changed signature notifies, so a quiet timer stays quiet.
    Timer {
        interval: 6 * 60 * 60 * 1000
        running: !root.visible
        repeat: true
        onTriggered: installedBackend.reload()
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
        // A non-empty pattern asks the repositories, so the number is the size
        // of the answer, not of the installed set.
        if (installedBackend.packages.search.length > 0) {
            return i18ndp("kxbps", "%1 search result", "%1 search results", count)
        }
        switch (filter) {
        case "installed":
            return i18ndp("kxbps", "%1 installed package", "%1 installed packages", count)
        case "updates":
            return i18ndp("kxbps", "%1 update", "%1 updates", count)
        case "queued":
            return i18ndp("kxbps", "%1 queued package", "%1 queued packages", count)
        }
        return i18ndp("kxbps", "%1 package", "%1 packages", count)
    }

    // Plan and job objects are C++-parented children of the backend, so the
    // dialog that opens one releases it. `startJob` hands the job to JobDialog,
    // which releases the previous job when it takes the new one; the strip and
    // the drawer keep reading `root.job` in between.
    //
    // A plan is a dry run, so `open*` only starts a worker thread and answers
    // with a request id; the object arrives through `onPlanReady`. The backend
    // drops an answer whose request a newer one superseded, so a plan is only
    // ever shown for the state the user asked from.
    function openQueuePlan() {
        planDialog.loadingText = i18nd("kxbps", "Asking xbps what it would do…")
        planDialog.loading = true
        planDialog.open()
        planPending = installedBackend.openQueuePlan()
    }

    function openSystemPlan() {
        planDialog.loadingText = i18nd("kxbps", "Asking xbps what it would do…")
        planDialog.loading = true
        planDialog.open()
        planPending = installedBackend.openSystemPlan()
    }

    // One row asked for its own transaction: `install` and `update` both plan
    // installing that name (xbps resolves the version itself), `remove` plans
    // the removal. Either way the dialog opens in its loading state and the dry
    // run decides what is really going to happen.
    function openPackagePlan(name, token) {
        if (name.length === 0) {
            return
        }
        planDialog.loadingText = i18nd("kxbps", "Asking xbps what it would do…")
        planDialog.loading = true
        planDialog.open()
        if (token === "remove") {
            planPending = installedBackend.openRemoveNames([name], false)
        } else {
            planPending = installedBackend.openNamedPlan([name], "planInstall")
        }
    }

    // `Remove…` from the menu bar. The backend answers with a request id only,
    // like the queue and system plans, so the dialog opens in its loading state
    // and `onPlanReady` fills it. A name that is not installed, or an empty one,
    // sends nothing: the menu item is disabled then, and this is the second net.
    function openRemoveName(name) {
        if (name.length === 0) {
            return
        }
        planDialog.loadingText = i18nd("kxbps", "Asking xbps what it would do…")
        planDialog.loading = true
        planDialog.open()
        const names = [name]
        planPending = installedBackend.openRemoveNames(names, false)
    }

    // One queue gesture for the menu bar, the same rule `PackageDelegate` uses:
    // take a queued row out, otherwise queue the token the row would send.
    function toggleSelectedQueue() {
        if (!hasSelection) {
            return
        }
        if (selectedQueuedAction !== "") {
            installedBackend.removeQueued(selectedName)
            return
        }
        const token = selectedIsUpdate ? "update" : (selectedInstalled ? "remove" : "install")
        installedBackend.toggleQueue(selectedName, token)
    }

    // "Show installed in xbps-query -L" prints the repository list; the pattern
    // is left alone because `-L` reads every configured repository.
    function showInstalledInQuery() {
        section = "query"
        queryView.selectMode("L")
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
            // The pending request goes too: its answer describes the queue the
            // user just changed and must not reopen the dialog.
            root.planPending = 0
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

    // The notification tries exactly once per start, when the first read has
    // answered. `stateRevision` is not the hook: a hold toggle moves it too,
    // and the popup is a start-time event.
    //
    // A Loader's `onLoaded` alone is not enough either: a notifier that
    // finishes loading after the read answered never sees another
    // `loadingChanged`. Both places call `updateStartNotified`, which runs the
    // check at most once.
    property bool startNotified: false

    function updateStartNotified() {
        if (root.startNotified || installedBackend.loading || notifierLoader.item === null) {
            return
        }
        root.startNotified = true
        notifierLoader.item.consider()
    }

    Connections {
        target: installedBackend
        function onLoadingChanged() {
            // The answer arrives before `loading` turns false, so the counters
            // and the signature are already current here.
            if (!installedBackend.loading) {
                root.updateStartNotified()
            }
        }
        // A reload while the window sits in the tray: only a set the user has
        // not dismissed yet produces a popup, because the signature moved.
        function onPendingSignatureChanged() {
            root.updateStartNotified()
            if (!root.startNotified || notifierLoader.item === null) {
                return
            }
            notifierLoader.item.consider()
        }
    }

    Connections {
        target: notifierLoader.item
        function onOpenUpdatesRequested() {
            root.showUpdates()
        }
        function onDelivered(signature) {
            root.markDelivered(signature)
        }
    }

    Connections {
        target: trayLoader.item
        function onShowRequested() {
            root.showUpdates()
        }
        function onQuitRequested() {
            // The flag first: `onClosing` would otherwise hide the window and
            // leave the process running with no tray to bring it back.
            root.quitting = true
            Qt.quit()
        }
    }

    // The request id waiting for its plan; 0 means nothing is pending.
    property real planPending: 0

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

    // Closing the dialog abandons the request it was waiting for; without this
    // a dry run that answers after Cancel would find its id still pending and
    // reopen the dialog. The backend's generation guard stays as the second
    // net for a plan superseded by a newer one.
    Connections {
        target: planDialog
        function onClosed() {
            root.planPending = 0
        }
    }

    // The selection reaches the window as one tuple, so no menu item can pair a
    // name with a stale flag.
    Connections {
        function onSelectionChanged(name, pkgver, installed, isUpdate, held, queuedAction) {
            root.selectedName = name
            root.selectedPkgver = pkgver
            root.selectedInstalled = installed
            root.selectedIsUpdate = isUpdate
            root.selectedHeld = held
            root.selectedQueuedAction = queuedAction
        }
        target: packagesView
    }

    // The dry run answered: build the plan object the backend described and
    // hand it to the dialog, which owns and releases it from then on.
    Connections {
        target: installedBackend
        function onPlanReady(payloadJson, request) {
            if (request !== root.planPending) {
                return
            }
            planDialog.clearLoading()
            root.planPending = 0
            const plan = installedBackend.makePlan(payloadJson)
            if (plan !== null) {
                planDialog.showPlan(plan)
            }
        }
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

    // Applying the selected mirror rewrites the file xbps reads.
    //
    // The confirmation names the URL and the backup the helper leaves behind,
    // because this is the one action that changes the system from this page.
    // Running the job needs root, so the dialog does not do it for the user: it
    // hands the job to `startJob`, which shows it and the log.
    Kirigami.Dialog {
        id: mirrorApplyDialog

        // The path the helper backs `00-repository-main.conf` up to before it
        // writes. Named here so the user knows what to restore by hand.
        readonly property string backupPath: "/etc/xbps.d/00-repository-main.conf.kxbps-bak"

        title: i18nd("kxbps", "Use this mirror on the system?")
        preferredWidth: Kirigami.Units.gridUnit * 40
        padding: Kirigami.Units.largeSpacing

        function apply() {
            const started = mirrorBackend.applySystemMirror()
            close()
            // Null means nothing was applied: the slot already refused it. The
            // backend clears `applyBusy` and re-reads the system half, so the
            // footer follows without a second read here.
            root.startJob(started)
        }

        ColumnLayout {
            implicitWidth: Kirigami.Units.gridUnit * 38
            spacing: Kirigami.Units.smallSpacing

            Kirigami.Action {
                id: mirrorCancelAction
                text: i18nd("kxbps", "Cancel")
                onTriggered: mirrorApplyDialog.reject()
            }

            Kirigami.Action {
                id: mirrorUseAction
                text: i18nd("kxbps", "Use on the system")
                onTriggered: mirrorApplyDialog.apply()
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "xbps starts using %1, and the repository index is synchronised.",
                            mirrorBackend.currentMirror)
                font.family: "monospace"
                wrapMode: Text.WordWrap
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "The helper writes %1 and keeps the previous file as %2.",
                            "/etc/xbps.d/00-repository-main.conf", mirrorApplyDialog.backupPath)
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
            }

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "This changes the system for every user. It needs administrator authentication.")
                color: Kirigami.Theme.negativeTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
            }
        }

        customFooterActions: [mirrorCancelAction, mirrorUseAction]
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
                        // Void has no catalogue to list here: the entry opens the
                        // package view and puts the cursor in the search field,
                        // because that is how a package that is not installed is
                        // found.
                        onClicked: {
                            root.selectFilter("all")
                            packagesView.focusSearch()
                        }
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

                // The menu bar. `Kirigami.ApplicationWindow` has no `menuBar`
                // property, so this is an in-window bar in the right column, not
                // an entry in the Plasma global menu. It sits above the header
                // and sizes itself from the style (no pixel height, or scaling
                // breaks).
                AppMenuBar {
                    id: appMenuBar
                    Layout.fillWidth: true

                    packageName: root.selectedName
                    packagePkgver: root.selectedPkgver
                    packageInstalled: root.selectedInstalled
                    packageIsUpdate: root.selectedIsUpdate
                    packageHeld: root.selectedHeld
                    packageQueuedAction: root.selectedQueuedAction

                    prefsBackend: prefsBackend
                    storeReady: installedBackend !== null

                    updatesCount: installedBackend.updatesCount
                    section: root.section
                    filter: installedBackend.packages.filter
                    onResetRequested: resetDialog.open()
                    onAboutRequested: aboutDialog.open()
                    onQueueToggleRequested: root.toggleSelectedQueue()
                    onHoldToggleRequested: installedBackend.toggleHold(root.selectedName)
                    onRemoveRequested: root.openRemoveName(root.selectedName)
                    onUpdateAllRequested: root.openSystemPlan()
                    onFilterSelected: (token) => root.selectFilter(token)
                    onSectionSelected: (token) => root.section = token
                    onShowInstalledInQueryRequested: root.showInstalledInQuery()
                }

                // One title, one muted line with the live numbers, and the
                // current view's actions on the right. No app icon: the window
                // title already carries the name, and a 32 px logo next to a
                // 16 px heading only pushes the text down.
                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: toolbarLayout.implicitHeight
                                    + Kirigami.Units.smallSpacing * 2
                    color: Kirigami.Theme.backgroundColor

                    RowLayout {
                        id: toolbarLayout
                        anchors.fill: parent
                        anchors.leftMargin: Kirigami.Units.largeSpacing
                        anchors.rightMargin: Kirigami.Units.largeSpacing
                        anchors.topMargin: Kirigami.Units.smallSpacing
                        anchors.bottomMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.largeSpacing

                        ColumnLayout {
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            spacing: 0

                            Kirigami.Heading {
                                Layout.fillWidth: true
                                Layout.minimumWidth: 0
                                level: 3
                                text: root.headerTitle()
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.fillWidth: true
                                Layout.minimumWidth: 0
                                text: root.headerSubtitle()
                                font: Kirigami.Theme.smallFont
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

                        // Writing `/etc/xbps.d` needs root, so the button only
                        // opens a confirmation; the job dialog carries progress.
                        // Every guard is its own property: a probe, a system
                        // read or a running apply each make the write unsafe or
                        // pointless, and `pending` keeps it from offering to
                        // write what xbps already uses.
                        QQC2.Button {
                            visible: root.section === "mirrors"
                            text: i18nd("kxbps", "Apply to system")
                            icon.name: "dialog-ok-apply"
                            enabled: mirrorBackend.pending
                                     && !mirrorBackend.probing
                                     && !mirrorBackend.systemLoading
                                     && !mirrorBackend.applyBusy
                            onClicked: mirrorApplyDialog.open()
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
                        onPlanRequested: (name, token) => root.openPackagePlan(name, token)
                    }

                    QueryView {
                        id: queryView
                        catalogBackend: catalogBackend
                    }

                    MirrorsView {
                        mirrorBackend: mirrorBackend
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
