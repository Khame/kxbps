// The package table and the controls that narrow it.
//
// Installed, Upgradable, Explicit, Orphans, On hold and Marked are the same
// table with a different `packages.filter`; the sidebar owns the choice, so this
// view owns only what is left: the search field's debounce, the sortable header
// and the details of the selected row. The log strip below is the table's own
// footer, because the output it shows is the output of what the table queues.
//
// The column widths are the binding contract: the header and every delegate read
// the same values, so no label can push the next column out of line. The list
// ends before its scrollbar, so the bar never covers a checkbox or a number.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
// `Window` is where the narrow drawer attaches its parent; the attached
// property is not exported by the plain QtQuick import set.
import QtQuick.Window
import "../components"

Item {
    id: view

    property var packages
    property var installedBackend
    property var catalogBackend
    // The current or last finished job: the log strip reports it.
    property var job

    // The selected row, as the `name-version_revision` xbps prints.
    property string selectedName: ""
    property string selectedPkgver: ""

    // The pattern the window's toolbar holds. The field itself is in the
    // toolbar, because it is one of the window's controls; this view debounces
    // it and answers `focusSearch()`/`clearSearch()` on it.
    property string searchText: ""
    onSearchTextChanged: {
        if (view.packages !== null && view.packages.search !== view.searchText) {
            searchDebounce.restart()
        }
    }

    // The row form, from the space the table actually gets: the view's width
    // only settles after the sidebar and the page column have taken theirs, so
    // the thresholds read the list, whose width is the truth the rows render
    // into. Three boundaries, the ones the plan fixed: every column, the
    // description folded under the name, and the narrow form.
    readonly property real tableWidth: Math.max(0, list.width - view.listScrollBarSpace)
    // The queue is a list of changes, not a table of packages: the action, the
    // version and the size are the whole row. A search inside the queue falls
    // back to the table, which is the only form that can rank an answer.
    readonly property bool queueMode: view.packages !== null
                                      && view.packages.filter === "queued"
                                      && view.packages.search.length === 0
    // The header row's height. The bars sit on it, so both read one number.
    readonly property real headerHeight: Kirigami.Units.gridUnit * 2
    // The thresholds are the window sizes the acceptance test names, carried
    // over to the table by the width the sidebar takes: 1200 px of window is
    // about 44 grid units of table and 900 px about 28, so `Wide` starts a
    // little below the first and `Medium` a little below the second.
    readonly property bool wide: view.tableWidth >= Kirigami.Units.gridUnit * 44
    readonly property bool medium: view.tableWidth >= Kirigami.Units.gridUnit * 28
    readonly property int rowLayout: view.wide ? PackageDelegate.Wide
                                    : (view.medium ? PackageDelegate.Medium
                                                   : PackageDelegate.Narrow)

    // Column widths, shared by the header and the delegates. Grid units, not
    // pixels: the same numbers scale with the user's font.
    readonly property real checkWidth: Kirigami.Units.gridUnit * 1.75
    // The gutter between two columns. The resize bar lives in it, so a bar can
    // never sit on top of a value.
    readonly property real gutter: Kirigami.Units.gridUnit * 0.75
    readonly property bool wideRow: view.rowLayout === PackageDelegate.Wide
    readonly property bool narrowRow: view.rowLayout === PackageDelegate.Narrow

    // The widths the user sets. The name is one of them: a name column that
    // only ever took the leftover could not be widened, and a cut package name
    // is the reason to reach for a bar in the first place. Until a drag gives it
    // a number of its own, it follows the window: three tenths of the table,
    // never narrower than nine grid units.
    readonly property real defaultNameWidth: Math.max(Kirigami.Units.gridUnit * 9,
                                                      view.tableWidth * 0.30)
    property real nameWidth: view.defaultNameWidth
    property real installedWidth: Kirigami.Units.gridUnit * 7
    property real availableColumnWidth: Kirigami.Units.gridUnit * 7
    property real sizeWidth: Kirigami.Units.gridUnit * 5
    property real downloadWidth: Kirigami.Units.gridUnit * 6

    // The repository version is worth a column where the row is about moving
    // between versions: the updates and the whole repository. On the installed
    // set it repeats the installed version, so it is not drawn there.
    readonly property bool showAvailable: view.packages !== null
                                          && (view.packages.filter === "updates"
                                              || view.packages.filter === "all")

    // The columns with a width of their own, in the order the row draws them.
    // `skip` leaves one out, which is how the stretching column measures the
    // room it is allowed to take.
    function sizedColumns(skip) {
        const columns = []
        if (skip !== "name") {
            columns.push({ key: "name", width: view.nameWidth })
        }
        if (view.wideRow && skip !== "description") {
            columns.push({ key: "description", width: view.descriptionWidth })
        }
        if (!view.narrowRow && skip !== "installed") {
            columns.push({ key: "installed", width: view.installedWidth })
        }
        if (view.showAvailable && skip !== "available") {
            columns.push({ key: "available", width: view.availableColumnWidth })
        }
        if (!view.narrowRow && skip !== "size") {
            columns.push({ key: "size", width: view.sizeWidth })
        }
        if (view.wideRow && skip !== "download") {
            columns.push({ key: "download", width: view.downloadWidth })
        }
        return columns
    }

    // What a row of these columns takes: the row's own insets, the checkbox,
    // the widths, and one gutter between each pair.
    function rowWidth(columns) {
        let width = Kirigami.Units.mediumSpacing * 2 + view.checkWidth
        for (let i = 0; i < columns.length; ++i) {
            width += columns[i].width
        }
        return width + view.gutter * columns.length
    }

    // The description stretches: it is the column that can be long without
    // losing meaning, and letting it take the leftover is what makes the table
    // end at the window's edge instead of leaving a hole on the right. One
    // gutter is added back because the skipped column still owns the gap that
    // follows it.
    readonly property real descriptionWidth: {
        if (!view.wideRow) {
            return 0
        }
        // Two spacings of slack: a row that asks for exactly the width it has
        // gets squeezed by the layout's own rounding, and the header and the
        // delegates then squeeze different cells.
        const room = view.tableWidth - view.rowWidth(view.sizedColumns("description"))
                     - view.gutter - Kirigami.Units.smallSpacing * 2
        return Math.max(Kirigami.Units.gridUnit * 6, room)
    }

    // In the two narrow forms there is no description column, so the name takes
    // the leftover instead: the row still ends at the window's edge, and the
    // package name is the one thing that must not be cut.
    readonly property real effectiveNameWidth: view.wideRow
        ? view.nameWidth
        : Math.max(Kirigami.Units.gridUnit * 8,
                   view.tableWidth - view.rowWidth(view.sizedColumns("name")) - view.gutter)

    // The space the list's scrollbar takes when it is visible. Delegates and
    // the header end before it, so the bar never covers a control or a number.
    readonly property real listScrollBarSpace: listBar.visible ? listBar.width : 0

    // The details area at the bottom, open while a row is selected. Below
    // 45 gu it would take the table's whole height, so it becomes the drawer.
    readonly property bool detailsVisible: view.medium && view.selectedName.length > 0

    // The narrow form has no details area; the log strip's `Details` sibling
    // opens the drawer instead.
    readonly property bool narrowDetails: !view.medium && view.selectedName.length > 0

    onDetailsVisibleChanged: {
        if (view.narrowDetails) {
            detailsDrawer.open()
        } else if (!view.medium && !detailsDrawer.opened) {
            view.clearSelection()
        }
    }

    // One selection, whatever moved it: a click and the keyboard both land
    // here, so the row highlight and the details pane follow either. The
    // delegate is the only place the row's fields are safe to read; a delegate
    // that is being created or destroyed has no name yet.
    function select(item) {
        if (item === null || item === undefined || item.packageName === undefined
                || item.packageName.length === 0) {
            return
        }
        view.selectedName = item.packageName
        view.selectedPkgver = item.packagePkgver
    }

    // Drops the selection, so the details area closes with the row it described.
    function clearSelection() {
        view.selectedName = ""
        view.selectedPkgver = ""
    }

    // The header's drag handles call this: one place clamps and assigns, so a
    // handle cannot make a column narrower than its header or wider than the
    // table. The description is not here: it takes whatever the fixed columns
    // leave.
    function setColumnWidth(name, width, minimum, maximum) {
        const next = Math.min(Math.max(width, minimum), maximum)
        switch (name) {
        case "installed":
            view.installedWidth = next
            break
        case "available":
            view.availableColumnWidth = next
            break
        case "size":
            view.sizeWidth = next
            break
        case "download":
            view.downloadWidth = next
            break
        case "name":
            view.nameWidth = next
            break
        }
    }

    // Moves one boundary. The left column takes `dx` and the right one gives up
    // exactly that much, so the sum of the two stays the same and the table
    // does not reflow around a third column. Clamping one side pulls the other
    // back with it, which keeps the boundary under the pointer.
    //
    // The description is the column that stretches, so it has no width to
    // assign: a boundary beside it moves the other column instead, and the
    // description takes whatever is left over.
    function resizeColumns(left, right, startLeft, startRight, dx,
                           minLeft, maxLeft, minRight, maxRight) {
        if (left === "description") {
            setColumnWidth(right, startRight - dx, minRight, maxRight)
            return
        }
        if (right === "description") {
            // The description is derived, so it cannot be assigned; instead the
            // name stops where the description would fall under its minimum.
            const room = startLeft + (startRight - minRight)
            setColumnWidth(left, Math.min(startLeft + dx, room), minLeft, maxLeft)
            return
        }
        let leftWidth = Math.min(Math.max(startLeft + dx, minLeft), maxLeft)
        if (right.length === 0) {
            // The last column has no neighbour to trade with, so it is the only
            // one that can widen the row. Clamp it to the room the table has.
            const room = view.tableWidth - view.rowWidth(view.sizedColumns("download"))
                         - view.gutter
            leftWidth = Math.min(leftWidth, Math.max(room, minLeft))
            setColumnWidth(left, leftWidth, minLeft, maxLeft)
            return
        }
        const shift = leftWidth - startLeft
        let rightWidth = startRight - shift
        if (rightWidth < minRight) {
            rightWidth = minRight
            leftWidth = startLeft + (startRight - minRight)
        } else if (rightWidth > maxRight) {
            rightWidth = maxRight
            leftWidth = startLeft + (startRight - maxRight)
        }
        setColumnWidth(left, leftWidth, minLeft, maxLeft)
        setColumnWidth(right, rightWidth, minRight, maxRight)
    }

    // The columns the current form draws, left to right, each with the width it
    // starts at and the range a drag may take it to. One list feeds the bars, so
    // a bar can never point at a column the row does not draw: the description
    // exists only in the wide form, the versions and sizes fold away in the
    // narrow one, and the repository version is a column only where it says
    // something.
    function visibleColumns() {
        const gu = Kirigami.Units.gridUnit
        const wide = view.wideRow
        const narrow = view.narrowRow
        const columns = [
            { key: "name", width: view.effectiveNameWidth,
              minimum: gu * 8, maximum: gu * 40 }
        ]
        if (wide) {
            columns.push({ key: "description", width: view.descriptionWidth,
                           minimum: gu * 6, maximum: gu * 40 })
        }
        if (!narrow) {
            columns.push({ key: "installed", width: view.installedWidth,
                           minimum: gu * 7, maximum: gu * 14 })
        }
        if (view.showAvailable) {
            columns.push({ key: "available", width: view.availableColumnWidth,
                           minimum: gu * 7, maximum: gu * 14 })
        }
        if (!narrow) {
            columns.push({ key: "size", width: view.sizeWidth,
                           minimum: gu * 5, maximum: gu * 10 })
        }
        if (wide) {
            columns.push({ key: "download", width: view.downloadWidth,
                           minimum: gu * 5, maximum: gu * 10 })
        }
        return columns
    }

    // The column behind a key, as the current form draws it, or null when the
    // form leaves it out.
    function columnInfo(key) {
        const columns = view.visibleColumns()
        for (let i = 0; i < columns.length; ++i) {
            if (columns[i].key === key) {
                return columns[i]
            }
        }
        return null
    }

    // The column on the other side of a boundary, or null when the key is the
    // last column the form draws.
    function nextColumnInfo(key) {
        const columns = view.visibleColumns()
        for (let i = 0; i < columns.length; ++i) {
            if (columns[i].key === key) {
                return i + 1 < columns.length ? columns[i + 1] : null
            }
        }
        return null
    }

    // The x of a column's right edge, in the table's coordinates: the header's
    // left inset and the checkbox column, then every drawn column before it and
    // the gap that follows each one, with the bar in the middle of that gap.
    // The header and the rows share the grid, so one measurement serves both.
    function columnEdge(key) {
        const gap = view.gutter
        const columns = view.visibleColumns()
        let start = Kirigami.Units.mediumSpacing + view.checkWidth + gap
        for (let i = 0; i < columns.length; ++i) {
            if (columns[i].key === key) {
                // The middle of the gutter that follows the column: the bar
                // stands on the column's right edge, not on its left one.
                return start + columns[i].width + gap / 2
            }
            start += columns[i].width + gap
        }
        return 0
    }

    // The resize bar on a column's right edge.
    //
    // It runs the height of the whole table, not just the header row, so a
    // boundary can be caught wherever the pointer happens to be. A drag trades
    // width with the column on the other side: the left one grows by exactly
    // what the right one gives up, so no third column changes behind the user's
    // back. An empty `nextColumn` is the last column, which has no neighbour
    // and clamps against the table instead.
    component ColumnGrip: MouseArea {
        id: grip

        property string column
        property real value: 0
        property real minimum: Kirigami.Units.gridUnit * 5
        property real maximum: Kirigami.Units.gridUnit * 20
        property string nextColumn: ""
        property real nextValue: 0
        property real nextMinimum: Kirigami.Units.gridUnit * 5
        property real nextMaximum: Kirigami.Units.gridUnit * 20
        property real startWidth: 0
        property real startNextWidth: 0
        property real startX: 0

        width: view.gutter
        height: parent === null ? 0 : parent.height
        z: 5
        hoverEnabled: true
        // Without this the list's Flickable takes the drag and the boundary
        // never moves.
        preventStealing: true
        cursorShape: Qt.SplitHCursor

        // A separator, not an accent: the same line the rows are divided by,
        // standing on end and a little short of the row's height. It is a
        // `Kirigami.Separator`, so it takes its colour from the theme exactly
        // as the horizontal lines in the list do, instead of guessing a shade.
        Kirigami.Separator {
            anchors.centerIn: parent
            width: Kirigami.Units.smallSpacing / 2
            height: parent.height * 0.8
            visible: !grip.containsMouse && !grip.pressed
        }

        // Under the pointer the line becomes the accent colour: that is the
        // only moment it has to be loud.
        Rectangle {
            anchors.centerIn: parent
            width: Kirigami.Units.smallSpacing / 2
            height: parent.height * 0.8
            visible: grip.containsMouse || grip.pressed
            color: Kirigami.Theme.highlightColor
        }

        onPressed: (mouse) => {
            startWidth = value
            startNextWidth = nextValue
            startX = mapToItem(view, mouse.x, mouse.y).x
        }
        onPositionChanged: (mouse) => {
            if (!pressed) {
                return
            }
            const dx = mapToItem(view, mouse.x, mouse.y).x - startX
            view.resizeColumns(column, nextColumn,
                               startWidth, startNextWidth, dx,
                               minimum, maximum, nextMinimum, nextMaximum)
        }
    }

    // The sidebar's `Available` entry: no command lists what a repository holds,
    // so the entry puts the cursor where that list would come from.
    function focusSearch() {
        view.searchFieldRequested()
    }

    // The window owns the field, so asking it to take focus is a signal.
    signal searchFieldRequested()

    // Enter in the toolbar's field applies the pattern at once.
    function submitSearch() {
        searchDebounce.stop()
        if (view.packages !== null) {
            view.packages.search = view.searchText
        }
    }

    // A filter change narrows the installed set, so it drops the repository
    // search; the field has to drop the pattern with it, or the list would show
    // installed rows under a pattern it no longer applies.
    function clearSearch() {
        view.searchText = ""
    }

    // The row as xbps names it: `installedVersion` is the version the system
    // has, `version` the repository's.
    function pkgverOf(pkg, name) {
        const version = pkg.installedVersion !== "" ? pkg.installedVersion : pkg.version
        return name + "-" + version
    }

    function emptyText() {
        if (view.packages.search.length > 0) {
            return i18nd("kxbps", "No package matches “%1”.", view.packages.search)
        }
        switch (view.packages.filter) {
        case "installed":
            return i18nd("kxbps", "No packages installed.")
        case "updates":
            return i18nd("kxbps", "Everything is up to date.")
        case "queued":
            return i18nd("kxbps", "Nothing marked. Add packages from the list.")
        case "held":
            return i18nd("kxbps", "No package is on hold.")
        case "orphans":
            return i18nd("kxbps", "No orphans.")
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

    // The one sentence for a failed save, chosen by the key the backend sent.
    // The `Display` text of a store error names paths and OS reasons, so it
    // stays in the log; the window gets a key and its `%1…%2` arguments.
    function storeErrorText(key, args) {
        const a = args.length > 0 ? args[0] : ""
        const b = args.length > 1 ? args[1] : ""
        switch (key) {
        case "errorStoreNoDataDir":
            return i18nd("kxbps", "The data directory could not be located: neither XDG_DATA_HOME nor HOME is set.")
        case "errorStoreWrite":
            return i18nd("kxbps", "Could not write %1: %2", a, b)
        case "errorStoreFormat":
            return i18nd("kxbps", "Could not read %1: %2", a, b)
        }
        return i18nd("kxbps", "Could not save the system state.")
    }

    // The table's own footer: the count of what the list shows. The window bar
    // carries the tool's state, so numbers live next to the table that
    // produced them.
    function footerText() {
        if (view.packages === null) {
            return ""
        }
        // The queue page reads the store's own list, which also holds queued
        // installs the package table cannot show.
        if (view.queueMode) {
            return i18ndp("kxbps", "%1 queued package", "%1 queued packages",
                          view.installedBackend.queueModel.count)
        }
        const count = view.packages.count
        // A read in flight: the rows on screen are the previous answer, so a
        // number would describe a list that is about to change.
        if (view.packages.searchLoading) {
            return view.packages.search.length > 0
                    ? i18nd("kxbps", "Searching the repositories…")
                    : i18nd("kxbps", "Reading the repository list…")
        }
        // A pattern asks the repositories, so the number is the size of the
        // answer, not of the installed set.
        if (view.packages.search.length > 0) {
            return i18ndp("kxbps", "%1 search result", "%1 search results", count)
        }
        switch (view.packages.filter) {
        case "installed":
            return i18ndp("kxbps", "%1 installed package", "%1 installed packages", count)
        case "updates":
            return i18ndp("kxbps", "%1 update", "%1 updates", count)
        case "queued":
            return i18ndp("kxbps", "%1 queued package", "%1 queued packages", count)
        }
        return i18ndp("kxbps", "%1 package", "%1 packages", count)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // The list cannot read xbps itself: it holds no transport. A non-empty
        // search asks the query façade, and the answer comes back as one JSON
        // document the model turns into rows.
        Connections {
            target: view.packages
            function onSearchRequested(pattern) {
                view.catalogBackend.readListSearch(pattern)
            }
        }

        Connections {
            target: view.catalogBackend
            function onListSearchDone(request, json) {
                const answer = JSON.parse(json)
                // `applySearchResults` takes the hits on their own: the model
                // ranks rows, it does not parse the transport's envelope.
                view.installedBackend.applySearchResults(JSON.stringify(answer.hits),
                                                         answer.errorKey,
                                                         answer.errorArgs)
            }
        }

        // A store that could not be loaded or saved keeps the session going in
        // memory; the reason is technical and wrapped here.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: installedBackend.error.length > 0
            color: Kirigami.Theme.neutralTextColor
            text: view.storeErrorText(installedBackend.error, installedBackend.errorArgs)
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        // A read of the system that failed (xbps missing, timed out, output this
        // version cannot parse). The backend sends the key and its arguments;
        // the sentence belongs here, so the read error never borrows the
        // save-error wording above.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: installedBackend.readError.length > 0
            color: Kirigami.Theme.negativeTextColor
            text: view.readErrorText(installedBackend.readError, installedBackend.readErrorArgs)
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        // A repository search that failed says so where the pattern was typed:
        // an empty list otherwise reads like "no such package".
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            visible: view.packages.searchError.length > 0
            text: view.readErrorText(view.packages.searchError, view.packages.searchErrorArgs)
            color: Kirigami.Theme.negativeTextColor
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        // The queue in its own form: one change per row, with the gesture that
        // takes it back out. The table's columns would say the same thing in
        // more words.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 0
            visible: view.queueMode

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                Kirigami.Separator {
                    Layout.fillWidth: true
                }

                ListView {
                    id: queueList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    // The queue is the store's own list, so a queued install of
                    // a package the machine does not hold is on it: the package
                    // table only knows installed rows.
                    model: view.installedBackend.queueModel
                    currentIndex: -1

                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        policy: QQC2.ScrollBar.AsNeeded
                    }

                    delegate: QueueDelegate {
                        width: queueList.width - view.listScrollBarSpace
                        installedBackend: view.installedBackend
                    }
                }
            }
        }

        // The table is one box so a resize bar can run its whole height: the
        // header is a fixed row, the list scrolls under it, and the bars sit
        // over both.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 0
            visible: !view.queueMode

            ColumnLayout {
                anchors.fill: parent
                spacing: 0

                // The header is a sibling of the list, not its `header`: a fixed
                // row that never scrolls away and never draws over the first
                // package.
                Item {
                    id: headerRow
                    Layout.fillWidth: true
                    Layout.rightMargin: view.listScrollBarSpace
                    Layout.preferredHeight: view.headerHeight

                    Rectangle {
                        anchors.fill: parent
                        color: Kirigami.Theme.alternateBackgroundColor
                    }

                    Kirigami.Separator {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                    }

                    RowLayout {
                        anchors.fill: parent
                        // Must match the delegate's row padding and column spacing,
                        // or the header drifts off its columns. The gutter is the
                        // lane the resize bars live in.
                        anchors.leftMargin: Kirigami.Units.mediumSpacing
                        anchors.rightMargin: Kirigami.Units.mediumSpacing
                        spacing: view.gutter

                        // The checkbox cell has no label, but the header needs
                        // its width: without it every heading sits one cell to
                        // the left of the column it names.
                        Item {
                            Layout.preferredWidth: view.checkWidth
                            Layout.minimumWidth: view.checkWidth
                            Layout.maximumWidth: view.checkWidth
                        }

                        QQC2.Label {
                            Layout.preferredWidth: view.effectiveNameWidth
                            Layout.minimumWidth: view.effectiveNameWidth
                            Layout.maximumWidth: view.effectiveNameWidth
                            rightPadding: Kirigami.Units.smallSpacing
                            text: i18nd("kxbps", "Package")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight

                            MouseArea {
                                anchors.fill: parent
                                onClicked: {
                                    const already = view.packages.sort === "name"
                                    view.packages.sort = "name"
                                    view.packages.sortDescending = already
                                                                   ? !view.packages.sortDescending : false
                                }
                            }
                        }

                        QQC2.Label {
                            visible: view.rowLayout === PackageDelegate.Wide
                            Layout.preferredWidth: view.descriptionWidth
                            Layout.minimumWidth: view.descriptionWidth
                            Layout.maximumWidth: view.descriptionWidth
                            rightPadding: Kirigami.Units.smallSpacing
                            text: i18nd("kxbps", "Description")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                        }

                        QQC2.Label {
                            visible: view.rowLayout !== PackageDelegate.Narrow
                            Layout.preferredWidth: view.installedWidth
                            Layout.minimumWidth: view.installedWidth
                            Layout.maximumWidth: view.installedWidth
                            horizontalAlignment: Text.AlignRight
                            text: i18nd("kxbps", "Installed")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            rightPadding: Kirigami.Units.smallSpacing
                        }

                        QQC2.Label {
                            visible: view.showAvailable
                            Layout.preferredWidth: view.availableColumnWidth
                            Layout.minimumWidth: view.availableColumnWidth
                            Layout.maximumWidth: view.availableColumnWidth
                            horizontalAlignment: Text.AlignRight
                            text: i18nd("kxbps", "Available")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            rightPadding: Kirigami.Units.smallSpacing
                        }

                        QQC2.Label {
                            visible: view.rowLayout !== PackageDelegate.Narrow
                            Layout.preferredWidth: view.sizeWidth
                            Layout.minimumWidth: view.sizeWidth
                            Layout.maximumWidth: view.sizeWidth
                            horizontalAlignment: Text.AlignRight
                            text: i18nd("kxbps", "Size")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            // A header is a word, not a value: a small right inset
                            // keeps it from touching the next column's label.
                            rightPadding: Kirigami.Units.smallSpacing

                            // The whole list is sorted, not the visible rows:
                            // the model orders every candidate before filtering.
                            // First click is largest first, the second reverses it.
                            MouseArea {
                                anchors.fill: parent
                                onClicked: {
                                    const already = view.packages.sort === "size"
                                    view.packages.sort = "size"
                                    view.packages.sortDescending = already
                                                                   ? !view.packages.sortDescending : true
                                }
                            }
                        }

                        QQC2.Label {
                            visible: view.rowLayout === PackageDelegate.Wide
                            Layout.preferredWidth: view.downloadWidth
                            Layout.minimumWidth: view.downloadWidth
                            Layout.maximumWidth: view.downloadWidth
                            horizontalAlignment: Text.AlignRight
                            text: i18nd("kxbps", "Download")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            rightPadding: Kirigami.Units.smallSpacing
                        }

                    }
                }

                // The header and the list share one column grid, so the header rows sit
                // exactly over the cells they name.
                ListView {
                    id: list
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.minimumHeight: 0
                    clip: true
                    model: view.packages
                    currentIndex: -1
                    // Arrow keys move the current row; the selection follows it.
                    onCurrentItemChanged: view.select(currentItem)
                    // A model reset (filter or search change) drops the current row: the
                    // window must not keep acting on a row that is no longer on screen.
                    onCountChanged: {
                        if (currentIndex < 0 && view.selectedName.length > 0) {
                            view.clearSelection()
                        }
                    }
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        id: listBar
                        policy: QQC2.ScrollBar.AsNeeded
                    }


                    delegate: PackageDelegate {
                        width: list.width - view.listScrollBarSpace
                        installedBackend: view.installedBackend
                        layout: view.rowLayout
                        checkWidth: view.checkWidth
                        gutter: view.gutter
                        nameWidth: view.effectiveNameWidth
                        descriptionWidth: view.descriptionWidth
                        installedWidth: view.installedWidth
                        availableColumnWidth: view.availableColumnWidth
                        showAvailable: view.showAvailable
                        sizeWidth: view.sizeWidth
                        downloadWidth: view.downloadWidth
                        selected: view.selectedName === packageName
                                  && view.selectedPkgver === packagePkgver
                        onClicked: {
                            view.select(this)
                            list.currentIndex = index
                        }
                    }

                    // The empty state stays centred in the list's own area, under the
                    // header: a sentence, not an illustration.
                    QQC2.Label {
                        anchors.centerIn: parent
                        width: parent.width - Kirigami.Units.largeSpacing * 2
                        visible: view.packages.count === 0
                        text: view.emptyText()
                        color: Kirigami.Theme.disabledTextColor
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.WordWrap
                    }
                }

            }

            // One bar per column edge, on the header row: that is where the
            // column names are, so that is where the hand looks for the edge.
            // The layer carries no MouseArea of its own, and it stops at the
            // header's bottom edge, so nothing over the rows changes.
            Item {
                id: gripLayer
                anchors.top: parent.top
                anchors.left: parent.left
                width: view.tableWidth
                height: view.headerHeight
                z: 10

                Repeater {
                    // A fixed list, so the bars are created once: a model that
                    // is rebuilt on every width change would destroy the bar
                    // that is being dragged. The last column is not here: it has
                    // no neighbour to trade width with, so its edge is not a
                    // handle.
                    model: ["name", "description", "installed", "available", "size"]

                    ColumnGrip {
                        readonly property var self: view.columnInfo(modelData)
                        readonly property var neighbour: view.nextColumnInfo(modelData)

                        visible: self !== null
                        x: view.columnEdge(modelData) - width / 2
                        column: modelData
                        value: self === null ? 0 : self.width
                        minimum: self === null ? 0 : self.minimum
                        maximum: self === null ? 0 : self.maximum
                        nextColumn: neighbour === null ? "" : neighbour.key
                        nextValue: neighbour === null ? 0 : neighbour.width
                        nextMinimum: neighbour === null ? 0 : neighbour.minimum
                        nextMaximum: neighbour === null ? 0 : neighbour.maximum
                    }
                }
            }
        }

        // The table's footer: what the list shows, right under it. The window
        // bar carries the tool's state instead of numbers.
        Kirigami.Separator {
            Layout.fillWidth: true
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            text: view.footerText()
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }

        // The details of the selected row, under the table. The panels take
        // height, not width, so the columns keep their sizes.
        Kirigami.Separator {
            Layout.fillWidth: true
            visible: view.detailsVisible
        }

        DetailsPane {
            visible: view.detailsVisible
            Layout.fillWidth: true
            Layout.preferredHeight: view.detailsVisible ? Kirigami.Units.gridUnit * 13 : 0
            rowLayout: view.rowLayout
            catalogBackend: view.catalogBackend
            installedBackend: view.installedBackend
            packageName: view.selectedPkgver
            packageLabel: view.selectedName
        }

        // The narrow form's way into the facts, next to the log strip. It only
        // exists below 45 gu, where the details area is gone.
        QQC2.ItemDelegate {
            Layout.fillWidth: true
            visible: view.narrowDetails
            onClicked: detailsDrawer.open()

            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 0
                    text: i18nd("kxbps", "Details of %1", view.selectedName)
                    elide: Text.ElideRight
                    font: Kirigami.Theme.smallFont
                }

                QQC2.Label {
                    text: i18nd("kxbps", "Show")
                    color: Kirigami.Theme.disabledTextColor
                    font: Kirigami.Theme.smallFont
                }
            }
        }

    }

    // The narrow placement: the same facts, opened from the `Details` strip
    // above the log. A right-edge drawer rather than a sheet: Kirigami 6.11's
    // `OverlaySheet` emits binding-loop warnings on Qt 6.11, and a clean runtime
    // log is worth more than the sheet's drop shadow.
    Kirigami.OverlayDrawer {
        id: detailsDrawer
        // The drawer belongs to the window overlay, not to the page: created as
        // a page child, its handle binding reads a null parent at startup.
        parent: view.Window.window ? view.Window.window.overlay : null
        edge: Qt.RightEdge
        modal: true
        // No edge handle: the pane opens from the strip, and a handle on the
        // window edge would sit on top of the log drawer.
        handleVisible: false
        width: Math.min(Kirigami.Units.gridUnit * 26,
                        view.Window.window ? view.Window.window.width * 0.85
                                           : Kirigami.Units.gridUnit * 26)
        onClosed: {
            if (!view.medium) {
                view.selectedName = ""
                view.selectedPkgver = ""
            }
        }

        DetailsPane {
            width: detailsDrawer.width
            height: detailsDrawer.height
            rowLayout: PackageDelegate.Narrow
            catalogBackend: view.catalogBackend
            installedBackend: view.installedBackend
            packageName: view.selectedPkgver
            packageLabel: view.selectedName
        }
    }

    // Typing narrows the list once the pattern settles; `submitSearch` applies
    // it at once when the field is accepted. In memory this is cheap, but 1340
    // rows per keystroke still reset the list, so the pause wins.
    Timer {
        id: searchDebounce
        interval: 200
        onTriggered: view.packages.search = view.searchText
    }
}
