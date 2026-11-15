// The mirror list and the repository state.
//
// Mirrors are the embedded 34-entry snapshot; refreshing the list from
// `void-linux/xmirror` and editing repositories are later tasks. Both tabs are
// read-only: the mirrors tab selects a mirror and, through the status strip,
// offers to apply it; the repositories tab reports what xbps reports, from
// `xbps-query -L` and the `.conf` files, and never invents a row.
//
// The two tabs answer two different questions. The mirrors tab is the app's
// choice (`mirror.json`, `Selected`); the repositories tab is the system's
// state (`/etc/xbps.d`, `/usr/share/xbps.d`). `In use` on the first tab and the
// rows of the second come from the same read, `mirrorBackend.systemRepos` and
// `mirrorBackend.systemMirror`.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../Wire.js" as Wire
import "../components"

Item {
    id: view

    property var mirrorBackend

    // Column widths, shared by the heading and every row: one number per
    // column, or the heading drifts off the values it names.
    readonly property real addressWidth: Kirigami.Units.gridUnit * 13
    readonly property real regionWidth: Kirigami.Units.gridUnit * 4
    readonly property real tierWidth: Kirigami.Units.gridUnit * 4
    // The heading is a word, the values are numbers: at five grid units
    // the Polish "Opóźnienie" was cut in half and the column drifted
    // from the rows, which use this same width.
    readonly property real latencyWidth: Kirigami.Units.gridUnit * 7
    readonly property real stateWidth: Kirigami.Units.gridUnit * 13
    readonly property real countWidth: Kirigami.Units.gridUnit * 6
    readonly property real signatureWidth: Kirigami.Units.gridUnit * 8
    readonly property real fileWidth: Kirigami.Units.gridUnit * 18

    // `xbps-query -L` plus the `.conf` files, as `{url, count, signed, file}`.
    // Recomputed only when the backend re-reads, so `JSON.parse` never runs per
    // binding evaluation. A value this view cannot parse is an empty list: a
    // half-parsed repository table would claim a state xbps did not report.
    property var repos: []
    // The strip the scrollbar takes when it is visible. Rows and the header end
    // before it, so the state badge is never cut by the bar.
    readonly property real mirrorScrollBarSpace: mirrorBar.visible ? mirrorBar.width : 0

    // The region codes the model knows, as the JSON array it publishes.
    readonly property var regionCodes: Wire.array(view.mirrorBackend.model.regionCodes)

    // Region codes are data; the words are UI copy.
    function regionLabel(code) {
        switch (code) {
        case "EU":
            return i18nd("kxbps", "Europe")
        case "NA":
            return i18nd("kxbps", "North America")
        case "AS":
            return i18nd("kxbps", "Asia")
        case "OC":
            return i18nd("kxbps", "Oceania")
        case "SA":
            return i18nd("kxbps", "South America")
        case "World":
            return i18nd("kxbps", "Worldwide")
        }
        return code
    }

    // The regions the combo offers, in the model's order. `World` is dropped:
    // its entry is the "Worldwide" one, which means every mirror rather than
    // the handful tagged `World`.
    readonly property var filterRegions: view.regionCodes.filter(code => code !== "World")

    // One list, two kinds of filter: every region first, then "Worldwide",
    // which clears the filter, then the one state worth isolating. "All" and
    // "OK" are gone — no filter is the default, and two more entries only made
    // the list longer without answering a question the user asks.
    function filterEntries() {
        const entries = []
        const regions = view.filterRegions
        for (let i = 0; i < regions.length; ++i) {
            entries.push(view.regionLabel(regions[i]))
        }
        entries.push(i18nd("kxbps", "Worldwide"))
        entries.push(i18nd("kxbps", "Failed"))
        return entries
    }

    // The index of the entry the model currently describes.
    readonly property int filterIndex: {
        const model = view.mirrorBackend.model
        const regions = view.filterRegions
        if (model.regionFilter.length > 0) {
            const at = regions.indexOf(model.regionFilter)
            return at >= 0 ? at : regions.length
        }
        if (model.filter === 2) {
            return regions.length + 1
        }
        return regions.length
    }

    function applyFilter(index) {
        const model = view.mirrorBackend.model
        const regions = view.filterRegions
        if (index < regions.length) {
            model.filter = 0
            model.regionFilter = regions[index]
        } else if (index === regions.length) {
            // "Worldwide": every mirror, not only the ones tagged `World`.
            model.filter = 0
            model.regionFilter = ""
        } else {
            model.regionFilter = ""
            model.filter = 2
        }
    }

    // The host of a URL, for the footer sentence; `/` marks the last one.
    function hostOf(url) {
        let rest = url.indexOf("://") >= 0 ? url.slice(url.indexOf("://") + 3) : url
        const slash = rest.indexOf("/")
        return slash >= 0 ? rest.slice(0, slash) : rest
    }

    // xbps reads repositories in `-L` order, lowering counts first; the number
    // is how many packages the index holds, so a bigger one wins. `-1` is a
    // count xbps could not read, not a small number, so it sorts last in both
    // directions and never outranks a readable index.
    function sortedRepos(list) {
        return list.slice().sort((a, b) => {
            const left = a.count < 0 ? -1 : a.count
            const right = b.count < 0 ? -1 : b.count
            if (left !== right) {
                return right - left
            }
            return a.url < b.url ? -1 : (a.url > b.url ? 1 : 0)
        })
    }

    onMirrorBackendChanged: repos = Wire.array(mirrorBackend ? mirrorBackend.systemRepos : "[]")
    Component.onCompleted: {
        repos = Wire.array(mirrorBackend ? mirrorBackend.systemRepos : "[]")
        // Europe first: the list is 34 rows long and the nearest mirrors are
        // the ones a user picks. "Worldwide" in the combo clears it again.
        if (mirrorBackend && mirrorBackend.model.regionFilter.length === 0) {
            mirrorBackend.model.regionFilter = "EU"
        }
    }

    Connections {
        target: view.mirrorBackend
        function onSystemReposChanged() {
            view.repos = Wire.array(view.mirrorBackend.systemRepos)
        }
        // The first frame can bind before the backend's own read has answered;
        // asking here keeps the tab from sitting on "No repositories." when a
        // value is already in hand.
        function onSystemLoadingChanged() {
            if (!view.mirrorBackend.systemLoading) {
                view.repos = Wire.array(view.mirrorBackend.systemRepos)
            }
        }
    }

    StackLayout {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom

        ColumnLayout {
            spacing: 0

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                // The header is a sibling of the list, not its `header`: an
                // overlay header draws over the first row, which is how the
                // latency and state headings ended up printed on a mirror's
                // badges.
                Item {
                    id: mirrorHeader
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: Kirigami.Units.gridUnit * 2

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
                        // The same gutter the rows use, or the headings sit one
                        // step to the left of the columns they name.
                        anchors.leftMargin: Kirigami.Units.gridUnit * 2
                        anchors.rightMargin: Kirigami.Units.largeSpacing
                                             + view.mirrorScrollBarSpace
                        spacing: Kirigami.Units.largeSpacing

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Server")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                        }

                        QQC2.Label {
                            Layout.preferredWidth: view.addressWidth
                            Layout.minimumWidth: view.addressWidth
                            Layout.maximumWidth: view.addressWidth
                            text: i18nd("kxbps", "Address")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                        }

                        QQC2.Label {
                            Layout.preferredWidth: view.regionWidth
                            Layout.minimumWidth: view.regionWidth
                            Layout.maximumWidth: view.regionWidth
                            text: i18nd("kxbps", "Region")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                        }

                        QQC2.Label {
                            Layout.preferredWidth: view.tierWidth
                            Layout.minimumWidth: view.tierWidth
                            Layout.maximumWidth: view.tierWidth
                            // Not translated: "Tier" is the word `xmirror`
                            // prints, a Void term like the region codes.
                            text: "Tier"
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

                        // The sort is the model's, not a QML reorder: a row
                        // keeps its mirror when the order changes, so a result
                        // that arrives mid-sort still lands on the right row.
                        // Last column: the number is what the sweep produces,
                        // and the row's own facts come first.
                        //
                        // A label with a mouse area, like the package table's
                        // sortable heading. Not a button: the style paints a tool
                        // button's text inside its own background, where no
                        // palette role reaches it (the heading came out bright
                        // while the others are muted), draws a frame around a
                        // word that is a heading, and its box is a pixel taller
                        // than a label, which lifted this heading off the line
                        // the others sit on.
                        QQC2.Label {
                            Layout.preferredWidth: view.latencyWidth
                            Layout.minimumWidth: view.latencyWidth
                            Layout.maximumWidth: view.latencyWidth
                            // "Ping", not "Latency": the column is the round
                            // trip the probe measures, and `ping` is the command
                            // that measures it. A command name, so not translated.
                            text: "Ping "
                                  + (view.mirrorBackend.model.sortDescending ? "\u25BC" : "\u25B2")
                            font.bold: true
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideRight
                            Accessible.role: Accessible.Button
                            Accessible.name: view.mirrorBackend.model.sortDescending
                                             ? i18nd("kxbps", "Latency, sorted slowest first")
                                             : i18nd("kxbps", "Latency, sorted fastest first")

                            MouseArea {
                                anchors.fill: parent
                                onClicked: view.mirrorBackend.model.sortDescending =
                                               !view.mirrorBackend.model.sortDescending
                            }
                        }
                    }
                }

                ListView {
                    id: mirrorList
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: mirrorHeader.bottom
                    anchors.bottom: parent.bottom
                    clip: true
                    model: view.mirrorBackend.model
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                        id: mirrorBar
                    }

                    delegate: MirrorDelegate {
                        width: mirrorList.width - view.mirrorScrollBarSpace
                        mirrorBackend: view.mirrorBackend
                        columns: view
                    }
                }

                QQC2.Label {
                    anchors.centerIn: parent
                    // The list's own count, not the model's: `count` is the
                    // snapshot size and does not move with a filter.
                    visible: mirrorList.count === 0
                    text: view.mirrorBackend.model.filter === 0
                          && view.mirrorBackend.model.regionFilter.length === 0
                          ? i18nd("kxbps", "No mirrors.")
                          : i18nd("kxbps", "No mirror matches this filter.")
                    color: Kirigami.Theme.disabledTextColor
                }
            }

            // The table's footer: what the list shows, right under it. The
            // window bar carries the tool's state, so numbers live next to the
            // table that produced them — the same place the package table puts
            // them.
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

            // A refused selection: the URL is not one the list offers or the
            // helper would take. The backend names the key; it stays untranslated
            // nowhere, so it is mapped here like every other key.
            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                visible: view.mirrorBackend.currentMirrorError.length > 0
                text: messageText(view.mirrorBackend.currentMirrorError, [])
                color: Kirigami.Theme.negativeTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
            }

            // The configuration itself could not be read: an unreadable file is
            // a different claim from "the selection is local only", so it gets
            // its own line instead of a guessed host.
            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                visible: view.mirrorBackend.systemMirrorError.length > 0
                text: messageText(view.mirrorBackend.systemMirrorError,
                                  view.mirrorBackend.systemMirrorErrorArgs)
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.WordWrap
            }
        }

    }

    // The table's own footer: the count of what the list shows, or the sweep's
    // progress while it runs. Same wording shape as the package table's.
    function footerText() {
        if (view.mirrorBackend.probing) {
            // The batch covers the visible rows, so the total is the list's
            // count, not the snapshot size.
            return i18nd("kxbps", "%1 of %2 mirrors probed",
                         view.mirrorBackend.finishedCount, mirrorList.count)
        }
        return i18ndp("kxbps", "%1 mirror", "%1 mirrors", mirrorList.count)
    }

    // The backend reports failures as a translation key plus its arguments, so
    // the wording lives in the catalog and never in the C++ or the snapshot.
    // An unknown key is shown as-is: a blank line would hide a real failure.
    function messageText(key, args) {
        const values = args === undefined || args === null ? [] : args
        switch (key) {
        case "errorMirrorConfigRead":
            return values.length > 0 ? i18nd("kxbps", "Could not read the xbps configuration: %1.", values[0])
                                     : i18nd("kxbps", "Could not read the xbps configuration.")
        case "errorMirrorConfigParse":
            return i18nd("kxbps", "The xbps configuration could not be parsed.")
        case "errorMirrorWrite":
            return values.length > 0 ? i18nd("kxbps", "Could not write %1.", values[0])
                                     : i18nd("kxbps", "Could not write the xbps configuration.")
        case "errorMirrorUrlRefused":
            return i18nd("kxbps", "The helper refused the URL.")
        }
        return key
    }
}
