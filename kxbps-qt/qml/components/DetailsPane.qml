// Everything xbps reports about one package, with the two actions that make
// sense from a list: queue a change and hold the package.
//
// The pane reads `catalogBackend.get(pkgver)` instead of the model row, because
// QML cannot index a `QAbstractItemModel` by row number. The facts come from
// xbps itself: the owner calls `requestFacts(pkgver)` when the selection moves,
// and `factsRevision` says when the answer can be read. Both the dependencies
// and the reverse dependencies are xbps' own lists (`-x` and `-X`), so nothing
// here scans another list to guess them.
//
// Two placements, one file: a side pane on wide views, a drawer on narrow ones
// (see `PackagesView`).
//
// The content is a `Flickable`: the fact grid plus a full "required by" list is
// taller than the slot, and a plain ColumnLayout would draw its children over
// the status strip instead of scrolling.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Flickable {
    id: pane

    property var catalogBackend
    property var installedBackend
    // The selected package as `name-version_revision`; empty closes the pane.
    property string packageName: ""

    // Bumped when the store changes, so `info` re-reads the snapshot: the
    // backend hands out a copy and has no notify signal for `get()`.
    property int refreshToken: 0
    property var blockedNames: []
    // The selection the blocked names belong to, so re-reading a fact does not
    // rebuild the plan.
    property string blockedPkgver: ""

    readonly property var info: {
        pane.refreshToken
        pane.catalogBackend.factsRevision
        if (pane.packageName.length === 0) {
            return ({})
        }
        if (pane.catalogBackend.factsPkgver !== pane.packageName) {
            // The selection moved on: the previous facts are not this package.
            return ({})
        }
        return pane.catalogBackend.get(pane.packageName)
    }
    readonly property bool installed: pane.info.installed === true
    readonly property var dependents: pane.info.requiredBy !== undefined && pane.info.requiredBy.length > 0
                                     ? pane.info.requiredBy.split(", ") : []
    readonly property bool orphan: pane.installed && !pane.info.explicit && pane.dependents.length === 0

    // The version, even before the facts arrive: the pkgver carries it.
    readonly property string version: pane.catalogBackend.versionOf(pane.packageName)
    readonly property string name: pane.info.name !== undefined
                                  ? pane.info.name
                                  : pane.packageName.split("-")[0]
    // The facts failed to load: say so rather than showing blanks.
    // The half of the read that mirrors `packageName`, so a check that needs
    // `info` waits for the facts instead of reading a map for another package.
    readonly property string factsPkgver: pane.catalogBackend.factsPkgver
    readonly property string errorKey: pane.catalogBackend.factsError
    readonly property var errorArgs: pane.catalogBackend.factsErrorArgs

    clip: true
    contentWidth: width
    contentHeight: content.implicitHeight + Kirigami.Units.smallSpacing * 2
    boundsBehavior: Flickable.StopAtBounds
    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

    onPackageNameChanged: {
        pane.blockedNames = []
        pane.blockedPkgver = ""
        if (pane.packageName.length > 0 && pane.catalogBackend) {
            pane.catalogBackend.requestFacts(pane.packageName)
        }
        pane.refreshBlocked()
    }
    onRefreshTokenChanged: pane.refreshBlocked()

    Connections {
        target: pane.catalogBackend
        function onFactsRevisionChanged() {
            pane.refreshBlocked()
        }
    }

    Connections {
        target: pane.installedBackend
        // One revision per store mutation, including hold toggles that change
        // neither the installed nor the queue counter.
        function onStateRevisionChanged() {
            pane.refreshToken++
        }
    }

    // The one sentence for failed facts, chosen by the key the backend sent.
    function errorText(key, args) {
        const a = args.length > 0 ? args[0] : ""
        const b = args.length > 1 ? args[1] : ""
        const c = args.length > 2 ? args[2] : ""
        switch (key) {
        case "errorXbpsStart":
            return i18nd("kxbps", "Could not run %1: %2", a, b)
        case "errorXbpsTimeout":
            return i18nd("kxbps", "%1 did not answer within %2 seconds.", a, b)
        case "errorXbpsExit":
            return i18nd("kxbps", "%1 failed with exit code %2: %3", a, b, c)
        case "errorXbpsOutput":
            return i18nd("kxbps", "%1 printed a line this version does not understand (line %2): %3", a, b, c)
        case "errorXbpsEmpty":
            return i18nd("kxbps", "%1 printed nothing.", a)
        }
        return i18nd("kxbps", "Could not read the package: %1", a)
    }

    // A queued removal that would break the system: the plan lists the
    // dependents. Plan objects are C++-parented children of the backend, so the
    // reference has to be released as soon as the names are copied out.
    function refreshBlocked() {
        pane.blockedNames = []
        if (!pane.installedBackend || pane.packageName.length === 0 || !pane.installed) {
            return
        }
        if (pane.factsPkgver !== pane.packageName || pane.info.queuedAction !== "remove") {
            return
        }
        pane.readBlocked()
    }

    // Reads the plan for the current selection; the caller guarantees that the
    // selection is installed, its facts answered and it is queued for removal.
    function readBlocked() {
        if (pane.blockedPkgver === pane.packageName) {
            return
        }
        pane.blockedPkgver = pane.packageName
        const names = []
        const plan = pane.installedBackend.openRemove(pane.info.name, false)
        if (plan) {
            const blocked = plan.blocked
            for (let i = 0; i < blocked.length; ++i) {
                if (blocked[i].name !== pane.info.name) {
                    continue
                }
                const neededBy = blocked[i].neededBy
                for (let j = 0; j < neededBy.length; ++j) {
                    names.push(neededBy[j])
                }
                break
            }
            pane.installedBackend.release(plan)
        }
        pane.blockedNames = names
    }

    function queueLabel() {
        if (pane.info.queuedAction !== "") {
            return i18nd("kxbps", "Remove from queue")
        }
        if (pane.info.isUpdate) {
            return i18nd("kxbps", "Queue update")
        }
        if (pane.installed) {
            return i18nd("kxbps", "Queue removal")
        }
        return i18nd("kxbps", "Add to queue")
    }

    function queueToken() {
        if (pane.info.isUpdate) {
            return "update"
        }
        return pane.installed ? "remove" : "install"
    }

    ColumnLayout {
        id: content
        x: Kirigami.Units.largeSpacing
        y: Kirigami.Units.smallSpacing
        width: pane.width - Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.smallSpacing

        // Name and description first: the row already showed the name, but the
        // pane is what the user reads before deciding.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.name
            font.bold: true
            elide: Text.ElideRight
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: text.length > 0
            text: pane.info.short !== undefined ? pane.info.short : ""
            wrapMode: Text.WordWrap
            color: Kirigami.Theme.disabledTextColor
        }

        // A package xbps refused to describe: the sentence, not empty fields.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: pane.errorKey.length > 0
            text: visible ? pane.errorText(pane.errorKey, pane.errorArgs) : ""
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing

            QQC2.Button {
                text: pane.queueLabel()
                icon.name: pane.info.queuedAction !== "" ? "list-remove"
                      : (pane.info.isUpdate ? "view-refresh"
                                            : (pane.installed ? "list-remove" : "list-add"))
                onClicked: {
                    if (pane.info.queuedAction !== "") {
                        pane.installedBackend.removeQueued(pane.name)
                    } else {
                        pane.installedBackend.toggleQueue(pane.name, pane.queueToken())
                    }
                }
            }

            QQC2.Button {
                visible: pane.installed
                text: pane.info.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
                icon.name: pane.info.held ? "object-unlocked" : "object-locked"
                onClicked: pane.installedBackend.toggleHold(pane.name)
            }

            Item {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            height: 1
            color: Kirigami.Theme.disabledTextColor
        }

        GridLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            columns: 2
            columnSpacing: Kirigami.Units.largeSpacing
            rowSpacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                text: i18nd("kxbps", "Installed version")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.installed ? pane.info.installedVersion : i18nd("kxbps", "Not installed")
                font.family: "monospace"
                elide: Text.ElideRight
            }

            QQC2.Label {
                text: i18nd("kxbps", "Repository version")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.version
                font.family: "monospace"
                elide: Text.ElideRight
            }

            QQC2.Label {
                text: i18nd("kxbps", "Size")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.info.size !== undefined ? pane.info.size : ""
                font.family: "monospace"
            }

            QQC2.Label {
                text: i18nd("kxbps", "Download")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.info.download !== undefined ? pane.info.download : ""
                font.family: "monospace"
            }

            QQC2.Label {
                text: i18nd("kxbps", "License")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.info.license !== undefined ? pane.info.license : ""
                elide: Text.ElideRight
            }

            QQC2.Label {
                text: i18nd("kxbps", "Repository")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                // Raw wire token, not UI copy.
                text: pane.info.repo !== undefined ? pane.info.repo : ""
                font.family: "monospace"
            }

            QQC2.Label {
                text: i18nd("kxbps", "State")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                // A package that is not installed is neither explicit nor a
                // dependency; `explicit` is simply false for it.
                text: !pane.installed ? i18nd("kxbps", "Not installed")
                      : (pane.orphan ? i18nd("kxbps", "Orphan")
                                     : (pane.info.explicit ? i18nd("kxbps", "Explicit")
                                                           : i18nd("kxbps", "Dependency")))
                elide: Text.ElideRight
            }

            QQC2.Label {
                text: i18nd("kxbps", "Held")
                color: Kirigami.Theme.disabledTextColor
            }
            QQC2.Label {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: pane.info.held ? i18nd("kxbps", "Held") : i18nd("kxbps", "Not held")
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "Depends")
            font.bold: true
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.info.depends !== undefined && pane.info.depends.length > 0
                  ? pane.info.depends
                  : i18nd("kxbps", "No dependencies")
            elide: Text.ElideRight

            QQC2.ToolTip.visible: dependsHover.hovered && text.length > 0
            QQC2.ToolTip.text: text

            HoverHandler {
                id: dependsHover
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "Required by")
            font.bold: true
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.dependents.length > 0 ? pane.dependents.join(", ") : i18nd("kxbps", "Nothing depends on it")
            elide: Text.ElideRight

            QQC2.ToolTip.visible: requiredByHover.hovered && text.length > 0
            QQC2.ToolTip.text: text

            HoverHandler {
                id: requiredByHover
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: pane.info.homepage !== undefined && pane.info.homepage.length > 0
            text: pane.info.homepage !== undefined
                  ? "<a href=\"" + pane.info.homepage + "\">" + pane.info.homepage + "</a>"
                  : ""
            textFormat: Text.StyledText
            elide: Text.ElideRight
            color: Kirigami.Theme.linkColor
            onLinkActivated: (link) => Qt.openUrlExternally(link)

            HoverHandler {
                cursorShape: Qt.PointingHandCursor
            }
        }

        // The removal that the plan refuses: the names come from the core, not
        // from a guess about the dependency graph.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: pane.blockedNames.length > 0
            text: i18nd("kxbps", "Cannot be removed: %1 depends on it.", pane.blockedNames.join(", "))
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }
    }
}
