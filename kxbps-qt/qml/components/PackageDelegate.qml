// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// One package row of the installed-set table.
//
// A table row, not a card: the checkbox that marks the package for the queue,
// the name, the description, and the numeric columns the tool is read for.
// Nothing here starts a transaction — the checkbox only moves the package in or
// out of the queue, and the toolbar's `Apply` is what opens the dry run, so a
// stray click costs a marked row, never a package.
//
// The column widths come from the view, which draws the header from the same
// values, so no label can push the next column out of line. `layout` picks the
// form: `Wide` is every column, `Medium` folds the description under the name
// and drops the download, `Narrow` drops the installed version and the sizes.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import "../Tint.js" as Tint
import org.kde.ki18n 1.0

QQC2.ItemDelegate {
    id: delegate

    enum Layout {
        Wide,
        Medium,
        Narrow
    }

    property var installedBackend
    // The row form, as the view decided for the width it was given.
    property int layout: PackageDelegate.Wide
    // Column widths shared with the header.
    property real checkWidth
    property real gutter: Kirigami.Units.gridUnit * 0.75
    property real nameWidth
    property real descriptionWidth
    property real installedWidth
    property real availableColumnWidth
    property bool showAvailable: true
    property real sizeWidth
    property real downloadWidth
    // Driven by the selected package, not `ListView.currentIndex`: a filter
    // change resets the model and would drop the current index while the
    // details still describe the same package.
    property bool selected: false

    // The row's own identity, as real properties. `model` is a context property
    // injected into this file's bindings only: code outside the delegate reads
    // `item.model` as undefined, so a click handler must not reach for it.
    readonly property string packageName: model.name === undefined ? "" : model.name
    readonly property string packageVersion: model.version === undefined ? "" : model.version
    readonly property string packageInstalledVersion: model.installedVersion === undefined ? ""
                                                                                            : model.installedVersion
    readonly property string packageQueuedAction: model.queuedAction === undefined ? ""
                                                                                  : model.queuedAction
    // The row's position, likewise a context property inside this file only.
    readonly property int packageIndex: index === undefined ? -1 : index
    readonly property bool packageHeld: model.held === true
    readonly property bool packageIsUpdate: model.isUpdate === true
    readonly property string packagePkgver: {
        const version = delegate.packageInstalledVersion.length > 0
                        ? delegate.packageInstalledVersion
                        : delegate.packageVersion
        return delegate.packageName.length > 0 ? delegate.packageName + "-" + version : ""
    }

    readonly property bool wideRow: delegate.layout === PackageDelegate.Wide
    readonly property bool narrowRow: delegate.layout === PackageDelegate.Narrow
    readonly property bool installed: model.installedVersion !== ""
    readonly property bool isUpdate: model.isUpdate === true
    readonly property bool queued: model.queuedAction !== ""
    // The row's own action: an update outranks "installed", which would pin the
    // old version; installed with no update is a removal; not installed is an
    // install.
    readonly property string actionToken: delegate.isUpdate ? "update"
                                          : (delegate.installed ? "remove" : "install")
    readonly property bool canQueue: delegate.actionToken.length > 0
    // The action the queue will run for this row, in the row's own words: a
    // bare "Marked" never said whether Apply would install or remove.
    readonly property string queuedActionText: {
        switch (delegate.packageQueuedAction) {
        case "install":
            return i18nd("kxbps", "To install")
        case "update":
            return i18nd("kxbps", "To update")
        case "remove":
            return i18nd("kxbps", "To remove")
        }
        return ""
    }
    // A mixed list — the repository listing or a search — says which rows the
    // machine already holds. On a filter whose every row is installed the
    // column says it, and a pill on all of them would be noise.
    readonly property bool showInstalledState: {
        if (!delegate.installed || delegate.isUpdate || delegate.installedBackend === null
                || delegate.installedBackend === undefined) {
            return false
        }
        const packages = delegate.installedBackend.packages
        return packages.filter === "all" || packages.searching
    }
    // An em dash for a value xbps did not report; never a fabricated zero.
    readonly property string dash: "—"
    readonly property string installedText: delegate.installed ? model.installedVersion : delegate.dash
    readonly property string availableText: {
        if (delegate.isUpdate) {
            return model.updateVersion
        }
        if (delegate.installed) {
            return model.installedVersion
        }
        return model.version !== undefined && model.version !== "" ? model.version : delegate.dash
    }
    readonly property string sizeText: {
        const value = delegate.isUpdate ? model.updateSize : model.size
        return value !== undefined && value !== "" ? value : delegate.dash
    }
    readonly property string downloadText: model.download !== undefined && model.download !== ""
                                           ? model.download : delegate.dash

    implicitHeight: Kirigami.Units.gridUnit * (delegate.wideRow ? 2 : 2.6)
    height: implicitHeight
    leftPadding: Kirigami.Units.mediumSpacing
    rightPadding: Kirigami.Units.mediumSpacing
    topPadding: 0
    bottomPadding: 0
    // The selection wash is drawn by this row's own background, so the style's
    // highlight fill never paints a solid block over it.
    highlighted: false

    Accessible.role: Accessible.ListItem
    Accessible.name: model.name
    Accessible.description: model.short
    Accessible.selected: delegate.selected

    background: Rectangle {
        color: delegate.selected
               ? Tint.wash(Kirigami.Theme.highlightColor, Tint.selection)
               : (delegate.hovered
                  ? Tint.wash(Kirigami.Theme.highlightColor, Tint.hover)
                  : "transparent")

        // The left accent of the selected row: the theme colour at a width the
        // unit scale defines, never a measured pixel line.
        Rectangle {
            visible: delegate.selected
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: Kirigami.Units.smallSpacing / 2
            color: Kirigami.Theme.highlightColor
        }

        // The hairline between rows, from the theme so it follows light and
        // dark without a colour of its own.
        Kirigami.Separator {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
        }
    }

    contentItem: RowLayout {
        spacing: delegate.gutter

        Item {
            Layout.preferredWidth: delegate.checkWidth
            Layout.minimumWidth: delegate.checkWidth
            Layout.maximumWidth: delegate.checkWidth
            Layout.fillHeight: true

            QQC2.CheckBox {
                anchors.centerIn: parent
                checked: delegate.queued
                // Nothing to queue for a package the machine already holds, so
                // the control is disabled rather than queueing a transaction
                // xbps would refuse to act on.
                enabled: delegate.queued || delegate.canQueue
                // The checkbox is the row's action, so its accessible name says
                // what marking does, not just "select".
                Accessible.name: {
                    if (delegate.queued) {
                        return i18nd("kxbps", "Unmark %1", model.name)
                    }
                    if (delegate.actionToken === "remove") {
                        return i18nd("kxbps", "Mark %1 for removal", model.name)
                    }
                    return delegate.isUpdate
                            ? i18nd("kxbps", "Mark %1 for the update", model.name)
                            : i18nd("kxbps", "Mark %1 for installation", model.name)
                }
                onToggled: {
                    if (delegate.installedBackend === null || delegate.installedBackend === undefined) {
                        return
                    }
                    if (delegate.queued) {
                        delegate.installedBackend.removeQueued(model.name)
                    } else {
                        delegate.installedBackend.toggleQueue(model.name, delegate.actionToken)
                    }
                }
            }
        }

        // Name, and the description's second line in the two narrower forms.
        ColumnLayout {
            Layout.preferredWidth: delegate.nameWidth
            Layout.minimumWidth: delegate.nameWidth
            Layout.maximumWidth: delegate.nameWidth
            Layout.fillHeight: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    text: model.name
                    font.bold: true
                    rightPadding: Kirigami.Units.smallSpacing
                    elide: Text.ElideRight
                }

                // The states that change what the package does. "Explicit" is
                // the normal case and would be a marker on every row.
                QQC2.Label {
                    visible: model.held
                    text: i18nd("kxbps", "Held")
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.neutralTextColor
                }

                QQC2.Label {
                    visible: delegate.showInstalledState
                    text: i18nd("kxbps", "Installed")
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.disabledTextColor
                }

                QQC2.Label {
                    visible: delegate.queued
                    text: delegate.queuedActionText
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.highlightColor
                }
            }

            QQC2.Label {
                visible: !delegate.wideRow
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                text: model.short
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
        }

        // The description column exists only in the wide form; the narrower
        // forms put it under the name, in the column above.
        QQC2.Label {
            visible: delegate.wideRow
            Layout.preferredWidth: delegate.descriptionWidth
            Layout.minimumWidth: delegate.descriptionWidth
            Layout.maximumWidth: delegate.descriptionWidth
            text: model.short
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            rightPadding: Kirigami.Units.smallSpacing
            elide: Text.ElideRight
        }

        QQC2.Label {
            visible: !delegate.narrowRow
            Layout.preferredWidth: delegate.installedWidth
            Layout.minimumWidth: delegate.installedWidth
            Layout.maximumWidth: delegate.installedWidth
            horizontalAlignment: Text.AlignRight
            rightPadding: Kirigami.Units.smallSpacing
            text: delegate.installedText
            font.family: "monospace"
            color: delegate.installed ? Kirigami.Theme.textColor
                                      : Kirigami.Theme.disabledTextColor
            elide: Text.ElideLeft
        }

        QQC2.Label {
            visible: delegate.showAvailable
            Layout.preferredWidth: delegate.availableColumnWidth
            Layout.minimumWidth: delegate.availableColumnWidth
            Layout.maximumWidth: delegate.availableColumnWidth
            horizontalAlignment: Text.AlignRight
            rightPadding: Kirigami.Units.smallSpacing
            text: delegate.availableText
            font.family: "monospace"
            // An upgradable version is the reason the row is interesting; the
            // theme's positive colour is how the tool marks a newer value.
            color: delegate.isUpdate ? Kirigami.Theme.positiveTextColor
                 : (delegate.installed ? Kirigami.Theme.textColor
                                       : Kirigami.Theme.disabledTextColor)
            elide: Text.ElideLeft
        }

        QQC2.Label {
            visible: !delegate.narrowRow
            Layout.preferredWidth: delegate.sizeWidth
            Layout.minimumWidth: delegate.sizeWidth
            Layout.maximumWidth: delegate.sizeWidth
            horizontalAlignment: Text.AlignRight
            rightPadding: Kirigami.Units.smallSpacing
            text: delegate.sizeText
            font.family: "monospace"
            elide: Text.ElideLeft
        }

        QQC2.Label {
            visible: delegate.wideRow
            Layout.preferredWidth: delegate.downloadWidth
            Layout.minimumWidth: delegate.downloadWidth
            Layout.maximumWidth: delegate.downloadWidth
            horizontalAlignment: Text.AlignRight
            rightPadding: Kirigami.Units.smallSpacing
            text: delegate.downloadText
            font.family: "monospace"
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideLeft
        }
    }
}
