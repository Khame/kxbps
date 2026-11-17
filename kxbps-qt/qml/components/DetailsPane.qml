// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// The details of the selected package, under the table.
//
// The page reads `catalogBackend.get(pkgver)` instead of the model row, because
// QML cannot index a `QAbstractItemModel` by row number. The facts come from
// xbps itself: the pane calls `requestFacts(pkgver)` when the selection moves,
// and `factsRevision` says when the answer can be read. Both the dependencies
// and the reverse dependencies are xbps' own lists (`-x` and `-X`), so nothing
// here scans another list to guess them.
//
// The pane is a header, a body of two columns and one row of actions. The
// header names the package and says what it is; the left column is a fact grid
// where every row has the same shape — a muted label, then the value, both
// left-aligned — and the right column holds the two xbps lists, one name per
// token so a version constraint stays with its package. The action row is the
// one place to act: the queue action on the left, where a destructive action
// belongs, and the two look-up buttons on the right.
//
// Below the wide form the columns stack, which is the form the narrow drawer
// shows.
//
// The queue button queues and unqueues, like the checkbox in the table: the
// queue is the one place a transaction is decided, and Apply is the one place
// it starts. A queued package offers the opposite gesture, read back from the
// store.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Item {
    id: pane

    property var catalogBackend
    property var installedBackend
    // The selected package as `name-version_revision`; empty closes the pane.
    property string packageName: ""
    // The name without the version, for the title before the facts arrive: a
    // `pkgver` cannot be split on "-", because names like `lib32-glibc` have
    // hyphens of their own.
    property string packageLabel: ""
    // The row form of the table above. `Wide` lays the body out in two columns;
    // every narrower form stacks them.
    property int rowLayout: 0

    // The job the hold toggle started, so the window can show its prompt, log
    // and result like any other privileged action.
    signal holdStarted(var started)

    // Bumped when the store changes, so `info` re-reads the snapshot: the
    // backend hands out a copy and has no notify signal for `get()`.
    property int refreshToken: 0

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
    // xbps hands the dependencies over as one comma-separated string. Split into
    // tokens so each name keeps its own version constraint next to it and the
    // list can wrap in columns instead of running as a paragraph.
    readonly property var dependsList: pane.info.depends !== undefined && pane.info.depends.length > 0
                                       ? pane.info.depends.split(", ") : []
    readonly property bool orphan: pane.installed && !pane.info.explicit && pane.dependents.length === 0

    // The version, even before the facts arrive: the pkgver carries it.
    readonly property string version: pane.catalogBackend.versionOf(pane.packageName)
    readonly property string name: pane.info.name !== undefined ? pane.info.name : pane.packageLabel
    // The one-line description xbps keeps for the package. Empty for a package
    // xbps refused to describe, and the header then shows only the name.
    readonly property string summary: pane.info.short !== undefined ? pane.info.short : ""
    // The facts failed to load: say so rather than showing blanks.
    readonly property string errorKey: pane.catalogBackend.factsError
    readonly property var errorArgs: pane.catalogBackend.factsErrorArgs
    // The half of the read that mirrors `packageName`, so a check that needs
    // `info` waits for the facts instead of reading a map for another package.
    readonly property string factsPkgver: pane.catalogBackend.factsPkgver
    readonly property bool factsReady: pane.factsPkgver === pane.packageName

    // The action the queue already holds for this package, empty when it is
    // not queued. `refreshToken` is the dependency: the store has no per-package
    // signal, and every mutation bumps the revision.
    readonly property string queuedAction: {
        pane.refreshToken
        return pane.installedBackend.queuedAction(pane.name)
    }

    // What the one button does. An update outranks "installed", because the
    // reason the user opened the page is usually the new version.
    readonly property string actionToken: pane.info.isUpdate ? "update"
                                        : (pane.installed ? "remove" : "install")
    readonly property string actionLabel: pane.queuedAction.length > 0
                                          ? i18nd("kxbps", "Remove from the queue")
                                          : (pane.info.isUpdate ? i18nd("kxbps", "Queue the update")
                                             : (pane.installed ? i18nd("kxbps", "Queue removal")
                                                               : i18nd("kxbps", "Queue installation")))
    readonly property string actionIcon: pane.queuedAction.length > 0 ? "edit-clear"
                                        : (pane.info.isUpdate ? "system-software-update"
                                           : (pane.installed ? "edit-delete" : "document-save"))

    readonly property bool wideLayout: pane.rowLayout === 0

    // The pane asks for the height its content needs, so no fact is cut in
    // half and the table above is not pushed down by a fixed guess. A very long
    // dependency list is capped and scrolls instead.
    readonly property real wantedHeight: headerRow.implicitHeight
                                        + body.implicitHeight
                                        + actions.implicitHeight
                                        + Kirigami.Units.largeSpacing * 4
                                        + Kirigami.Units.smallSpacing * 2
    implicitHeight: Math.max(Kirigami.Units.gridUnit * 6,
                             Math.min(wantedHeight, Kirigami.Units.gridUnit * 18))

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
            // xbps exits non-zero with an empty stderr when it has nothing to
            // say — a package the index does not know, for one — and a sentence
            // ending in a dangling colon would read as a missing detail.
            return c.length > 0
                   ? i18nd("kxbps", "%1 failed with exit code %2: %3", a, b, c)
                   : i18nd("kxbps", "%1 failed with exit code %2.", a, b)
        case "errorXbpsOutput":
            return i18nd("kxbps", "%1 printed a line this version does not understand (line %2): %3", a, b, c)
        case "errorXbpsEmpty":
            return i18nd("kxbps", "%1 printed nothing.", a)
        }
        return i18nd("kxbps", "Could not read the package: %1", a)
    }

    onPackageNameChanged: {
        if (pane.packageName.length > 0 && pane.catalogBackend) {
            pane.catalogBackend.requestFacts(pane.packageName)
        }
    }

    Connections {
        target: pane.installedBackend
        // One revision per store mutation, including hold toggles that change
        // neither the installed nor the queue counter.
        function onStateRevisionChanged() {
            pane.refreshToken++
            // The backend drops its cached facts when the installed records
            // move — a hold changes one — so ask again for the package on
            // screen; otherwise the pane keeps the state from before the click.
            if (pane.packageName.length > 0 && pane.catalogBackend) {
                pane.catalogBackend.requestFacts(pane.packageName)
            }
        }
    }

    // One fact row: a muted label in a fixed column, then the value. Both are
    // left-aligned, so every row starts at the same two places and the eye can
    // run down the labels without crossing a ragged right edge.
    component Fact: RowLayout {
        id: fact

        // No inline initialisers: a `property string value: ""` would replace
        // the binding the instance passes in, and every fact would render blank.
        property string label
        property string value
        property bool mono: false
        // A version reads from the right (`1.2.3_1`), a URL from both ends:
        // the host alone does not say which repository it is.
        property int elideMode: Text.ElideLeft

        Layout.fillWidth: true
        Layout.minimumWidth: 0
        spacing: Kirigami.Units.largeSpacing

        // The children address the fact by its id: inside a RowLayout the
        // parent is the layout itself, not the component's root.
        QQC2.Label {
            // A fixed share, not a fraction of the row: a binding on the
            // layout's own width from inside it re-triggers layout forever.
            Layout.preferredWidth: Kirigami.Units.gridUnit * 6
            text: fact.label
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: fact.value
            font.family: fact.mono ? "monospace" : Kirigami.Theme.defaultFont.family
            elide: fact.elideMode
        }
    }

    // A list of xbps names: monospace, one token per name, two aligned columns
    // so the names line up with each other the way the facts on the left do.
    // A positioner derives its own implicit size from the placed tokens, which
    // is what the scroll body reads.
    component NameGrid: Grid {
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        columns: 2
        rowSpacing: Kirigami.Units.smallSpacing
        columnSpacing: Kirigami.Units.largeSpacing * 2
    }

    // The monospace label a name in the flow is written with, kept in one place
    // so both lists read the same.
    component NameToken: QQC2.Label {
        font: Qt.font({
            family: "monospace",
            pixelSize: Kirigami.Theme.smallFont.pixelSize
        })
    }

    // A vertical hairline between the two columns. `Kirigami.Separator` is
    // horizontal only, so this is a themed rectangle sized in grid units.
    component VSeparator: Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: Kirigami.Units.smallSpacing / 2
        color: Kirigami.Theme.disabledTextColor
    }

    // The fact grid: one row per fact, the same shape every time.
    component Facts: ColumnLayout {
        Layout.minimumWidth: 0
        Layout.alignment: Qt.AlignTop
        spacing: Kirigami.Units.smallSpacing

        Fact {
            label: i18nd("kxbps", "Version")
            value: pane.installed && pane.version.length > 0
                   ? i18nd("kxbps", "%1 → %2", pane.info.installedVersion, pane.version)
                   : pane.version
            mono: true
        }

        Fact {
            label: i18nd("kxbps", "State")
            // A package that is not installed is neither explicit nor a
            // dependency; `explicit` is simply false for it.
            value: !pane.installed ? i18nd("kxbps", "Not installed")
                 : (pane.orphan ? i18nd("kxbps", "Orphan")
                                : (pane.info.explicit ? i18nd("kxbps", "Explicit")
                                                      : i18nd("kxbps", "Dependency")))
        }

        Fact {
            label: i18nd("kxbps", "Held")
            value: pane.info.held ? i18nd("kxbps", "Held") : i18nd("kxbps", "Not held")
        }

        Fact {
            label: i18nd("kxbps", "Repository")
            value: pane.info.repo !== undefined ? pane.info.repo : ""
            mono: true
            elideMode: Text.ElideMiddle
        }

        Fact {
            label: i18nd("kxbps", "License")
            value: pane.info.license !== undefined ? pane.info.license : ""
        }

        Fact {
            label: i18nd("kxbps", "Size")
            // The download comes first: it is the number a reader needs before
            // deciding, and the installed size is what the package takes on
            // disk afterwards.
            value: (pane.info.download !== undefined && pane.info.download !== ""
                    ? pane.info.download : "—")
                   + " → "
                   + (pane.info.size !== undefined && pane.info.size !== ""
                      ? pane.info.size : "—")
            mono: true
        }

        // A package xbps refused to describe: the sentence, not empty fields.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: pane.errorKey.length > 0
            text: visible ? pane.errorText(pane.errorKey, pane.errorArgs) : ""
            color: Kirigami.Theme.negativeTextColor
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }
    }

    // The two xbps lists, on their own so a long dependency list wraps inside
    // the column instead of pushing the facts.
    component Deps: ColumnLayout {
        Layout.minimumWidth: 0
        Layout.alignment: Qt.AlignTop
        spacing: Kirigami.Units.smallSpacing

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: i18nd("kxbps", "Depends")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        NameGrid {
            visible: pane.dependsList.length > 0
            Repeater {
                model: pane.dependsList
                NameToken {
                    text: modelData
                }
            }
        }

        // An empty field reads as "not read"; say it instead.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: pane.dependsList.length === 0
            text: i18nd("kxbps", "No dependencies")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "Required by")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        NameGrid {
            visible: pane.dependents.length > 0
            Repeater {
                model: pane.dependents
                NameToken {
                    text: modelData
                }
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: pane.dependents.length === 0
            text: i18nd("kxbps", "Nothing depends on it")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }
    }

    // The header: what the package is, at a glance.
    RowLayout {
        id: headerRow
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        anchors.topMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        // One generic package icon, never a per-package picture: the catalogue
        // has none, and an invented one would be decoration.
        Kirigami.Icon {
            source: "package-x-generic"
            implicitWidth: Kirigami.Units.iconSizes.medium
            implicitHeight: Kirigami.Units.iconSizes.medium
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: 0

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
                text: pane.summary
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }
    }

    Kirigami.Separator {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: headerRow.bottom
        anchors.topMargin: Kirigami.Units.smallSpacing
    }

    // The scroll body: the facts and the lists are taller than the slot on a
    // narrow window, and a plain layout would draw over the action row instead
    // of scrolling. The scrollbar takes its own strip, or it covers the values.
    Flickable {
        id: bodyFlick
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: headerRow.bottom
        anchors.topMargin: Kirigami.Units.smallSpacing * 3
        anchors.bottom: actions.top
        anchors.bottomMargin: Kirigami.Units.smallSpacing
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        clip: true
        contentWidth: width - paneBar.width
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            id: paneBar
            policy: QQC2.ScrollBar.AsNeeded
        }

        ColumnLayout {
            id: body
            width: bodyFlick.contentWidth
            spacing: Kirigami.Units.largeSpacing

            // Two columns on a wide pane, one on a narrow one. The blocks are
            // inline components, so both forms show the same content instead
            // of a second copy that could drift.
            RowLayout {
                visible: pane.wideLayout
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: Kirigami.Units.largeSpacing

                Facts {
                    Layout.fillWidth: true
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 26
                }

                VSeparator {}

                Deps {
                    Layout.fillWidth: true
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 32
                }
            }

            ColumnLayout {
                visible: !pane.wideLayout
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: Kirigami.Units.largeSpacing

                Facts {}

                Deps {}
            }
        }
    }

    // The actions, pinned so a long dependency list scrolls under them instead
    // of carrying them away. The queue action sits on the left, apart from the
    // two buttons that only look something up: it is the one that changes what
    // Apply will do, and it is the one that removes.
    RowLayout {
        id: actions
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        DenseButton {
            text: pane.actionLabel
            icon.name: pane.actionIcon
            enabled: pane.packageName.length > 0 && pane.factsReady
            onClicked: {
                if (pane.queuedAction.length > 0) {
                    pane.installedBackend.removeQueued(pane.name)
                } else {
                    pane.installedBackend.toggleQueue(pane.name, pane.actionToken)
                }
            }
        }

        Item {
            Layout.fillWidth: true
        }

        DenseButton {
            text: i18nd("kxbps", "Files")
            enabled: pane.packageName.length > 0
            Accessible.name: i18nd("kxbps", "Files of the package")
            onClicked: filesDialog.open()
        }

        DenseButton {
            visible: pane.installed
            text: pane.info.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
            icon.name: pane.info.held ? "object-unlocked" : "object-locked"
            onClicked: pane.holdStarted(pane.installedBackend.toggleHold(pane.name))
        }
    }

    // Nothing selected: say so rather than showing a grid of blanks.
    QQC2.Label {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: headerRow.bottom
        anchors.topMargin: Kirigami.Units.largeSpacing
        width: parent.width - Kirigami.Units.largeSpacing * 2 - paneBar.width
        visible: pane.packageName.length === 0
        text: i18nd("kxbps", "Select a package to see its details.")
        color: Kirigami.Theme.disabledTextColor
        font: Kirigami.Theme.smallFont
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
    }

    PackageFilesDialog {
        id: filesDialog
        catalogBackend: pane.catalogBackend
        packageName: pane.packageName
        packageLabel: pane.name
    }
}
