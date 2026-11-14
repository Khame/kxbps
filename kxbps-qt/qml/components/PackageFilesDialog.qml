// The list of files one package owns.
//
// `xbps-query -f` answers from the installed database and `-R -f` from the
// repository index on disk, so the list exists for a package that is not
// installed too, without a network round trip. The details pane has no room
// for a list this long — some packages own thousands of paths — so it opens
// here.
//
// The dialog is sized in grid units and capped against the window: it takes
// most of the window, never all of it, and scales with the font. Every answer
// comes from the backend's worker thread; nothing here invents a row while the
// read is in flight.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

QQC2.Dialog {
    id: dialog

    property var catalogBackend
    // The selected package as `name-version_revision`.
    property string packageName: ""
    // The name without the version, for the heading.
    property string packageLabel: ""

    // The rows the filter lets through. Recomputed when the backend answers or
    // the filter changes, never on every `ListView` rebind: a package like
    // linux-firmware has thousands of paths.
    property var shownFiles: []

    title: i18nd("kxbps", "Package files")
    modal: true
    closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
    // Centred over the window, not over the pane: the pane is short and the
    // dialog would inherit its height.
    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    // Most of the window: the list is what the dialog is for. Still capped, so
    // the window behind stays visible and this never becomes a second window.
    width: Math.min(Kirigami.Units.gridUnit * 100,
                    parent ? parent.width * 0.9 : Kirigami.Units.gridUnit * 100)
    height: Math.min(Kirigami.Units.gridUnit * 48,
                     parent ? parent.height * 0.85 : Kirigami.Units.gridUnit * 48)
    padding: Kirigami.Units.largeSpacing

    // The one sentence for a failed read, chosen by the key the backend sent.
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
        return i18nd("kxbps", "Could not run the query: %1", a)
    }

    function refilter() {
        const source = dialog.catalogBackend ? dialog.catalogBackend.files : []
        const needle = filterField.text.trim().toLowerCase()
        if (needle.length === 0) {
            dialog.shownFiles = source
            return
        }
        const kept = []
        for (let i = 0; i < source.length; ++i) {
            if (source[i].toLowerCase().indexOf(needle) >= 0) {
                kept.push(source[i])
            }
        }
        dialog.shownFiles = kept
    }

    // Reading on open, not on construction: the dialog is built with the pane
    // and the package changes while it is closed.
    onOpened: {
        dialog.refilter()
        if (dialog.packageName.length > 0) {
            dialog.catalogBackend.readPackageFiles(dialog.packageName)
        }
        filterField.forceActiveFocus()
    }

    Connections {
        target: dialog.catalogBackend
        function onFilesChanged() {
            dialog.refilter()
        }
    }

    contentItem: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing

        QQC2.Label {
            Layout.fillWidth: true
            Layout.minimumWidth: 0
            text: dialog.packageLabel.length > 0
                  ? i18nd("kxbps", "Files owned by %1", dialog.packageLabel)
                  : i18nd("kxbps", "No package selected.")
            color: Kirigami.Theme.disabledTextColor
            font: Kirigami.Theme.smallFont
            elide: Text.ElideRight
        }

        Kirigami.SearchField {
            id: filterField
            Layout.fillWidth: true
            placeholderText: i18nd("kxbps", "Filter paths…")
            onTextChanged: dialog.refilter()
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: dialog.catalogBackend && dialog.catalogBackend.filesError.length > 0
            text: visible
                  ? dialog.errorText(dialog.catalogBackend.filesError,
                                     dialog.catalogBackend.filesErrorArgs)
                  : ""
            color: Kirigami.Theme.negativeTextColor
            font: Kirigami.Theme.smallFont
            wrapMode: Text.WordWrap
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            QQC2.BusyIndicator {
                anchors.centerIn: parent
                running: dialog.catalogBackend && dialog.catalogBackend.filesLoading
                visible: running
            }

            ListView {
                id: fileList
                anchors.fill: parent
                clip: true
                visible: !(dialog.catalogBackend && dialog.catalogBackend.filesLoading)
                         && dialog.shownFiles.length > 0
                model: dialog.shownFiles
                boundsBehavior: Flickable.StopAtBounds
                QQC2.ScrollBar.vertical: QQC2.ScrollBar {}

                delegate: QQC2.Label {
                    required property string modelData
                    width: fileList.width - Kirigami.Units.smallSpacing
                    text: modelData
                    font.family: "monospace"
                    textFormat: Text.PlainText
                    elide: Text.ElideMiddle
                }
            }

            QQC2.Label {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.largeSpacing * 2
                visible: !(dialog.catalogBackend && dialog.catalogBackend.filesLoading)
                         && dialog.catalogBackend
                         && dialog.catalogBackend.filesError.length === 0
                         && dialog.shownFiles.length === 0
                text: filterField.text.length > 0
                      ? i18nd("kxbps", "No path matches the filter.")
                      : i18nd("kxbps", "The package owns no files.")
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            visible: !!dialog.catalogBackend
            text: {
                if (!dialog.catalogBackend) {
                    return ""
                }
                const total = dialog.catalogBackend.files.length
                const shown = dialog.shownFiles.length
                if (filterField.text.trim().length > 0) {
                    return i18nd("kxbps", "%1 of %2 paths shown", shown, total)
                }
                return i18ndp("kxbps", "%1 path", "%1 paths", total)
            }
            color: Kirigami.Theme.disabledTextColor
            font: Kirigami.Theme.smallFont
        }
    }

    footer: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        Item {
            Layout.fillWidth: true
        }

        DenseButton {
            text: i18nd("kxbps", "Close")
            onClicked: dialog.close()
        }
    }
}
