// Everything xbps reports about one package, with the one action the user came
// for: install it, update it or remove it.
//
// The page reads `catalogBackend.get(pkgver)` instead of the model row, because
// QML cannot index a `QAbstractItemModel` by row number. The facts come from
// xbps itself: the owner calls `requestFacts(pkgver)` when the selection moves,
// and `factsRevision` says when the answer can be read. Both the dependencies
// and the reverse dependencies are xbps' own lists (`-x` and `-X`), so nothing
// here scans another list to guess them.
//
// The action is direct, not a queue toggle: it emits `planRequested` and the
// window opens the dry-run plan, whose Apply runs through pkexec. The queue is
// still there for the menu bar's batch gestures, but a package page is not a
// place to put something in a list.
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
    // The name without the version, for the title before the facts arrive: a
    // `pkgver` cannot be split on "-", because names like `lib32-glibc` have
    // hyphens of their own.
    property string packageLabel: ""

    // Bumped when the store changes, so `info` re-reads the snapshot: the
    // backend hands out a copy and has no notify signal for `get()`.
    property int refreshToken: 0

    // The page asks for a transaction; the window owns the plan dialog and the
    // job, so this file never calls `pkexec` itself. `token` is `install`,
    // `update` or `remove`.
    signal planRequested(string name, string token)

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
                                  : pane.packageLabel
    // The facts failed to load: say so rather than showing blanks.
    readonly property string errorKey: pane.catalogBackend.factsError
    readonly property var errorArgs: pane.catalogBackend.factsErrorArgs
    // The half of the read that mirrors `packageName`, so a check that needs
    // `info` waits for the facts instead of reading a map for another package.
    readonly property string factsPkgver: pane.catalogBackend.factsPkgver

    // What the one button does. An update outranks "installed", because the
    // reason the user opened the page is usually the new version.
    readonly property string actionToken: pane.info.isUpdate ? "update"
                                        : (pane.installed ? "remove" : "install")
    readonly property string actionLabel: pane.info.isUpdate ? i18nd("kxbps", "Update")
                                         : (pane.installed ? i18nd("kxbps", "Remove")
                                                           : i18nd("kxbps", "Install"))
    readonly property string actionIcon: pane.info.isUpdate ? "system-software-update"
                                        : (pane.installed ? "edit-delete"
                                                          : "document-save")

    clip: true
    contentWidth: width
    contentHeight: pane.packageName.length > 0
                   ? content.implicitHeight + Kirigami.Units.largeSpacing * 2 : 0
    boundsBehavior: Flickable.StopAtBounds
    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

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

    // One fact row: a muted label and the value. The value is monospace where
    // it is a version, a size or a raw xbps token, so columns line up.
    component Fact: RowLayout {
        property string label: ""
        property string value: ""
        property bool mono: false
        // A version reads from the right (`1.2.3_1`), a URL from both ends:
        // the host alone does not say which repository it is.
        property int elideMode: Text.ElideLeft

        Layout.fillWidth: true
        Layout.minimumWidth: 0
        spacing: Kirigami.Units.largeSpacing

        QQC2.Label {
            Layout.preferredWidth: Kirigami.Units.gridUnit * 8.5
            text: parent.label
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            horizontalAlignment: Text.AlignRight
            text: parent.value
            font.family: parent.mono ? "monospace" : Kirigami.Theme.defaultFont.family
            elide: parent.elideMode
        }
    }

    // Nothing selected: an empty fact grid with a dead button reads like a bug,
    // so the pane says what it is waiting for. `y` rather than `anchors.centerIn`
    // because a Flickable positions its children in content coordinates, and the
    // content has no height while the placeholder is the only child.
    QQC2.Label {
        y: (pane.height - implicitHeight) / 2
        width: pane.width - Kirigami.Units.largeSpacing * 2
        x: Kirigami.Units.largeSpacing
        visible: pane.packageName.length === 0
        text: i18nd("kxbps", "Select a package to see its details.")
        color: Kirigami.Theme.disabledTextColor
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
    }

    ColumnLayout {
        id: content
        x: Kirigami.Units.largeSpacing
        y: Kirigami.Units.largeSpacing
        width: pane.width - Kirigami.Units.largeSpacing * 2
        spacing: Kirigami.Units.smallSpacing
        visible: pane.packageName.length > 0

        // The title is the package name, at heading size: the row already
        // showed it small, and this is the page the user is reading.
        Kirigami.Heading {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            level: 2
            text: pane.name
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

        // The one action, full width: it is what the page is for. Hold stays
        // beside it as the smaller, secondary choice.
        RowLayout {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.Button {
                Layout.fillWidth: true
                text: pane.actionLabel
                icon.name: pane.actionIcon
                enabled: pane.packageName.length > 0 && pane.factsPkgver === pane.packageName
                onClicked: pane.planRequested(pane.name, pane.actionToken)
            }

            QQC2.Button {
                visible: pane.installed
                text: pane.info.held ? i18nd("kxbps", "Unhold") : i18nd("kxbps", "Hold")
                icon.name: pane.info.held ? "object-unlocked" : "object-locked"
                onClicked: pane.installedBackend.toggleHold(pane.name)
            }
        }

        Kirigami.Separator {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
        }

        Fact {
            label: i18nd("kxbps", "Installed version")
            value: pane.installed ? pane.info.installedVersion : i18nd("kxbps", "Not installed")
            mono: true
        }
        Fact {
            label: i18nd("kxbps", "Available version")
            value: pane.version
            mono: true
        }
        Fact {
            label: i18nd("kxbps", "Size")
            value: pane.info.size !== undefined ? pane.info.size : ""
            mono: true
        }
        Fact {
            label: i18nd("kxbps", "Download")
            value: pane.info.download !== undefined ? pane.info.download : ""
            mono: true
        }
        Fact {
            label: i18nd("kxbps", "License")
            value: pane.info.license !== undefined ? pane.info.license : ""
        }
        Fact {
            label: i18nd("kxbps", "Repository")
            value: pane.info.repo !== undefined ? pane.info.repo : ""
            mono: true
            elideMode: Text.ElideMiddle
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

        Kirigami.Heading {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            level: 4
            text: i18nd("kxbps", "Depends")
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.info.depends !== undefined && pane.info.depends.length > 0
                  ? pane.info.depends
                  : i18nd("kxbps", "No dependencies")
            wrapMode: Text.WordWrap
            font.family: "monospace"
            color: pane.info.depends !== undefined && pane.info.depends.length > 0
                   ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
        }

        Kirigami.Heading {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            Layout.topMargin: Kirigami.Units.smallSpacing
            level: 4
            text: i18nd("kxbps", "Required by")
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: pane.dependents.length > 0 ? pane.dependents.join(", ") : i18nd("kxbps", "Nothing depends on it")
            wrapMode: Text.WordWrap
            font.family: "monospace"
            color: pane.dependents.length > 0 ? Kirigami.Theme.textColor
                                              : Kirigami.Theme.disabledTextColor
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
    }
}
