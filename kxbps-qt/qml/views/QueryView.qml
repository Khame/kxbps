// `xbps-query` by hand: a mode, a pattern, and what xbps answers.
//
// The mode tokens are command syntax and stay untranslated; only their
// accessible name and the hint line are copy.
//
// The answer comes from xbps itself: `catalogBackend.search()` starts one
// `xbps-query` take on a worker thread and `resultsRevision` says when a new
// answer can be read. The listing modes (`-Rs`, `-s`, `-l`, `-m`, `-O`, `-H`)
// answer with package rows. Every other mode prints something that is not a
// package list (repositories, dependencies, properties), and a row built from
// one of those lines would be invented, so the raw output is shown verbatim in
// a monospace block under the command that produced it.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var catalogBackend

    // The newest answer, in the role-keyed maps the backend hands out.
    property var results: []
    readonly property int resultCount: results.length
    readonly property string errorKey: catalogBackend.searchError
    readonly property var errorArgs: catalogBackend.searchErrorArgs
    // The text of a mode that answers with output rather than rows.
    readonly property string rawText: catalogBackend.rawText
    readonly property string rawCommand: catalogBackend.rawCommand

    // The selection, by pkgver: one package, read on demand.
    property string selectedPkgver: ""

    readonly property real versionWidth: Kirigami.Units.gridUnit * 9.5

    // The fourteen modes the core knows. `-Rs` is the default, like the web app.
    readonly property var modes: ["Rs", "s", "l", "R", "show", "x", "X", "f", "o", "m", "O", "H", "L", "p"]

    // Modes whose answer is a package list; the rest print raw output.
    readonly property var listModes: ["Rs", "s", "l", "m", "O", "H"]

    function isListMode(mode) {
        return view.listModes.indexOf(mode) >= 0
    }

    function run() {
        const mode = modeCombo.currentText
        const query = searchField.text.trim()
        view.selectedPkgver = ""
        // A listing mode needs a pattern; typing one is the next thing the
        // user does, so an empty field clears instead of reading everything.
        if (view.isListMode(mode) && query.length === 0) {
            view.results = []
            return
        }
        view.catalogBackend.search(query, mode)
    }

    function reloadResults() {
        view.results = view.catalogBackend.results()
    }

    function hint() {
        const mode = modeCombo.currentText
        const query = searchField.text
        return i18nd("kxbps", "Equivalent: %1", ("xbps-query -" + mode + " " + query).trim())
    }

    function emptyText() {
        if (view.errorKey.length > 0 || view.catalogBackend.searching) {
            return ""
        }
        if (view.isListMode(modeCombo.currentText) && searchField.text.length === 0) {
            return i18nd("kxbps", "Type a query.")
        }
        if (!view.isListMode(modeCombo.currentText) && view.rawText.length === 0) {
            // The take answered nothing: say so, rather than a blank block.
            return i18nd("kxbps", "xbps printed nothing.")
        }
        if (view.isListMode(modeCombo.currentText)) {
            return i18nd("kxbps", "No matches.")
        }
        return ""
    }

    // The one sentence for a failed take, chosen by the key the backend sent.
    // The arguments are already ordered for the sentence's `%1…%3`.
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
        return i18nd("kxbps", "Could not run the query: %1", a)
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
                Accessible.name: i18nd("kxbps", "Query mode")
                onActivated: {
                    debounce.stop()
                    view.run()
                }
            }

            Kirigami.SearchField {
                id: searchField
                Layout.fillWidth: true
                placeholderText: i18nd("kxbps", "Query…")
                onAccepted: {
                    debounce.stop()
                    view.run()
                }
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

        // A failed take is shown, never hidden behind an empty list.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            visible: view.errorKey.length > 0
            text: visible ? view.errorText(view.errorKey, view.errorArgs) : ""
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }

        // The listing modes: one row per package xbps printed.
        ListView {
            id: resultsList
            Layout.fillWidth: true
            Layout.fillHeight: view.isListMode(modeCombo.currentText)
            visible: view.isListMode(modeCombo.currentText)
            clip: true
            model: view.results
            QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

            delegate: QueryDelegate {
                width: resultsList.width
                versionWidth: view.versionWidth
                current: view.selectedPkgver === modelData.pkgver
                onClicked: {
                    view.selectedPkgver = modelData.pkgver
                    view.catalogBackend.requestFacts(modelData.pkgver)
                }
            }

            QQC2.Label {
                anchors.centerIn: parent
                visible: view.resultCount === 0 && text.length > 0
                text: view.emptyText()
                color: Kirigami.Theme.disabledTextColor
            }
        }

        // Every other mode: the output as xbps printed it.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: !view.isListMode(modeCombo.currentText)
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            visible: !view.isListMode(modeCombo.currentText)
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                Layout.fillWidth: true
                visible: view.rawCommand.length > 0
                text: i18nd("kxbps", "Output of %1", view.rawCommand)
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                elide: Text.ElideRight
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                Flickable {
                    id: rawView
                    anchors.fill: parent
                    clip: true
                    visible: view.rawText.length > 0
                    contentWidth: width
                    contentHeight: rawLabel.implicitHeight
                    boundsBehavior: Flickable.StopAtBounds

                    QQC2.Label {
                        id: rawLabel
                        width: rawView.width
                        text: view.rawText
                        font.family: "monospace"
                        textFormat: Text.PlainText
                        wrapMode: Text.WrapAnywhere
                    }
                }

                QQC2.Label {
                    anchors.centerIn: parent
                    visible: view.rawText.length === 0 && text.length > 0
                    text: view.emptyText()
                    color: Kirigami.Theme.disabledTextColor
                }
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            visible: view.isListMode(modeCombo.currentText) && view.resultCount > 0
            text: i18ndp("kxbps", "%1 result", "%1 results", view.resultCount)
            color: Kirigami.Theme.disabledTextColor
            font: Kirigami.Theme.smallFont
            elide: Text.ElideRight
        }
    }

    // Typing runs the query after the pattern settles; Enter runs it at once,
    // so it does not wait for the timer.
    Timer {
        id: debounce
        interval: 250
        onTriggered: view.run()
    }

    Connections {
        target: view.catalogBackend
        function onResultsRevisionChanged() {
            view.reloadResults()
        }
        // A fresh answer with hits identical to the previous one does not bump
        // the revision; the error state still has to leave the banner.
        function onSearchErrorChanged() {
            view.results = view.catalogBackend.results()
        }
    }

    Component.onCompleted: view.run()
}
