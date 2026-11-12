// Everything the catalog knows about one package, with the two actions that
// make sense from a list: queue a change and hold the package.
//
// The pane reads `catalogBackend.get(name)` instead of the model row, because
// QML cannot index a `QAbstractItemModel` by row number. Reverse dependencies
// are not exposed by any backend, so they are computed here from the catalog
// listing the window read once at startup (115 entries: a linear scan).
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
    property var catalogIndex: []
    property string packageName: ""

    // Bumped when the store changes, so `info` re-reads the snapshot: the
    // backend hands out a copy and has no notify signal for `get()`.
    property int refreshToken: 0
    property var blockedNames: []

    readonly property var info: {
        pane.refreshToken
        return pane.packageName.length > 0 ? pane.catalogBackend.get(pane.packageName) : ({})
    }
    readonly property bool installed: pane.info.installedVersion !== undefined && pane.info.installedVersion.length > 0
    readonly property var dependents: pane.packageName.length > 0 ? pane.requiredBy(pane.packageName) : []
    readonly property bool orphan: pane.installed && !pane.info.explicit && pane.dependents.length === 0

    clip: true
    contentWidth: width
    contentHeight: content.implicitHeight + Kirigami.Units.smallSpacing * 2
    boundsBehavior: Flickable.StopAtBounds
    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

    onPackageNameChanged: pane.refreshBlocked()
    onRefreshTokenChanged: pane.refreshBlocked()

    Connections {
        target: pane.installedBackend
        // One revision per store mutation, including hold toggles that change
        // neither the installed nor the queue counter.
        function onStateRevisionChanged() {
            pane.refreshToken++
        }
    }

    function dependsOf(pkg) {
        return pkg.depends.length > 0 ? pkg.depends.split(", ") : []
    }

    // Installed packages whose `depends` names this one.
    function requiredBy(name) {
        const out = []
        for (let i = 0; i < pane.catalogIndex.length; ++i) {
            const pkg = pane.catalogIndex[i]
            if (pkg.installedVersion === "") {
                continue
            }
            if (pane.dependsOf(pkg).indexOf(name) >= 0) {
                out.push(pkg.name)
            }
        }
        return out
    }

    // A queued removal that would break the system: the plan lists the
    // dependents. Plan objects are C++-parented children of the backend, so the
    // reference has to be released as soon as the names are copied out.
    function refreshBlocked() {
        pane.blockedNames = []
        if (!pane.installedBackend || pane.packageName.length === 0) {
            return
        }
        // A fresh read, not `info`: a handler runs before the `info` binding is
        // re-evaluated, so reading it here would see the previous queue state.
        const info = pane.catalogBackend.get(pane.packageName)
        if (info.queuedAction !== "remove") {
            return
        }
        const plan = pane.installedBackend.openRemove(pane.packageName, false)
        if (!plan) {
            return
        }
        const blocked = plan.blocked
        for (let i = 0; i < blocked.length; ++i) {
            if (blocked[i].name !== pane.packageName) {
                continue
            }
            const names = []
            const neededBy = blocked[i].neededBy
            for (let j = 0; j < neededBy.length; ++j) {
                names.push(neededBy[j])
            }
            pane.blockedNames = names
            break
        }
        pane.installedBackend.release(plan)
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
            text: pane.info.name !== undefined ? pane.info.name : ""
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
                        pane.installedBackend.removeQueued(pane.packageName)
                    } else {
                        pane.installedBackend.toggleQueue(pane.packageName, pane.queueToken())
                    }
                }
            }

            QQC2.Button {
                visible: pane.installed
                text: pane.info.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
                icon.name: pane.info.held ? "object-unlocked" : "object-locked"
                onClicked: pane.installedBackend.toggleHold(pane.packageName)
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
                text: pane.info.version !== undefined ? pane.info.version : ""
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

        // A Label rather than `Kirigami.LinkButton`: the longest homepage in
        // the catalog is 75 characters, and a button clips its text instead of
        // eliding it.
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
