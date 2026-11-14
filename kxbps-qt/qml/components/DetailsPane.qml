// The details of the selected package, under the table.
//
// The page reads `catalogBackend.get(pkgver)` instead of the model row, because
// QML cannot index a `QAbstractItemModel` by row number. The facts come from
// xbps itself: the pane calls `requestFacts(pkgver)` when the selection moves,
// and `factsRevision` says when the answer can be read. Both the dependencies
// and the reverse dependencies are xbps' own lists (`-x` and `-X`), so nothing
// here scans another list to guess them.
//
// Three panels while the table is wide enough: what the package is, what it
// costs, and what it depends on. Below the wide form the panels stack into one
// column, which is the form the narrow drawer shows.
//
// The button queues and unqueues, like the checkbox in the table: the queue is
// the one place a transaction is decided, and Apply is the one place it starts.
// A queued package offers the opposite gesture, read back from the store.

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
    // The row form of the table above. `Wide` lays the three panels side by
    // side; every narrower form stacks them.
    property int rowLayout: 0

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
    readonly property bool orphan: pane.installed && !pane.info.explicit && pane.dependents.length === 0

    // The version, even before the facts arrive: the pkgver carries it.
    readonly property string version: pane.catalogBackend.versionOf(pane.packageName)
    readonly property string name: pane.info.name !== undefined ? pane.info.name : pane.packageLabel
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

    readonly property bool threeColumns: pane.rowLayout === 0

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
        }
    }

    // One fact row: a muted label and the value. The value is monospace where
    // it is a version, a size or a raw xbps token.
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
        spacing: Kirigami.Units.smallSpacing

        // The children address the fact by its id: inside a RowLayout the
        // parent is the layout itself, not the component's root.
        QQC2.Label {
            // A fixed share, not a fraction of the row: a binding on the
            // layout's own width from inside it re-triggers layout forever.
            Layout.preferredWidth: Kirigami.Units.gridUnit * 5
            Layout.minimumWidth: Kirigami.Units.gridUnit * 3
            Layout.maximumWidth: Kirigami.Units.gridUnit * 7
            text: fact.label
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            horizontalAlignment: Text.AlignRight
            text: fact.value
            font.family: fact.mono ? "monospace" : Kirigami.Theme.defaultFont.family
            elide: fact.elideMode
        }
    }

    // The panel heading: a small muted uppercase label, so three panels read as
    // three named groups rather than one wall of values.
    component PanelTitle: QQC2.Label {
        // The heading is a group name, not a sentence: uppercase so it reads as
        // a label above the values. `Font.AllUppercase` is not used, because a
        // translation that starts with a capital would stay mixed case.
        property string heading: ""
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        text: heading.toUpperCase()
        font.family: Kirigami.Theme.smallFont.family
        font.pixelSize: Kirigami.Theme.smallFont.pixelSize
        font.bold: true
        color: Kirigami.Theme.disabledTextColor
        elide: Text.ElideRight
    }

    // A list of xbps names: monospace, wrapped, and a sentence when empty, so a
    // blank field never reads as "not read".
    component NameList: QQC2.Label {
        Layout.fillWidth: true
        Layout.minimumWidth: 0
        wrapMode: Text.WordWrap
        font: Qt.font({
            family: "monospace",
            pixelSize: Kirigami.Theme.smallFont.pixelSize
        })
    }

    // A vertical hairline between panels. `Kirigami.Separator` is horizontal
    // only, so this is a themed rectangle sized in grid units.
    component VSeparator: Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: Kirigami.Units.smallSpacing / 2
        color: Kirigami.Theme.disabledTextColor
    }

    // Panel 1: what the package is, and the one action it offers.
    component InfoPanel: ColumnLayout {
        Layout.fillWidth: true
        Layout.minimumWidth: Kirigami.Units.gridUnit * 12
        Layout.alignment: Qt.AlignTop
        spacing: Kirigami.Units.smallSpacing

        PanelTitle {
            heading: i18nd("kxbps", "Package information")
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: Kirigami.Units.smallSpacing

            // One generic package icon, never a per-package picture: the
            // catalogue has none, and an invented one would be decoration.
            Kirigami.Icon {
                source: "package-x-generic"
                implicitWidth: Kirigami.Units.iconSizes.large
                implicitHeight: Kirigami.Units.iconSizes.large
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

                Fact {
                    label: i18nd("kxbps", "Version")
                    value: pane.installed && pane.version.length > 0
                           ? i18nd("kxbps", "%1 → %2", pane.info.installedVersion, pane.version)
                           : pane.version
                    mono: true
                }
            }
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

        // A licence is a phrase, not a value: it wraps under its label
        // rather than eliding to a fragment of itself. It belongs with
        // the package's identity, not with its sizes.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.info.license !== undefined && pane.info.license.length > 0
                  ? i18nd("kxbps", "License: %1", pane.info.license) : ""
            visible: text.length > 0
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }

        // A package xbps refused to describe: the sentence, not empty
        // fields.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            visible: pane.errorKey.length > 0
            text: visible ? pane.errorText(pane.errorKey, pane.errorArgs) : ""
            color: Kirigami.Theme.negativeTextColor
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.Button {
                Layout.fillWidth: true
                text: pane.actionLabel
                icon.name: pane.actionIcon
                font: Kirigami.Theme.smallFont
                enabled: pane.packageName.length > 0 && pane.factsReady
                onClicked: {
                    if (pane.queuedAction.length > 0) {
                        pane.installedBackend.removeQueued(pane.name)
                    } else {
                        pane.installedBackend.toggleQueue(pane.name, pane.actionToken)
                    }
                }
            }

            QQC2.Button {
                visible: pane.installed
                text: pane.info.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
                icon.name: pane.info.held ? "object-unlocked" : "object-locked"
                font: Kirigami.Theme.smallFont
                onClicked: pane.installedBackend.toggleHold(pane.name)
            }

            // The two questions the pane itself cannot show: which files the
            // package owns, and which package owns a path. Both need room the
            // pane does not have, so they open in one compact dialog.
            QQC2.ToolButton {
                id: moreButton
                text: i18nd("kxbps", "More")
                icon.name: "overflow-menu"
                font: Kirigami.Theme.smallFont
                enabled: pane.packageName.length > 0
                Accessible.name: i18nd("kxbps", "More package queries")
                onClicked: moreMenu.open()

                QQC2.Menu {
                    id: moreMenu
                    y: moreButton.height

                    QQC2.MenuItem {
                        text: i18nd("kxbps", "Package files…")
                        enabled: pane.packageName.length > 0
                        onTriggered: filesDialog.openFiles()
                    }

                    QQC2.MenuItem {
                        text: i18nd("kxbps", "File owner…")
                        onTriggered: filesDialog.openOwner()
                    }
                }
            }
        }
    }

    // Panel 2: what it costs. A zero byte size is an em dash, never a
    // fabricated "0 B".
    component SizePanel: ColumnLayout {
        Layout.fillWidth: false
        Layout.minimumWidth: Kirigami.Units.gridUnit * 11
        Layout.alignment: Qt.AlignTop
        spacing: Kirigami.Units.smallSpacing

        PanelTitle {
            heading: i18nd("kxbps", "Size statistics")
        }

        Fact {
            label: i18nd("kxbps", "Installed")
            value: pane.info.size !== undefined && pane.info.size !== "" ? pane.info.size : "—"
            mono: true
        }

        Fact {
            label: i18nd("kxbps", "Download")
            value: pane.info.download !== undefined && pane.info.download !== "" ? pane.info.download : "—"
            mono: true
        }
    }

    // Panel 3: the two xbps lists, on their own so a long dependency list
    // wraps inside the panel instead of pushing the others.
    component DepsPanel: ColumnLayout {
        Layout.fillWidth: true
        Layout.minimumWidth: Kirigami.Units.gridUnit * 14
        Layout.alignment: Qt.AlignTop
        spacing: Kirigami.Units.smallSpacing

        PanelTitle {
            heading: i18nd("kxbps", "Dependencies / required by")
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: i18nd("kxbps", "Depends")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        NameList {
            text: pane.info.depends !== undefined && pane.info.depends.length > 0
                  ? pane.info.depends : i18nd("kxbps", "No dependencies")
            color: pane.info.depends !== undefined && pane.info.depends.length > 0
                   ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "Required by")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
        }

        NameList {
            text: pane.dependents.length > 0 ? pane.dependents.join(", ")
                                             : i18nd("kxbps", "Nothing depends on it")
            color: pane.dependents.length > 0 ? Kirigami.Theme.textColor
                                              : Kirigami.Theme.disabledTextColor
        }
    }

    // The scroll body: the three panels are taller than the slot on a narrow
    // window, and a plain layout would draw over the log strip instead of
    // scrolling. The scrollbar takes its own strip, or it covers the right-aligned
    // values.
    Flickable {
        anchors.fill: parent
        clip: true
        contentWidth: width - paneBar.width
        contentHeight: content.implicitHeight + Kirigami.Units.largeSpacing * 2
        boundsBehavior: Flickable.StopAtBounds
        QQC2.ScrollBar.vertical: QQC2.ScrollBar {
            id: paneBar
            policy: QQC2.ScrollBar.AsNeeded
        }

        ColumnLayout {
            id: content
            x: Kirigami.Units.largeSpacing
            y: Kirigami.Units.largeSpacing
            width: pane.width - Kirigami.Units.largeSpacing * 2 - paneBar.width
            spacing: Kirigami.Units.largeSpacing

        // Three panels on a wide pane, one column on a narrow one. The panels
        // are inline components, so both forms show the same content instead
        // of a second copy that could drift.
        RowLayout {
            visible: pane.threeColumns
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            // The hairlines are dividers, not borders: a gap on both sides
            // keeps every heading and every right-aligned value off the line.
            spacing: Kirigami.Units.largeSpacing

            InfoPanel {
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 22
            }

            VSeparator {}

            SizePanel {
                Layout.fillWidth: false
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
            }

            VSeparator {}

            DepsPanel {
                Layout.fillWidth: true
                Layout.preferredWidth: Kirigami.Units.gridUnit * 26
            }
        }

        ColumnLayout {
            visible: !pane.threeColumns
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            spacing: Kirigami.Units.largeSpacing

            InfoPanel {}

            SizePanel {}

            DepsPanel {}
        }
        }
    }

    // Nothing selected, or the facts still on their way: say so rather than
    // showing a grid of blanks.
    QQC2.Label {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
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
