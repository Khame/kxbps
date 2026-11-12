// `xbps-query` by hand: a mode, a pattern, and the matching packages.
//
// The mode tokens are command syntax and stay untranslated; only their
// accessible name and the hint line are copy. Modes that list a set (`-l`,
// `-m`, `-O`, `-H`) cannot be resolved by the catalog alone, so the result set
// is narrowed here against the catalog snapshot.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var catalogBackend
    property var catalogIndex: []

    property var results: []
    readonly property int resultCount: results.length

    readonly property real versionWidth: Kirigami.Units.gridUnit * 9.5
    readonly property real sizeWidth: Kirigami.Units.gridUnit * 4.5

    // The fourteen modes the core knows. `-Rs` is the default, like the web app.
    readonly property var modes: ["Rs", "s", "l", "R", "show", "x", "X", "f", "o", "m", "O", "H", "L", "p"]

    // These take no pattern: they list a set, and the catalog cannot know what
    // is installed.
    readonly property var listModes: ["l", "m", "O", "H", "L"]

    function isListMode(mode) {
        return view.listModes.indexOf(mode) >= 0
    }

    function dependsOf(pkg) {
        return pkg.depends.length > 0 ? pkg.depends.split(", ") : []
    }

    function isOrphan(pkg) {
        if (pkg.installedVersion === "" || pkg.explicit) {
            return false
        }
        for (let i = 0; i < view.catalogIndex.length; ++i) {
            const other = view.catalogIndex[i]
            if (other.installedVersion !== "" && view.dependsOf(other).indexOf(pkg.name) >= 0) {
                return false
            }
        }
        return true
    }

    function narrow(mode, hits) {
        switch (mode) {
        case "s":
        case "l":
            return hits.filter(pkg => pkg.installedVersion !== "")
        case "m":
            return hits.filter(pkg => pkg.explicit)
        case "H":
            return hits.filter(pkg => pkg.held)
        case "O":
            return hits.filter(pkg => view.isOrphan(pkg))
        }
        return hits
    }

    function run() {
        const mode = modeCombo.currentText
        const query = searchField.text
        // `-L` lists repositories; the catalog only knows packages, so this mode
        // has no honest result set to show yet.
        if (mode === "L") {
            view.results = []
            return
        }
        if (!view.isListMode(mode) && query.length === 0) {
            view.results = []
            return
        }
        view.results = view.narrow(mode, view.catalogBackend.search(query, mode, ""))
    }

    function hint() {
        const mode = modeCombo.currentText
        const query = searchField.text
        if (mode === "L") {
            return i18nd("khaxbps", "xbps-query -L lists repositories; this app does not implement it yet.")
        }
        if (query.length === 0 && view.isListMode(mode)) {
            return i18nd("khaxbps", "Equivalent: %1", "xbps-query -" + mode)
        }
        return i18nd("khaxbps", "Equivalent: %1", "xbps-query -" + mode + " " + query)
    }

    function emptyText() {
        if (modeCombo.currentText === "L") {
            // The hint above already explains the mode; a second sentence would
            // just repeat it.
            return ""
        }
        if (view.isListMode(modeCombo.currentText) || searchField.text.length > 0) {
            return i18nd("khaxbps", "No matches.")
        }
        return i18nd("khaxbps", "Type a query and choose a mode.")
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.ComboBox {
                id: modeCombo
                model: view.modes
                currentIndex: 0
                Accessible.name: i18nd("khaxbps", "Query mode")
                onActivated: view.run()
            }

            Kirigami.SearchField {
                id: searchField
                Layout.fillWidth: true
                placeholderText: i18nd("khaxbps", "Query…")
                onAccepted: view.run()
                onTextChanged: debounce.restart()
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            text: view.hint()
            color: Kirigami.Theme.disabledTextColor
            font.family: "monospace"
            elide: Text.ElideRight
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            ListView {
                id: resultsList
                anchors.fill: parent
                clip: true
                model: view.results
                QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

                delegate: QueryDelegate {
                    width: resultsList.width
                    versionWidth: view.versionWidth
                    sizeWidth: view.sizeWidth
                }
            }

            QQC2.Label {
                anchors.centerIn: parent
                visible: view.resultCount === 0 && text.length > 0
                text: view.emptyText()
                color: Kirigami.Theme.disabledTextColor
            }
        }
    }

    // Typing runs the query after the pattern settles; `accepted` runs it at
    // once, so Enter does not wait for the timer.
    Timer {
        id: debounce
        interval: 200
        onTriggered: view.run()
    }

    Component.onCompleted: view.run()
}
