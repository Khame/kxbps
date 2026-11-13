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

    // 16:10, the shape the tool is laid out for, and a 600 px floor: the
    // sidebar, the table and the status bar have to survive it unclipped.
    width: Kirigami.Units.gridUnit * 72
    height: Kirigami.Units.gridUnit * 42
    minimumWidth: Kirigami.Units.gridUnit * 34
    minimumHeight: Kirigami.Units.gridUnit * 28
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
    // starts, so the status strip and the log drawer stay readable afterwards.
    // There is no transaction window: the strip carries the progress and the
    // drawer carries the output, which is where the user watches xbps anyway.
    property var job: null
    property bool showJobResult: false

    // Milliseconds since the job was handed over; the backend turns it into the
    // bar. The 250 ms pass is also what makes the log live: the worker queues
    // lines and `tick` appends them, so they land while xbps runs instead of in
    // one burst after it exits.
    property real jobElapsed: 0

    Timer {
        interval: 250
        repeat: true
        running: root.job !== null && !root.job.done
        onTriggered: {
            if (root.job === null || root.job.done) {
                return
            }
            root.jobElapsed += interval
            root.job.tick(Math.round(root.jobElapsed))
        }
    }

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
            return i18nd("kxbps", "Mirror applied.")
        // The helper's failures. `labelArg` carries only the first verdict
        // argument, which for these keys is the helper's English sentence.
        case "errorMirrorUrlRefused":
            return i18nd("kxbps", "The helper refused the mirror URL.")
        case "errorMirrorWrite":
            return i18nd("kxbps", "Could not write the xbps configuration: %1", job.labelArg)
        case "errorMirrorVerify":
            return i18nd("kxbps", "The new repository URL is not in effect: %1", job.labelArg)
        case "errorMirrorRevert":
            return i18nd("kxbps", "The previous configuration could not be restored: %1", job.labelArg)
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
            item.appBackend = appBackend
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
        // `--filter <token>` opens on a given page, which is what a launcher
        // entry or the tray's "show updates" needs without a second window.
        const at = Qt.application.arguments.indexOf("--filter")
        if (at >= 0 && at + 1 < Qt.application.arguments.length) {
            root.selectFilter(Qt.application.arguments[at + 1])
        }
        // The sidebar lists the repositories xbps reads; the read is cheap
        // (world-readable files plus `xbps-query -L`) and runs with the first
        // frame so the group is not empty on start.
        root.systemRepos = root.parseRepos(mirrorBackend.systemRepos)
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

    // The sidebar is the only navigation, so every page change goes through
    // this one function. A filter change also clears the repository pattern:
    // carrying a search into another page would show rows the page does not own.
    // `Available` has no list of its own — no xbps command lists what a
    // repository holds — so it puts the cursor in the search field instead.
    function selectFilter(token) {
        section = "packages"
        packagesView.clearSearch()
        installedBackend.packages.filter = token
        if (token === "all") {
            packagesView.focusSearch()
        }
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
        if (section === "settings") {
            return i18nd("kxbps", "Settings")
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

    function headerSubtitle(count) {
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
        if (section === "settings") {
            return i18nd("kxbps", "Notifications and startup")
        }
        const filter = installedBackend.packages.filter
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

    // The one sentence for a missing privileged half, chosen by the key the
    // backend sent. A disabled button with no reason is the bug this names.
    function mirrorHelperText(key) {
        switch (key) {
        case "errorMirrorHelperMissing":
            return i18nd("kxbps", "Apply to system needs the installed helper: run packaging/install.sh.")
        case "errorMirrorHelperNotExecutable":
            return i18nd("kxbps", "The mirror helper at %1 is not executable.", "/usr/libexec/kxbps-mirror-apply")
        case "errorMirrorPolicyMissing":
            return i18nd("kxbps", "The polkit policy is missing, so the system cannot ask for a password: run packaging/install.sh.")
        }
        return ""
    }

    // Plan and job objects are C++-parented children of the backend, so the
    // dialog that opens one releases it. `startJob` keeps the job in
    // `root.job`, which the strip and the drawer read; the backend releases the
    // previous one when it hands over the next.
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

    function startJob(started) {
        if (started === null) {
            return
        }
        // The previous job is released when the backend hands over the next
        // one, so nothing here has to free it first.
        job = started
        jobElapsed = 0
        showJobResult = false
        // The drawer scrolls itself: its list follows the last line.
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
    // True while a plan is only being computed so it can be applied at once.
    // The dry run is still what turns the queue into exact versions and sizes,
    // but nothing is shown for it: the queue is the confirmation.
    property bool applyDirectly: false

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
            if (plan === null) {
                root.applyDirectly = false
                return
            }
            if (root.applyDirectly) {
                root.applyDirectly = false
                const started = installedBackend.applyPlan(plan)
                installedBackend.release(plan)
                root.startJob(started)
                return
            }
            planDialog.showPlan(plan)
        }
    }

    BlockedDialog {
        id: blockedDialog
        installedBackend: installedBackend
        onCascadeRequested: (cascadePlan) => planDialog.showPlan(cascadePlan)
    }

    AboutDialog {
        id: aboutDialog
        version: appBackend.version()
    }

    // Applying the selected mirror rewrites the file xbps reads, for every user:
    // the helper runs through pkexec, so the password prompt is the
    // confirmation and the drawer carries the helper's own log. There is no
    // second dialog to click through.
    function applySystemMirror() {
        const started = mirrorBackend.applySystemMirror()
        // Null means nothing was applied: the slot already refused it. The
        // backend clears `applyBusy` and re-reads the system half, so the footer
        // follows without a second read here.
        root.startJob(started)
    }


    // The sidebar's filter entries, in the order the tool reads them: the
    // installed set, what can move, the two states worth acting on, the queue,
    // and the repositories — which are only reachable through a search.
    // Most used first: the installed set, the updates, the search for new
    // packages, then the queue and the diagnostic filters.
    readonly property var filterEntries: [
        { token: "installed", label: i18nd("kxbps", "Installed"), icon: "package-installed-updated" },
        { token: "updates", label: i18nd("kxbps", "Upgradable"), icon: "system-software-update", alert: true },
        { token: "all", label: i18nd("kxbps", "Available"), icon: "system-search", focusSearch: true },
        { token: "queued", label: i18nd("kxbps", "Marked"), icon: "view-task" },
        { token: "explicit", label: i18nd("kxbps", "Explicit"), icon: "package-x-generic" },
        { token: "orphans", label: i18nd("kxbps", "Orphans"), icon: "package-remove" },
        { token: "held", label: i18nd("kxbps", "On hold"), icon: "object-locked" }
    ]

    // `xbps-query -L` plus the `.conf` files, as `{url, count, signed, file}`.
    // The sidebar only reports them: a package's repository is known for search
    // hits alone, so the rows must not pretend to filter the installed set.
    property var systemRepos: []
    function parseRepos(json) {
        try {
            const parsed = JSON.parse(json)
            if (!Array.isArray(parsed)) {
                return []
            }
            // `-L` and the `.conf` files can name the same repository more than
            // once (a base URL and its `/current` form, or several files with
            // the same value). The sidebar is a list of repositories, not of
            // files, so one row per repository is the honest view.
            const seen = []
            return parsed.filter(repo => {
                if (typeof repo.url !== "string" || repo.url.length === 0) {
                    return false
                }
                const key = repo.url.replace(/\/+$/, "").replace(/\/current$/, "")
                if (seen.indexOf(key) >= 0) {
                    return false
                }
                seen.push(key)
                return true
            })
        } catch (error) {
            return []
        }
    }
    // A repository URL as a sidebar row: the host, then the path that tells two
    // repositories on one host apart. `https://repo-fi.voidlinux.org/current`
    // reads as `repo-fi.voidlinux.org`. A third-party repository keeps its first
    // path segment, because the host alone would name two of them the same.
    function shortRepo(url) {
        let rest = url.indexOf("://") >= 0 ? url.slice(url.indexOf("://") + 3) : url
        if (rest.endsWith("/")) {
            rest = rest.slice(0, -1)
        }
        const slash = rest.indexOf("/")
        if (slash < 0) {
            return rest
        }
        const host = rest.slice(0, slash)
        const path = rest.slice(slash + 1)
        // The official layout is `<base>/current[/<component>]`. The component
        // after `current` is what tells multilib from nonfree; dropping it made
        // three different repositories read as the same row.
        const tail = path === "current" ? "" : path.replace(/^current\//, "")
        return tail.length === 0 ? host : host + "/" + tail
    }

    // A repository is in use when its URL is the one the system half reports:
    // `-L` lists `<base>current`, so both spellings match.
    function repoInUse(url) {
        const base = mirrorBackend.systemMirror
        if (base.length === 0 || url.length === 0) {
            return false
        }
        return url === base || url === base.replace(/\/+$/, "") + "/current"
    }

    // Counts for the status bar: what the filters show is not the machine's
    // total, so the total is summed once here over the model's own filters.
    property int upgradableBadge: installedBackend.updatesCount

    // The repository rows open the Mirrors page on the repository they name.
    function showRepository(url) {
        section = "mirrors"
        if (url.length > 0) {
            mirrorBackend.currentMirror = url
        }
    }

    // Refreshing xbps: the installed set and the repository list are two
    // different reads, and the toolbar's button refreshes both.
    function refreshAll() {
        installedBackend.reload()
        mirrorBackend.refreshSystem()
    }

    // The queue, applied from the toolbar's Apply button and the drawer's
    // Review. The user has already confirmed by marking packages, so the dry run
    // runs silently and the transaction starts the moment xbps answers: the
    // first thing to appear is the password prompt, not another window.
    function reviewQueue() {
        if (installedBackend.queueCount === 0) {
            return
        }
        root.applyDirectly = true
        root.planPending = installedBackend.openQueuePlan()
    }

    pageStack.initialPage: Kirigami.Page {
        padding: 0
        // The page carries no header of its own: the window draws one toolbar
        // for the whole window, and a second title above the table would say
        // what the selected sidebar row already says.

        RowLayout {
            anchors.fill: parent
            spacing: 0

            // The persistent sidebar. Navigation, not decoration: its rows
            // change what the table shows and nothing else.
            Rectangle {
                Layout.preferredWidth: Kirigami.Units.gridUnit * (root.narrow ? 10 : 12.5)
                Layout.minimumWidth: Kirigami.Units.gridUnit * (root.narrow ? 10 : 12.5)
                Layout.maximumWidth: Kirigami.Units.gridUnit * (root.narrow ? 10 : 12.5)
                Layout.fillHeight: true
                color: Kirigami.Theme.alternateBackgroundColor

                // The sidebar's right edge: a vertical hairline, drawn as a
                // themed rectangle because a Separator is horizontal.
                Rectangle {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: Kirigami.Units.smallSpacing / 2
                    color: Kirigami.Theme.disabledTextColor
                }

                ColumnLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Kirigami.Units.smallSpacing
                    anchors.rightMargin: Kirigami.Units.smallSpacing
                    anchors.topMargin: Kirigami.Units.smallSpacing
                    spacing: 0

                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        text: i18nd("kxbps", "Filter")
                        font.family: Kirigami.Theme.smallFont.family
                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                        font.capitalization: Font.AllUppercase
                        color: Kirigami.Theme.disabledTextColor
                        elide: Text.ElideRight
                    }

                    Repeater {
                        model: root.filterEntries

                        NavItem {
                            required property var modelData
                            Layout.fillWidth: true
                            label: modelData.label
                            iconName: modelData.icon
                            badgeText: modelData.token === "updates" && installedBackend.updatesCount > 0
                                       ? String(installedBackend.updatesCount)
                                       : (modelData.token === "queued" && installedBackend.queueCount > 0
                                          ? String(installedBackend.queueCount) : "")
                            badgeAlert: modelData.alert === true
                            highlighted: root.section === "packages"
                                         && installedBackend.packages.filter === modelData.token
                            onClicked: {
                                root.selectFilter(modelData.token)
                                if (modelData.focusSearch === true) {
                                    packagesView.focusSearch()
                                }
                            }
                        }
                    }

                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        text: i18nd("kxbps", "Repository")
                        font.family: Kirigami.Theme.smallFont.family
                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                        font.capitalization: Font.AllUppercase
                        color: Kirigami.Theme.disabledTextColor
                        elide: Text.ElideRight
                    }

                    Repeater {
                        model: root.systemRepos

                        NavItem {
                            required property var modelData
                            Layout.fillWidth: true
                            compact: true
                            muted: true
                            label: root.shortRepo(modelData.url)
                            badgeText: root.repoInUse(modelData.url) ? i18nd("kxbps", "In use") : ""
                            Accessible.name: modelData.url
                            QQC2.ToolTip.visible: hovered
                            QQC2.ToolTip.text: modelData.url
                            onClicked: root.showRepository(modelData.url)
                        }
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        visible: root.systemRepos.length === 0
                        text: i18nd("kxbps", "No repository read yet")
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.disabledTextColor
                        elide: Text.ElideRight
                    }

                    Item {
                        Layout.fillHeight: true
                    }

                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                    }

                    QQC2.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        Layout.bottomMargin: Kirigami.Units.smallSpacing
                        text: i18nd("kxbps", "Tools")
                        font.family: Kirigami.Theme.smallFont.family
                        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                        font.capitalization: Font.AllUppercase
                        color: Kirigami.Theme.disabledTextColor
                        elide: Text.ElideRight
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Query")
                        iconName: "utilities-terminal"
                        highlighted: root.section === "query"
                        onClicked: root.section = "query"
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Mirrors")
                        iconName: "network-server"
                        highlighted: root.section === "mirrors"
                        onClicked: root.section = "mirrors"
                    }

                    Kirigami.Separator {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.smallSpacing
                    }

                    // Settings sits at the bottom, the place a desktop
                    // application keeps it.
                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Settings")
                        iconName: "settings-configure"
                        highlighted: root.section === "settings"
                        onClicked: root.section = "settings"
                    }

                    // The architecture is not exposed by any backend, so the
                    // foot names the system it manages and colours the dot by
                    // whether the last read failed.
                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.smallSpacing
                        Layout.margins: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        Rectangle {
                            Layout.preferredWidth: Kirigami.Units.smallSpacing * 1.5
                            Layout.preferredHeight: Kirigami.Units.smallSpacing * 1.5
                            radius: Layout.preferredWidth / 2
                            color: installedBackend.error.length > 0 ? Kirigami.Theme.negativeTextColor
                                                                     : Kirigami.Theme.positiveTextColor
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            text: i18nd("kxbps", "Void Linux")
                            font: Kirigami.Theme.smallFont
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            // The page column: one toolbar, the page, the xbps log strip, the
            // status bar. Nothing above the toolbar repeats what it says.
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: toolbarLayout.implicitHeight
                                    + Kirigami.Units.smallSpacing * 2
                    color: Kirigami.Theme.backgroundColor

                    Kirigami.Separator {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                    }

                    RowLayout {
                        id: toolbarLayout
                        anchors.fill: parent
                        anchors.leftMargin: Kirigami.Units.smallSpacing
                        anchors.rightMargin: Kirigami.Units.largeSpacing
                        anchors.topMargin: Kirigami.Units.smallSpacing
                        anchors.bottomMargin: Kirigami.Units.smallSpacing
                        spacing: Kirigami.Units.smallSpacing

                        // No menu button: navigation is the sidebar, and every
                        // page action is a control on this one toolbar. A
                        // hamburger would repeat both.
                        //
                        // A button, not a tool button: the row is buttons, and
                        // a flat icon among bordered ones reads as disabled.
                        QQC2.Button {
                            icon.name: "view-refresh"
                            text: i18nd("kxbps", "Refresh")
                            Accessible.name: i18nd("kxbps", "Refresh")
                            font: Kirigami.Theme.smallFont
                            enabled: !installedBackend.loading
                            onClicked: root.refreshAll()
                        }

                        Kirigami.SearchField {
                            id: toolbarSearch
                            Layout.fillWidth: true
                            Layout.minimumWidth: Kirigami.Units.gridUnit * 6
                            // The repository query belongs to the package page;
                            // a pattern typed elsewhere would also change what
                            // that page shows when it comes back.
                            enabled: root.section === "packages"
                            placeholderText: i18nd("kxbps", "Search packages…")
                            text: packagesView.searchText
                            onTextChanged: packagesView.searchText = text
                            onAccepted: packagesView.submitSearch()
                        }

                        // The two bulk gestures. Below 60 gu they keep their
                        // icons and drop the words, so the toolbar stays one row
                        // without losing an action.
                        QQC2.Button {
                            visible: root.section === "packages"
                            text: i18nd("kxbps", "Mark all upgrades")
                            display: root.narrow ? QQC2.AbstractButton.IconOnly
                                                 : QQC2.AbstractButton.TextBesideIcon
                            Accessible.name: i18nd("kxbps", "Mark all upgrades")
                            icon.name: "checkmark"
                            font: Kirigami.Theme.smallFont
                            enabled: installedBackend.updatesCount > 0
                            onClicked: installedBackend.queueAllUpdates()
                        }

                        QQC2.Button {
                            visible: root.section === "packages"
                            text: i18nd("kxbps", "Unmark all")
                            display: root.narrow ? QQC2.AbstractButton.IconOnly
                                                 : QQC2.AbstractButton.TextBesideIcon
                            Accessible.name: i18nd("kxbps", "Unmark all")
                            icon.name: "edit-clear"
                            font: Kirigami.Theme.smallFont
                            enabled: installedBackend.queueCount > 0
                            onClicked: installedBackend.clearQueue()
                        }

                        // The one accent action of the window: it opens the dry
                        // run, which is the only step that can change the system.
                        QQC2.Button {
                            visible: root.section === "packages"
                            text: installedBackend.queueCount > 0
                                  ? i18ndp("kxbps", "Apply (%1)", "Apply (%1)", installedBackend.queueCount)
                                  : i18nd("kxbps", "Apply")
                            icon.name: "dialog-ok-apply"
                            highlighted: installedBackend.queueCount > 0
                            enabled: installedBackend.queueCount > 0
                            onClicked: root.reviewQueue()
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
                        QQC2.Button {
                            visible: root.section === "mirrors"
                            text: i18nd("kxbps", "Apply to system")
                            icon.name: "dialog-ok-apply"
                            enabled: mirrorBackend.pending
                                     && !mirrorBackend.probing
                                     && !mirrorBackend.systemLoading
                                     && !mirrorBackend.applyBusy
                                     && mirrorBackend.helperReady
                            onClicked: root.applySystemMirror()
                        }
                    }
                }

                // The page's own title, kept as a single muted line so the
                // toolbar stays one row and the table starts as high as it can.
                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    text: root.headerTitle() + " · " + root.headerSubtitle(installedBackend.packages.count)
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.disabledTextColor
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    visible: root.section === "mirrors" && mirrorBackend.helperErrorKey.length > 0
                    text: visible ? root.mirrorHelperText(mirrorBackend.helperErrorKey) : ""
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.negativeTextColor
                    wrapMode: Text.WordWrap
                }

                StackLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    currentIndex: {
                        switch (root.section) {
                        case "query":
                            return 1
                        case "mirrors":
                            return 2
                        case "settings":
                            return 3
                        }
                        return 0
                    }

                    PackagesView {
                        id: packagesView
                        packages: installedBackend.packages
                        installedBackend: installedBackend
                        catalogBackend: catalogBackend
                        job: root.job
                        onUpdateAllRequested: root.openSystemPlan()
                        onPlanRequested: (name, token) => root.openPackagePlan(name, token)
                        onSearchFieldRequested: toolbarSearch.forceActiveFocus()
                    }

                    QueryView {
                        id: queryView
                        catalogBackend: catalogBackend
                    }

                    MirrorsView {
                        mirrorBackend: mirrorBackend
                    }

                    SettingsView {
                        prefsBackend: prefsBackend
                        onAboutRequested: aboutDialog.open()
                    }
                }

                StatusStrip {
                    Layout.fillWidth: true
                    implicitHeight: Kirigami.Units.gridUnit * 2
                    installedBackend: installedBackend
                    job: root.job
                    jobLabel: root.jobLabel
                    showResult: root.showJobResult
                    currentMirror: mirrorBackend.systemMirror.length > 0
                                   ? mirrorBackend.systemMirror
                                   : mirrorBackend.currentMirror
                    totalCount: installedBackend.packages.count
                    installedCount: installedBackend.installedCount
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

    // The repository list is read once at start, and again whenever the
    // Mirrors page or a job changes what xbps reads.
    Connections {
        target: mirrorBackend
        function onSystemReposChanged() {
            root.systemRepos = root.parseRepos(mirrorBackend.systemRepos)
        }
    }

    // The sidebar's repository rows and the Mirrors page share one read, so
    // the first frame does not sit on an empty list.
    onVisibleChanged: {
        if (visible) {
            mirrorBackend.refreshSystem()
        }
    }
}
