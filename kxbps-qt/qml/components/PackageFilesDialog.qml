// The two `xbps-query` questions the details pane had nowhere to ask.
//
// The pane already shows the facts, the dependencies and the reverse
// dependencies of the selected package. What it cannot show is the list of
// files a package owns (`-f`) and which package owns a given path (`-o`): one
// is too long for the pane, the other is not about the selected package at all.
// Both live here, behind the pane's "More" menu, in one compact dialog with a
// tab each.
//
// The dialog is sized in grid units and capped against the window, so it never
// covers the whole screen and scales with the font. Every answer comes from the
// backend's worker thread; nothing here invents a row while the read is in
// flight.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

QQC2.Dialog {
    id: dialog

    property var catalogBackend
    // The selected package as `name-version_revision`; empty disables the files
    // tab, because `-f` needs a package to ask about.
    property string packageName: ""
    // The name without the version, for the tab title.
    property string packageLabel: ""

    // The rows the filter lets through. Recomputed when the backend answers or
    // the filter changes, never on every `ListView` rebind: a package like
    // linux-firmware has thousands of paths.
    property var shownFiles: []

    title: i18nd("kxbps", "Package files and owner")
    modal: true
    closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
    // Centred over the window, not over the pane: the pane is short and the
    // dialog would inherit its height.
    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    // Grid units, capped against the window: about half of it on a normal
    // screen, never the whole of it.
    width: Math.min(Kirigami.Units.gridUnit * 46,
                    parent ? parent.width - Kirigami.Units.gridUnit * 6
                           : Kirigami.Units.gridUnit * 46)
    height: Math.min(Kirigami.Units.gridUnit * 34,
                     parent ? parent.height - Kirigami.Units.gridUnit * 6
                            : Kirigami.Units.gridUnit * 34)
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
            return i18nd("kxbps", "%1 failed with exit code %2: %3", a, b, c)
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

    // Opens on the files tab and reads them again: the list is as old as the
    // last time this dialog asked, and the package may have moved since.
    function openFiles() {
        tabBar.currentIndex = 0
        dialog.refilter()
        if (dialog.packageName.length > 0) {
            dialog.catalogBackend.readPackageFiles(dialog.packageName)
        }
        dialog.open()
    }

    // Opens on the owner tab; the path field keeps what was typed last, because
    // looking up a second path next to the first is the usual reason to come
    // back.
    function openOwner() {
        tabBar.currentIndex = 1
        dialog.open()
    }

    onOpened: {
        if (tabBar.currentIndex === 0) {
            dialog.refilter()
            filterField.forceActiveFocus()
        } else {
            pathField.forceActiveFocus()
        }
    }

    Connections {
        target: dialog.catalogBackend
        function onFilesChanged() {
            dialog.refilter()
        }
    }

    contentItem: ColumnLayout {
        spacing: Kirigami.Units.smallSpacing

        QQC2.TabBar {
            id: tabBar
            Layout.fillWidth: true
            Layout.minimumWidth: 0

            QQC2.TabButton {
                text: i18nd("kxbps", "Files")
                enabled: dialog.packageName.length > 0
                Accessible.name: i18nd("kxbps", "Files of the package")
            }

            QQC2.TabButton {
                text: i18nd("kxbps", "Owner")
                Accessible.name: i18nd("kxbps", "Owner of a file")
            }
        }

        // Files: one path per row, filtered by the field above the list.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            visible: tabBar.currentIndex === 0
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
                    text: dialog.packageName.length === 0
                          ? i18nd("kxbps", "Select a package first.")
                          : (filterField.text.length > 0
                             ? i18nd("kxbps", "No path matches the filter.")
                             : i18nd("kxbps", "The package owns no files."))
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

        // Owner: a path in, the package that owns it out.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumWidth: 0
            visible: tabBar.currentIndex === 1
            spacing: Kirigami.Units.smallSpacing

            QQC2.Label {
                Layout.fillWidth: true
                text: i18nd("kxbps", "Path")
                font: Kirigami.Theme.smallFont
                color: Kirigami.Theme.disabledTextColor
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.minimumWidth: 0
                spacing: Kirigami.Units.smallSpacing

                QQC2.TextField {
                    id: pathField
                    Layout.fillWidth: true
                    placeholderText: "/usr/bin/nvim"
                    Accessible.name: i18nd("kxbps", "Path to look up")
                    onAccepted: dialog.catalogBackend.readFileOwner(pathField.text)
                }

                QQC2.Button {
                    text: i18nd("kxbps", "Check")
                    enabled: pathField.text.trim().length > 0
                    onClicked: dialog.catalogBackend.readFileOwner(pathField.text)
                }
            }

            QQC2.BusyIndicator {
                Layout.alignment: Qt.AlignHCenter
                running: dialog.catalogBackend && dialog.catalogBackend.ownerLoading
                visible: running
            }

            QQC2.Label {
                Layout.fillWidth: true
                visible: dialog.catalogBackend && dialog.catalogBackend.ownerError.length > 0
                text: visible
                      ? dialog.errorText(dialog.catalogBackend.ownerError,
                                         dialog.catalogBackend.ownerErrorArgs)
                      : ""
                color: Kirigami.Theme.negativeTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
            }

            QQC2.Label {
                Layout.fillWidth: true
                visible: dialog.catalogBackend && dialog.catalogBackend.owner.length > 0
                text: visible ? dialog.catalogBackend.owner : ""
                font.family: "monospace"
                textFormat: Text.PlainText
                wrapMode: Text.WrapAnywhere
            }

            QQC2.Label {
                Layout.fillWidth: true
                visible: dialog.catalogBackend
                         && dialog.catalogBackend.owner.length === 0
                         && dialog.catalogBackend.ownerError.length === 0
                         && !dialog.catalogBackend.ownerLoading
                text: i18nd("kxbps", "Type a path and press Check.")
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.WordWrap
            }

            Item {
                Layout.fillHeight: true
            }
        }
    }

    footer: RowLayout {
        spacing: Kirigami.Units.smallSpacing

        Item {
            Layout.fillWidth: true
        }

        QQC2.Button {
            text: i18nd("kxbps", "Close")
            onClicked: dialog.close()
        }
    }
}
