// The package list: search, filter chips and the dense table.
//
// Installed, Updates, Available and Queue are the same list with a different
// `packages.filter`; this view owns the controls that narrow it further and the
// details pane that follows the selected row.
//
// The column widths are the binding contract from the UI spec: the header and
// every delegate read the same four values, so no label can push the next
// column out of line.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var packages
    property var installedBackend
    property var catalogBackend
    // The current or last finished job: Update all must not start a second
    // transaction on top of a running one.
    property var job

    signal updateAllRequested()

    // The selected row, as the `name-version_revision` xbps prints: the details
    // pane reads that one package's facts back through `catalogBackend.get()`.
    property string selectedName: ""
    property string selectedPkgver: ""

    // Fixed column widths, shared by the header and the delegates. The actions
    // are an overlay on the row's right edge, so they do not get a column.
    // 8.5 gu (153 px) cannot hold "8.12.1_1 → 8.16.0_1" at this font: the two
    // versions are 77 px each, the arrow 13 px and the two gaps 8 px. 10 gu
    // (180 px) fits it; the name column still has 263 px for 141 px of text.
    readonly property real versionWidth: Kirigami.Units.gridUnit * 10
    readonly property real stateWidth: Kirigami.Units.gridUnit * 7

    // At 14 gridUnits the fact label column (141 px) leaves only ~87 px for
    // values, which cuts `20250812_1`. 16 gridUnits give the value column
    // ~123 px, enough for the longest version and license xbps prints.
    readonly property real paneWidth: Math.max(Kirigami.Units.gridUnit * 16, 288)

    // The pane appears from 46 gridUnits of view width. The longest name needs
    // ~47.5 gu with the pane open, so a window between 58.5 and 60 gu can elide
    // it; the default (64) and minimum (48) sit outside that band.
    readonly property bool wide: width >= Kirigami.Units.gridUnit * 46

    // The row as xbps names it: `installedVersion` is the version the system
    // has, `version` the repository's.
    function pkgverOf(pkg, name) {
        const version = pkg.installedVersion !== "" ? pkg.installedVersion : pkg.version
        return name + "-" + version
    }

    function emptyText() {
        if (packages.search.length > 0) {
            return i18nd("kxbps", "No package matches “%1”.", packages.search)
        }
        switch (packages.filter) {
        case "installed":
            return i18nd("kxbps", "No packages installed.")
        case "updates":
            return i18nd("kxbps", "Everything is up to date.")
        case "queued":
            return i18nd("kxbps", "Nothing queued. Add packages from the list.")
        }
        return i18nd("kxbps", "No packages.")
    }

    // The one sentence for a failed read, chosen by the key the backend sent.
    // The arguments are already ordered for the sentence's `%1…%3`.
    function readErrorText(key, args) {
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
            return i18nd("kxbps", "%1 printed no packages.", a)
        }
        return i18nd("kxbps", "Could not read the installed packages: %1", a)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // A store that could not be loaded or saved keeps the session going in
        // memory; the reason is technical and wrapped here.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: installedBackend.error.length > 0
            color: Kirigami.Theme.neutralTextColor
            text: i18nd("kxbps", "Could not save the system state: %1", installedBackend.error)
            wrapMode: Text.WordWrap
        }

        // A read of the system that failed (xbps missing, timed out, output
        // this version cannot parse). The backend sends the key and its
        // arguments; the sentence belongs here, so the read error never borrows
        // the save-error wording above.
        QQC2.Label {
            id: readError
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: installedBackend.readError.length > 0
            color: Kirigami.Theme.negativeTextColor
            text: view.readErrorText(installedBackend.readError,
                                     installedBackend.readErrorArgs)
            wrapMode: Text.WordWrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            Kirigami.SearchField {
                id: searchField
                Layout.fillWidth: true
                placeholderText: i18nd("kxbps", "Search packages…")
                onTextChanged: view.packages.search = text
            }

            // The bulk action belongs to the Updates filter only. It sits in
            // this row, not the chip flow below, so appearing and disappearing
            // never pushes the filter chips around.
            //
            // Enablement follows the update count, never `packages.count`: the
            // search field narrows the visible rows, so a
            // search that matches no update would otherwise disable the button
            // while the system still has updates. Only a running job blocks it.
            QQC2.Button {
                visible: view.packages.filter === "updates" && view.installedBackend.updatesCount > 0
                text: i18nd("kxbps", "Update all")
                icon.name: "system-software-update"
                highlighted: true
                enabled: view.installedBackend.updatesCount > 0
                         && (view.job === null || view.job.done)
                onClicked: view.updateAllRequested()
            }
        }

        Flow {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.ButtonGroup {
                id: filterGroup
                exclusive: true
            }

            Repeater {
                // Tokens are exactly the ones `PackageModelRust::matches`
                // accepts; the labels are the only translated part.
                model: [
                    { token: "all", label: i18nd("kxbps", "All") },
                    { token: "installed", label: i18nd("kxbps", "Installed") },
                    { token: "updates", label: i18nd("kxbps", "Updates") },
                    { token: "explicit", label: i18nd("kxbps", "Explicit") },
                    { token: "orphans", label: i18nd("kxbps", "Orphans") },
                    { token: "held", label: i18nd("kxbps", "Held") },
                    { token: "queued", label: i18nd("kxbps", "Queued") }
                ]

                delegate: QQC2.Button {
                    id: chip
                    required property var modelData
                    checkable: true
                    flat: true
                    text: modelData.label
                    QQC2.ButtonGroup.group: filterGroup
                    onClicked: view.packages.filter = chip.modelData.token

                    // A Binding rather than a plain `checked:` expression: the
                    // exclusive group assigns `checked` itself, which would
                    // break the binding after the first click.
                    Binding {
                        target: chip
                        property: "checked"
                        value: view.packages.filter === chip.modelData.token
                        restoreMode: Binding.RestoreNone
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: 0

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ListView {
                    id: list
                    anchors.fill: parent
                    clip: true
                    model: view.packages
                    currentIndex: -1
                    // The header scrolls with the rows but stays on top, so a
                    // visible scrollbar cannot shift one against the other.
                    headerPositioning: ListView.OverlayHeader
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

                    header: Item {
                        width: list.width
                        height: Kirigami.Units.gridUnit * 2

                        Rectangle {
                            anchors.fill: parent
                            color: Kirigami.Theme.alternateBackgroundColor
                        }

                        Rectangle {
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            height: 1
                            color: Kirigami.Theme.disabledTextColor
                        }

                        RowLayout {
                            anchors.fill: parent
                            // Must match the delegate's row padding and column
                            // spacing, or the header drifts off its columns.
                            anchors.leftMargin: Kirigami.Units.mediumSpacing
                            anchors.rightMargin: Kirigami.Units.mediumSpacing
                            spacing: Kirigami.Units.smallSpacing

                            QQC2.Label {
                                Layout.fillWidth: true
                                text: i18nd("kxbps", "Package")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.versionWidth
                                Layout.minimumWidth: view.versionWidth
                                Layout.maximumWidth: view.versionWidth
                                text: i18nd("kxbps", "Version")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.stateWidth
                                Layout.minimumWidth: view.stateWidth
                                Layout.maximumWidth: view.stateWidth
                                text: i18nd("kxbps", "State")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }
                        }
                    }

                    delegate: PackageDelegate {
                        width: list.width
                        installedBackend: view.installedBackend
                        versionWidth: view.versionWidth
                        stateWidth: view.stateWidth
                        selected: view.selectedName === model.name
                                  && view.selectedPkgver === view.pkgverOf(model, model.name)
                        onClicked: {
                            view.selectedName = model.name
                            view.selectedPkgver = view.pkgverOf(model, model.name)
                            list.currentIndex = index
                            if (!view.wide) {
                                detailsDrawer.open()
                            }
                        }
                    }
                }

                QQC2.Label {
                    anchors.centerIn: parent
                    visible: view.packages.count === 0
                    text: view.emptyText()
                    color: Kirigami.Theme.disabledTextColor
                }
            }

            Rectangle {
                visible: view.wide
                Layout.fillHeight: true
                width: 1
                color: Kirigami.Theme.disabledTextColor
            }

            DetailsPane {
                visible: view.wide
                // Pinned, not just preferred: the pane's labels have huge
                // implicit widths (a full "required by" list), and a RowLayout
                // would otherwise hand them the whole row.
                Layout.minimumWidth: view.paneWidth
                Layout.maximumWidth: view.paneWidth
                Layout.fillHeight: true
                catalogBackend: view.catalogBackend
                installedBackend: view.installedBackend
                // The installed version when there is one, the list's otherwise.
                packageName: view.selectedPkgver
            }
        }
    }

    // The narrow placement: the same pane, opened over the list on a click.
    //
    // A right-edge drawer rather than the mockup's sheet: Kirigami 6.11's
    // `OverlaySheet` emits binding-loop warnings on Qt 6.11, and a clean runtime
    // log is worth more than the sheet's drop shadow.
    Kirigami.OverlayDrawer {
        id: detailsDrawer
        // The drawer belongs to the window overlay, not to the page: created as
        // a page child, its handle binding reads a null parent at startup.
        parent: view.Window.window ? view.Window.window.overlay : null
        edge: Qt.RightEdge
        modal: true
        // No edge handle: the pane opens from a row click, and a handle on the
        // window edge would sit on top of the log drawer.
        handleVisible: false
        width: Kirigami.Units.gridUnit * 26
        onClosed: {
            view.selectedName = ""
            view.selectedPkgver = ""
        }

        DetailsPane {
            width: detailsDrawer.width
            height: detailsDrawer.height
            catalogBackend: view.catalogBackend
            installedBackend: view.installedBackend
            packageName: view.selectedPkgver
        }
    }
}
