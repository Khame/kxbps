import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import org.kde.kxbps
import "components"
import "views"

Kirigami.ApplicationWindow {
    id: root

    // 16:10, the shape the tool is laid out for, clamped to the screen: at a
    // 2x scale the nominal size is wider than a 1280 px display, and a window
    // larger than the screen clips its own toolbar.
    width: Math.min(Kirigami.Units.gridUnit * 72, Screen.width * 0.9)
    height: Math.min(Kirigami.Units.gridUnit * 42, Screen.height * 0.9)
    minimumWidth: Math.min(Kirigami.Units.gridUnit * 34, Screen.width * 0.9)
    minimumHeight: Math.min(Kirigami.Units.gridUnit * 28, Screen.height * 0.9)

    // Below 60 grid units the toolbar drops the words and the sidebar narrows.
    // The measure is in grid units, so a scaled display counts too: a 1280 px
    // window at 2x is 35 units, not 70.
    readonly property bool narrow: width < Kirigami.Units.gridUnit * 60
    // QQmlApplicationEngine does not show the root window on its own.
    visible: true
    title: i18nd("kxbps", "KXbps")

    // The window draws its own header; Kirigami's application toolbar would put
    // the page title above it a second time.
    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None

    // Which sidebar entry is active. The package sections share one view and
    // differ only by the model's filter; Mirrors, Repositories, Settings and
    // Help are pages of their own.
    property string section: "packages"

    // The current or last finished job. It is kept until the next transaction
    // starts, so the status strip and the log drawer stay readable afterwards.
    // There is no transaction window: the strip carries the progress and the
    // drawer carries the output, which is where the user watches xbps anyway.
    property var job: null
    property bool showJobResult: false

    // The one sentence the status strip carries when a transaction never
    // started: the direct queue path runs its dry run without a dialog, so a
    // refusal or an empty answer has nowhere else to appear. Empty means the
    // strip says nothing beyond its counts.
    property string stripMessage: ""
    property bool stripMessageIsError: false

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
        // The mirror is written, but `xbps-install -S` did not finish, so the
        // package lists are still short: the sentence has to say both.
        case "mirrorSyncFailed":
            return i18nd("kxbps", "Mirror set, but the package index could not be refreshed: %1", job.labelArg)
        case "repoAddDone":
            return i18nd("kxbps", "Repository %1 added.", job.labelArg)
        case "repoRemoveDone":
            return i18nd("kxbps", "Repository %1 removed.", job.labelArg)
        case "repoSyncFailed":
            return i18nd("kxbps", "Repository %1 added, but the package index could not be refreshed", job.labelArg)
        case "errorRepoUrl":
            return i18nd("kxbps", "The helper refused the repository URL.")
        case "errorRepoPresent":
            return i18nd("kxbps", "A file for %1 already exists.", job.labelArg)
        case "errorRepoNotManaged":
            return i18nd("kxbps", "That file was not written by KXbps.")
        case "errorRepoIo":
            return i18nd("kxbps", "Could not write the repository configuration.")
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

    // The xbps reads behind the details pane and the Available listing
    // (`get`, `readPackageFiles`, `readFileOwner`, `readListSearch`).
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

    // The tray icon is a StatusNotifierItem owned by C++ (`tray_shim.h`): the
    // menu is exported over D-Bus, so no widget popup exists for Wayland to
    // grab. The window only feeds it the count and answers its two requests.
    Binding {
        target: trayBackend
        property: "updateCount"
        value: installedBackend !== null ? installedBackend.updatesCount : 0
    }

    // The desktop notification. The notifier itself never fails to load: it is
    // a `QtObject` with no module of its own beyond KI18n, which the window
    // already imports.
    Loader {
        id: notifierLoader
        source: "components/Notifier.qml"
        asynchronous: true
        onLoaded: {
            item.installedBackend = installedBackend
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
        // The local read above only knows the last sync; the fresh check fills
        // the gap when it is due. It runs after the read so the table appears
        // immediately and the check never delays the first frame.
        installedBackend.checkUpdates(false)
        // `--filter <token>` opens on a given page, which is what a launcher
        // entry or the tray's "show updates" needs without a second window.
        const at = Qt.application.arguments.indexOf("--filter")
        if (at >= 0 && at + 1 < Qt.application.arguments.length) {
            root.selectFilter(Qt.application.arguments[at + 1])
        }
        // `--section <name>` opens on a page, for screenshots and for a launcher
        // that points at one: packages, mirrors, repositories, settings, help.
        const sectionAt = Qt.application.arguments.indexOf("--section")
        if (sectionAt >= 0 && sectionAt + 1 < Qt.application.arguments.length) {
            const name = Qt.application.arguments[sectionAt + 1]
            if (["packages", "mirrors", "settings", "repositories", "help"].indexOf(name) >= 0) {
                root.section = name
            }
        }
    }

    // Whether closing the window should hand it to the tray. The C++ side
    // checks for a status notifier host, which is what decides both the icon
    // and whether hiding the window can strand the process.
    readonly property bool trayAvailable: appBackend.trayAvailable

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

    // The announcement was delivered, seen or ignored: the backend wrote the
    // bookkeeping in the same call that offered it, so nothing is written here.
    // The hook stays because the start sequence and the signature watcher both
    // pass through it.

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

    // The fresh check: the repositories are asked for updates, and the answer
    // replaces the list when it succeeds. It runs in the tray too — a long-lived
    // instance must still notice new updates — so the timer never stops. The
    // backend decides whether the check is due and whether the network is
    // reachable; QML only says "now would be a good time".
    Timer {
        interval: prefsBackend.updateCheckMinutes * 60 * 1000
        running: true
        repeat: true
        onTriggered: {
            // The list on screen ages too: refresh the installed set, then ask
            // the repositories. A local read that lands after a fresh remote
            // one keeps the remote list (the backend decides).
            installedBackend.reload()
            installedBackend.checkUpdates(false)
        }
        onIntervalChanged: restart()
    }

    // The same check after a local read: the index only knows what the last
    // sync left, so the list on screen is as old as that. The backend skips it
    // when the last check is younger than its gap.
    function checkOnShow() {
        if (root.visible) {
            installedBackend.reload()
            installedBackend.checkUpdates(false)
        }
    }

    // The header follows the filter, not the sidebar entry: clicking "Orphans"
    // must not keep saying "Installed".
    function headerTitle() {
        if (section === "mirrors") {
            return i18nd("kxbps", "Mirrors")
        }
        if (section === "settings") {
            return i18nd("kxbps", "Settings")
        }
        if (section === "repositories") {
            return i18nd("kxbps", "Repositories")
        }
        if (section === "help") {
            return i18nd("kxbps", "Help")
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
        return ""
    }

    // The muted band above the page: the title, and the subtitle only where the
    // section has one. A page with nothing to add says nothing, rather than
    // trailing a separator into the band.
    function headerLine(count) {
        const subtitle = headerSubtitle(count)
        return subtitle.length > 0 ? headerTitle() + " · " + subtitle : headerTitle()
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
    // `root.job`, which the bar and the drawer read; the backend releases the
    // previous one when it hands over the next.
    //
    // A plan is a dry run, so `openQueuePlan` only starts a worker thread and
    // answers with a request id; the object arrives through `onPlanReady`. The
    // backend drops an answer whose request a newer one superseded, so a plan
    // is only ever shown for the state the user asked from.
    function startJob(started) {
        if (started === null) {
            return
        }
        // The previous job is released when the backend hands over the next
        // one, so nothing here has to free it first.
        job = started
        jobElapsed = 0
        showJobResult = false
        // The sentence about the last dry run described a queue that has now
        // turned into a transaction.
        root.stripMessage = ""
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
            // The sentence described the queue the user just changed.
            root.stripMessage = ""
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

    // Whether the next announcement offer may leave the start sequence: the
    // start switch gates it, and a popup that went out closes the sequence; a
    // deferred start offer is retried with this still true.
    property bool startupOffer: true

    // The notification tries exactly once per start, when the first read has
    // answered. `stateRevision` is not the hook: a hold toggle moves it too,
    // and the popup is a start-time event.
    //
    // A Loader's `onLoaded` alone is not enough either: a notifier that
    // finishes loading after the read answered never sees another
    // `loadingChanged`. Both places call `updateStartNotified`, which runs the
    // check at most once.
    property bool startNotified: false

    // Why the last call was silent, when it was silent only because the window
    // is on screen: the offer was deferred, not dropped, so a timer retries it
    // once the window is hidden. Rust writes nothing for a deferred offer.
    property bool announcementDeferred: false

    function updateStartNotified() {
        if (root.startNotified || installedBackend.loading || notifierLoader.item === null) {
            return
        }
        root.startNotified = true
        root.considerAnnouncement()
    }

    // Offers one announcement and keeps the reminder cycle in step: a delivered
    // popup arms the one-shot reminder, a deferred one arms the retry, and a
    // reminder that went out stops both.
    function considerAnnouncement() {
        if (notifierLoader.item === null) {
            return
        }
        const kind = notifierLoader.item.consider(root.visible, root.startupOffer)
        if (kind !== "") {
            // A popup went out — the start offer counted or a running check
            // delivered — so the start gate is closed for the next cycle.
            root.startupOffer = false
        }
        if (kind === "first") {
            reminderTimer.restart()
            root.announcementDeferred = false
        } else if (kind === "reminder") {
            reminderTimer.stop()
            root.announcementDeferred = false
        } else {
            // Nothing sent. When the window is on screen the offer was deferred
            // (the backend writes nothing for it), so it is due the moment the
            // window is hidden — the handler below. Otherwise nothing is due at
            // all, and only the reminder timer or a new set can change that.
            root.announcementDeferred = root.visible
            reminderTimer.stop()
        }
    }

    // The one reminder, three hours after the first popup. A restart past the
    // three-hour mark is caught by the start call instead; this only covers a
    // running instance.
    Timer {
        id: reminderTimer
        interval: 3 * 60 * 60 * 1000
        repeat: false
        onTriggered: root.considerAnnouncement()
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
        // The first consideration belongs to the start sequence; a later one is
        // a check the app runs while it is open, which the start switch does not
        // gate.
        function onPendingSignatureChanged() {
            const first = !root.startNotified
            root.updateStartNotified()
            if (first) {
                return
            }
            root.startupOffer = false
            root.considerAnnouncement()
        }

        // The dock badge. Plasma's task manager reads the count from the Unity
        // `LauncherEntry.Update` signal Qt sends, and zero clears it. A
        // windowless `--tray` instance has no task button to badge, which is why
        // the tray icon stays the indicator there.
        function onUpdatesCountChanged() {
            appBackend.setBadge(installedBackend.updatesCount)
        }
    }

    // A click on the popup. The bus refuses activation without a token on
    // Wayland, so the window is shown and raised, and `requestActivate` is only
    // a request: visibility is what the user is guaranteed.
    Connections {
        target: appBackend
        function onNotificationActivated() {
            root.showUpdates()
        }
    }

    Connections {
        target: trayBackend
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
                // A transaction can move the repository index; the next
                // Available visit reads it instead of the cached listing.
                catalogBackend.invalidateListSearch()
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
                root.showPlanMessage(null)
                return
            }
            if (root.applyDirectly) {
                root.applyDirectly = false
                // A blocked transaction cannot run at all, and the dialog is
                // the only surface that names what still needs the packages.
                if (plan.isBlocked) {
                    planDialog.showPlan(plan)
                    return
                }
                // Nothing to apply means no job. xbps either refused the dry
                // run or found the queue already done; the strip is where the
                // silent path says so, because a click that does nothing reads
                // as a broken button.
                if (plan.errorKey.length > 0 || plan.noticeKey.length > 0
                        || plan.count === 0) {
                    root.showPlanMessage(plan)
                    installedBackend.release(plan)
                    return
                }
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
        qtVersion: appBackend.qtVersion()
        kfVersion: appBackend.kfVersion()
        platformName: appBackend.platformName()
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
        { token: "installed", label: i18nd("kxbps", "Installed"), icon: "installed" },
        { token: "updates", label: i18nd("kxbps", "Upgradable"), icon: "updates", alert: true },
        { token: "all", label: i18nd("kxbps", "Available"), icon: "available", focusSearch: true },
        { token: "queued", label: i18nd("kxbps", "Marked"), icon: "selected" },
        { token: "explicit", label: i18nd("kxbps", "Explicit"), icon: "manual" },
        { token: "orphans", label: i18nd("kxbps", "Orphans"), icon: "orphaned" },
        { token: "held", label: i18nd("kxbps", "On hold"), icon: "locked" }
    ]

    // Refreshing xbps: the installed set and the repository list are two
    // different reads, and the toolbar's button refreshes both. The cached
    // repository listing is dropped with them, so Available reads the index
    // again instead of showing rows it has moved past.
    function refreshAll() {
        installedBackend.reload()
        mirrorBackend.refreshSystem()
        catalogBackend.invalidateListSearch()
    }

    // The queue, applied from the toolbar's Apply button and the drawer's
    // Review. The user has already confirmed by marking packages, so the dry run
    // runs silently and the transaction starts the moment xbps answers: the
    // first thing to appear is the password prompt, not another window.
    function reviewQueue() {
        if (installedBackend.queueCount === 0) {
            return
        }
        root.stripMessage = ""
        root.applyDirectly = true
        root.planPending = installedBackend.openQueuePlan()
    }

    // A dry run that ends without a transaction still has to say why. The
    // sentence comes from the same mapping the plan dialog uses, so the strip
    // and the dialog never word the same answer differently. A null plan means
    // the payload could not be built at all.
    // The age of the last successful check, in words: the one number every
    // failure sentence ends with. Pluralised with `i18ndp`, never glued from
    // fragments. `0` means no check ever succeeded.
    function ageText(milliseconds) {
        if (milliseconds <= 0) {
            return i18nd("kxbps", "Not checked yet")
        }
        const minutes = Math.floor(milliseconds / 60000)
        if (minutes < 60) {
            return i18ndp("kxbps", "%1 minute ago", "%1 minutes ago", Math.max(1, minutes))
        }
        const hours = Math.floor(minutes / 60)
        if (hours < 24) {
            return i18ndp("kxbps", "%1 hour ago", "%1 hours ago", hours)
        }
        const days = Math.floor(hours / 24)
        return i18ndp("kxbps", "%1 day ago", "%1 days ago", days)
    }

    // The sentence the last check produced. The backend hands a key and its
    // arguments; only the age needs wording here, because only QML can
    // pluralise it, and the backend always puts it last.
    function checkText() {
        const key = installedBackend.checkKey
        if (key.length === 0) {
            return ""
        }
        const args = installedBackend.checkArgs
        const age = args.length > 0 ? root.ageText(Number(args[args.length - 1])) : ""
        const head = args.length > 1 ? args.slice(0, args.length - 1) : []
        switch (key) {
        case "checkUpToDate":
            return i18nd("kxbps", "System is up to date")
        case "checkUpdatesFound":
            return i18ndp("kxbps", "%1 update available", "%1 updates available", Number(head[0]))
        case "checkOffline":
            return i18nd("kxbps", "No network connection. The update list could not be refreshed; it is from the last successful check (%1).", age)
        case "checkRepositoriesFailed":
            return i18nd("kxbps", "Could not read %1 of %2 repositories (%3). Last successful check: %4.", head[0], head[1], head[2], age)
        case "checkReadFailed":
            return i18nd("kxbps", "The repositories could not be read. The list below is unchanged. Last successful check: %1.", age)
        }
        return ""
    }

    // The strip's sentence: a running check says so, then the last answer.
    // A binding, not a function call at the call site, so the strip follows
    // every property change the backend emits.
    readonly property string checkSentence: {
        if (installedBackend.checking) {
            return i18nd("kxbps", "Checking for updates…")
        }
        return root.checkText()
    }

    function showPlanMessage(plan) {
        let text = ""
        let failed = true
        if (plan !== null && plan.errorKey.length > 0) {
            text = planDialog.errorTextFor(plan)
        } else if (plan !== null && plan.noticeKey.length > 0) {
            text = planDialog.noticeTextFor(plan)
            failed = false
        }
        root.stripMessageIsError = failed
        root.stripMessage = text.length > 0 ? text
                                            : i18nd("kxbps", "The dry run returned nothing that could be applied.")
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
                        label: i18nd("kxbps", "Mirrors")
                        iconName: "mirrors"
                        highlighted: root.section === "mirrors"
                        onClicked: root.section = "mirrors"
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Repositories")
                        iconName: "repositories"
                        highlighted: root.section === "repositories"
                        onClicked: root.section = "repositories"
                    }

                    NavItem {
                        Layout.fillWidth: true
                        label: i18nd("kxbps", "Help")
                        iconName: "help"
                        highlighted: root.section === "help"
                        onClicked: root.section = "help"
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
                        iconName: "settings"
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

                // The window toolbar. It exists only where it carries an
                // action: the package list and the mirrors page. The other
                // pages have their own controls, and an empty bar above them
                // said nothing.
                Rectangle {
                    Layout.fillWidth: true
                    visible: root.section === "packages" || root.section === "mirrors"
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
                        // the shared component is what keeps them at one height
                        // and one font step.
                        DenseButton {
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
                            visible: root.section === "packages"
                            placeholderText: i18nd("kxbps", "Search packages…")
                            text: packagesView.searchText
                            onTextChanged: packagesView.searchText = text
                            onAccepted: packagesView.submitSearch()
                        }

                        // The two bulk gestures. Below 60 gu they keep their
                        // icons and drop the words, so the toolbar stays one row
                        // without losing an action.
                        DenseButton {
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

                        DenseButton {
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
                        DenseButton {
                            visible: root.section === "packages"
                            text: installedBackend.queueCount > 0
                                  ? i18ndp("kxbps", "Apply (%1)", "Apply (%1)", installedBackend.queueCount)
                                  : i18nd("kxbps", "Apply")
                            icon.name: "dialog-ok-apply"
                            // The frame is what separates the accent action
                            // from the flat controls around it.
                            flat: false
                            highlighted: installedBackend.queueCount > 0
                            enabled: installedBackend.queueCount > 0
                            onClicked: root.reviewQueue()
                        }

                        DenseButton {
                            visible: root.section === "mirrors"
                            text: i18nd("kxbps", "Probe all")
                            icon.name: "network-connect"
                            enabled: !mirrorBackend.probing
                            onClicked: mirrorBackend.probeAll()
                        }

                        // Writing `/etc/xbps.d` needs root, so the button only
                        // opens a confirmation; the job dialog carries progress.
                        DenseButton {
                            visible: root.section === "mirrors"
                            text: i18nd("kxbps", "Apply to system")
                            icon.name: "dialog-ok-apply"
                            // The other accent action: it writes /etc/xbps.d.
                            flat: false
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
                // The band stays even when it has nothing to say: it separates
                // the toolbar from the table. Counts belong to the page footer,
                // not here.
                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    Layout.topMargin: Kirigami.Units.smallSpacing
                    text: root.headerLine(installedBackend.packages.count)
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
                        case "mirrors":
                            return 1
                        case "settings":
                            return 2
                        case "repositories":
                            return 3
                        case "help":
                            return 4
                        }
                        return 0
                    }

                    PackagesView {
                        id: packagesView
                        packages: installedBackend.packages
                        installedBackend: installedBackend
                        catalogBackend: catalogBackend
                        job: root.job
                        onSearchFieldRequested: toolbarSearch.forceActiveFocus()
                    }

                    MirrorsView {
                        mirrorBackend: mirrorBackend
                    }

                    SettingsView {
                        prefsBackend: prefsBackend
                        onAboutRequested: aboutDialog.open()
                    }

                    RepositoriesView {
                        mirrorBackend: mirrorBackend
                        onJobStarted: (started) => root.startJob(started)
                    }

                    HelpView {}
                }

                StatusStrip {
                    Layout.fillWidth: true
                    implicitHeight: Kirigami.Units.gridUnit * 2
                    job: root.job
                    jobLabel: root.jobLabel
                    showResult: root.showJobResult
                    message: root.stripMessage
                    messageIsError: root.stripMessageIsError
                    checkMessage: root.checkSentence
                    checkIsError: !installedBackend.checking
                                  && installedBackend.checkErrorKey.length > 0
                    logOpen: logDrawer.expanded
                    onCancelRequested: {
                        if (root.job !== null) {
                            root.job.cancel()
                        }
                    }
                    onLogToggleRequested: logDrawer.toggleExpanded()
                }

                LogDrawer {
                    id: logDrawer
                    Layout.fillWidth: true
                    job: root.job
                    checkLines: installedBackend.checkLog
                }
            }
        }
    }

    // The repository list is read once at start, and again whenever the
    // Mirrors page or a job changes what xbps reads.
    onVisibleChanged: {
        if (visible) {
            mirrorBackend.refreshSystem()
            // A window brought back from the tray shows a list as old as the
            // last check; the backend skips this when that check is fresh.
            root.checkOnShow()
        } else if (root.announcementDeferred) {
            // The window was hidden: the deferred announcement is due now.
            root.considerAnnouncement()
        }
    }

    onSectionChanged: {
        if (section === "repositories") {
            mirrorBackend.refreshSystem()
        }
    }
}
