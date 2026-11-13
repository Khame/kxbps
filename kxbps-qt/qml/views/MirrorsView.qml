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
import "../components"

Item {
    id: view

    property var mirrorBackend

    readonly property real selectionWidth: Kirigami.Units.gridUnit * 2
    readonly property real regionWidth: Kirigami.Units.gridUnit * 6
    // The heading is a word, the values are numbers: at five grid units
    // the Polish "Opóźnienie" was cut in half and the column drifted
    // from the rows, which use this same width.
    readonly property real latencyWidth: Kirigami.Units.gridUnit * 7
    readonly property real stateWidth: Kirigami.Units.gridUnit * 16
    // The strip the scrollbar takes when it is visible. Rows and the header end
    // before it, so the state badge is never cut by the bar.
    readonly property real mirrorScrollBarSpace: mirrorBar.visible ? mirrorBar.width : 0

    function parseRepos(json) {
        try {
            const parsed = JSON.parse(json)
            return Array.isArray(parsed) ? parsed : []
        } catch (error) {
            return []
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

    onMirrorBackendChanged: repos = parseRepos(mirrorBackend ? mirrorBackend.systemRepos : "[]")
    Component.onCompleted: {
        filterLabels = [
            i18nd("kxbps", "All"),
            i18nd("kxbps", "OK"),
            i18nd("kxbps", "Failed")
        ]
        repos = parseRepos(mirrorBackend ? mirrorBackend.systemRepos : "[]")
    }

    Connections {
        target: view.mirrorBackend
        function onSystemReposChanged() {
            view.repos = view.parseRepos(view.mirrorBackend.systemRepos)
        }
        // The first frame can bind before the backend's own read has answered;
        // asking here keeps the tab from sitting on "No repositories." when a
        // value is already in hand.
        function onSystemLoadingChanged() {
            if (!view.mirrorBackend.systemLoading) {
                view.repos = view.parseRepos(view.mirrorBackend.systemRepos)
            }
        }
    }

    QQC2.TabBar {
        id: tabs
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top

        QQC2.TabButton {
            text: i18nd("kxbps", "Mirrors")
        }
        QQC2.TabButton {
            text: i18nd("kxbps", "Repositories")
        }
    }

    StackLayout {
        anchors.top: tabs.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        currentIndex: tabs.currentIndex

        ColumnLayout {
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.topMargin: Kirigami.Units.smallSpacing
                spacing: Kirigami.Units.smallSpacing

                QQC2.Label {
                    Layout.fillWidth: true
                    visible: view.mirrorBackend.probing
                    text: i18nd("kxbps", "%1 of %2 mirrors probed",
                                view.mirrorBackend.finishedCount, view.mirrorBackend.model.count)
                    color: Kirigami.Theme.disabledTextColor
                }

                QQC2.Label {
                    text: i18nd("kxbps", "Show")
                    color: Kirigami.Theme.disabledTextColor
                }

                QQC2.ComboBox {
                    id: filterCombo
                    model: view.filterLabels
                    // The model's `filter` is the truth; the combo only writes
                    // it and reads the refusal back, so a value the model
                    // clamps to `0` moves the combo with it.
                    currentIndex: view.mirrorBackend.model.filter
                    Accessible.name: i18nd("kxbps", "Mirror state filter")
                    onActivated: view.mirrorBackend.model.filter = currentIndex
                }
            }

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
                        anchors.leftMargin: Kirigami.Units.largeSpacing
                        anchors.rightMargin: Kirigami.Units.largeSpacing
                                                     + view.mirrorScrollBarSpace
                        spacing: Kirigami.Units.largeSpacing

                        Item {
                            Layout.preferredWidth: view.selectionWidth
                            Layout.minimumWidth: view.selectionWidth
                            Layout.maximumWidth: view.selectionWidth
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: i18nd("kxbps", "Server")
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

                        // The sort is the model's, not a QML reorder: a row
                        // keeps its mirror when the order changes, so a
                        // result that arrives mid-sort still lands on the
                        // right row.
                        QQC2.ToolButton {
                            Layout.preferredWidth: view.latencyWidth
                            Layout.minimumWidth: view.latencyWidth
                            Layout.maximumWidth: view.latencyWidth
                            padding: 0
                            display: QQC2.AbstractButton.TextOnly
                            text: i18nd("kxbps", "Latency") + " "
                                  + (view.mirrorBackend.model.sortDescending ? "\u25BC" : "\u25B2")
                            font.bold: true
                            checkable: false
                            Accessible.name: view.mirrorBackend.model.sortDescending
                                             ? i18nd("kxbps", "Latency, sorted slowest first")
                                             : i18nd("kxbps", "Latency, sorted fastest first")
                            Accessible.role: Accessible.Button
                            onClicked: view.mirrorBackend.model.sortDescending =
                                           !view.mirrorBackend.model.sortDescending
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
                    }
                }

                QQC2.Label {
                    anchors.centerIn: parent
                    visible: view.mirrorBackend.model.count === 0
                    text: view.mirrorBackend.model.filter === 0
                          ? i18nd("kxbps", "No mirrors.")
                          : i18nd("kxbps", "No mirror matches this filter.")
                    color: Kirigami.Theme.disabledTextColor
                }
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

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                text: i18nd("kxbps", "Saved in %1", "~/.local/share/kxbps/mirror.json")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }

            // What xbps actually uses. The sentence is only the "in use" claim
            // while the saved selection and the system agree and no read is in
            // flight; in every other state the truth is that the selection is
            // local only, and the footer says that with the host it will be
            // applied to.
            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                visible: view.mirrorBackend.systemMirrorError.length === 0
                text: {
                    const backend = view.mirrorBackend
                    if (!backend.pending && !backend.systemLoading && backend.systemMirror.length > 0) {
                        return i18nd("kxbps", "xbps uses %1, from %2.",
                                     backend.systemMirror, "/etc/xbps.d/00-repository-main.conf")
                    }
                    return i18nd("kxbps", "The selection is saved for KXbps only; xbps uses %1.",
                                 view.hostOf(backend.systemMirror.length > 0
                                             ? backend.systemMirror
                                             : backend.currentMirror))
                }
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
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

        ColumnLayout {
            id: repositoriesTab

            spacing: 0

            readonly property var rows: view.sortedRepos(view.repos)

            QQC2.Label {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.largeSpacing
                visible: view.mirrorBackend.systemLoading
                text: i18nd("kxbps", "Reading repositories…")
                color: Kirigami.Theme.disabledTextColor
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.largeSpacing
                visible: !view.mirrorBackend.systemLoading
                         && view.mirrorBackend.systemMirrorError.length > 0
                text: messageText(view.mirrorBackend.systemMirrorError,
                                  view.mirrorBackend.systemMirrorErrorArgs)
                color: Kirigami.Theme.negativeTextColor
                wrapMode: Text.WordWrap
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.largeSpacing
                visible: !view.mirrorBackend.systemLoading
                         && view.mirrorBackend.systemMirrorError.length === 0
                         && repositoriesTab.rows.length === 0
                text: i18nd("kxbps", "No repositories.")
                color: Kirigami.Theme.disabledTextColor
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                ListView {
                    id: repoList
                    anchors.fill: parent
                    clip: true
                    visible: repositoriesTab.rows.length > 0
                    model: repositoriesTab.rows
                    QQC2.ScrollBar.vertical: QQC2.ScrollBar {}
                    QQC2.ScrollBar.horizontal: QQC2.ScrollBar {}

                    header: Item {
                        width: repoList.width
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
                            anchors.leftMargin: Kirigami.Units.largeSpacing
                            anchors.rightMargin: Kirigami.Units.largeSpacing
                            spacing: Kirigami.Units.largeSpacing

                            QQC2.Label {
                                Layout.fillWidth: true
                                Layout.minimumWidth: 0
                                text: i18nd("kxbps", "Repository")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.countWidth
                                Layout.minimumWidth: view.countWidth
                                Layout.maximumWidth: view.countWidth
                                horizontalAlignment: Text.AlignRight
                                text: i18nd("kxbps", "Packages")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.signatureWidth
                                Layout.minimumWidth: view.signatureWidth
                                Layout.maximumWidth: view.signatureWidth
                                text: i18nd("kxbps", "Signature")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.fileWidth
                                Layout.minimumWidth: view.fileWidth
                                Layout.maximumWidth: view.fileWidth
                                text: i18nd("kxbps", "File")
                                font.bold: true
                                color: Kirigami.Theme.disabledTextColor
                            }
                        }
                    }

                    delegate: QQC2.ItemDelegate {
                        id: repoRow

                        required property var modelData

                        width: repoList.width
                        height: Kirigami.Units.gridUnit * 2.4

                        // A row is a record, not a link: nothing in this build
                        // enables or disables a repository from here, so the row
                        // does not pretend to be clickable.
                        enabled: false
                        Accessible.name: modelData.url

                        contentItem: RowLayout {
                            spacing: Kirigami.Units.largeSpacing

                            QQC2.Label {
                                Layout.fillWidth: true
                                Layout.minimumWidth: 0
                                text: modelData.url
                                font.family: "monospace"
                                elide: Text.ElideMiddle
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.countWidth
                                Layout.minimumWidth: view.countWidth
                                Layout.maximumWidth: view.countWidth
                                horizontalAlignment: Text.AlignRight
                                // `-1` is "xbps could not read this index",
                                // which a "0" would misreport as an empty repo.
                                text: modelData.count < 0 ? i18nd("kxbps", "No index")
                                                          : String(modelData.count)
                                font.family: "monospace"
                                color: modelData.count < 0 ? Kirigami.Theme.disabledTextColor
                                                           : Kirigami.Theme.textColor
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.signatureWidth
                                Layout.minimumWidth: view.signatureWidth
                                Layout.maximumWidth: view.signatureWidth
                                // `signed` is "verified with a local key". A
                                // maybe-signed index parses as `false` in the
                                // core, so the label must not claim "unsigned".
                                text: modelData.signed ? i18nd("kxbps", "Signed")
                                                       : i18nd("kxbps", "Not verified")
                                color: modelData.signed ? Kirigami.Theme.positiveTextColor
                                                        : Kirigami.Theme.disabledTextColor
                                elide: Text.ElideRight
                            }

                            QQC2.Label {
                                Layout.preferredWidth: view.fileWidth
                                Layout.minimumWidth: view.fileWidth
                                Layout.maximumWidth: view.fileWidth
                                // The path is data, never translated.
                                text: modelData.file.length > 0 ? modelData.file : "-"
                                font.family: "monospace"
                                color: Kirigami.Theme.disabledTextColor
                                elide: Text.ElideLeft
                            }
                        }
                    }
                }
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                text: i18nd("kxbps", "Read from xbps-query -L and the xbps.d files. Changing a repository is a later task.")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
                wrapMode: Text.WordWrap
            }
        }
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
