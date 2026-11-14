// `xbps-query` for the questions no other page answers.
//
// The package list already shows what is installed, upgradable, marked,
// explicit, orphaned or held; "Dostępne" lists and searches every package in
// the repositories; the Repositories page reads `xbps-query -L`; the details
// pane shows a selected package's facts. What is left is the diagnostic side:
// dependencies (`-x`), reverse dependencies (`-X`), owned files (`-f`), the
// owner of a file (`-o`) and a package's properties (`-p`).
//
// The tokens are command syntax and stay untranslated; the backend renders the
// same `QueryMode::ALL` list, so the selector cannot drift from the modes the
// core knows. Every one of these takes prints text rather than package rows,
// so the answer is shown verbatim in a monospace block under the command that
// produced it.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import "../components"

Item {
    id: view

    property var catalogBackend

    readonly property string errorKey: catalogBackend.searchError
    readonly property var errorArgs: catalogBackend.searchErrorArgs
    // The text of the newest take and the command it came from.
    readonly property string rawText: catalogBackend.rawText
    readonly property string rawCommand: catalogBackend.rawCommand
    readonly property bool searching: catalogBackend.searching

    // The tokens the core offers, in its order: `x`, `X`, `f`, `o`, `p`.
    readonly property var modes: catalogBackend.queryModes
    readonly property string modeToken: modeCombo.currentIndex >= 0
                                          && modeCombo.currentIndex < view.modes.length
                                          ? view.modes[modeCombo.currentIndex]
                                          : ""

    // One take: the backend builds the argv from the token and the text, so
    // the `-p` split lives in Rust and not here.
    function run() {
        if (view.modeToken.length === 0) {
            return
        }
        view.catalogBackend.search(searchField.text.trim(), view.modeToken)
    }

    function hint() {
        if (view.modeToken.length === 0) {
            return ""
        }
        return i18nd("kxbps", "Equivalent: %1",
                     ("xbps-query -" + view.modeToken + " " + searchField.text).trim())
    }

    function emptyText() {
        if (view.errorKey.length > 0 || view.searching) {
            return ""
        }
        if (searchField.text.trim().length === 0) {
            return i18nd("kxbps", "Type a query.")
        }
        // The take answered nothing: say so, rather than a blank block.
        return i18nd("kxbps", "xbps printed nothing.")
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

        // A failed take is shown, never hidden behind an empty block.
        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            visible: view.errorKey.length > 0
            text: visible ? view.errorText(view.errorKey, view.errorArgs) : ""
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }

        // The answer as xbps printed it, under the command it came from.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
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
    }

    // Typing runs the take after the text settles; Enter runs it at once, so it
    // does not wait for the timer.
    Timer {
        id: debounce
        interval: 250
        onTriggered: view.run()
    }

    Component.onCompleted: view.run()
}
