// The repositories xbps reads, and the two edits this tool owns.
//
// The list is the system's: `/etc/xbps.d` plus `xbps-query -L`, read through the
// mirror backend. Only a file this tool wrote (`20-kxbps-*.conf`) can be removed
// here — the helper refuses anything else and the button is hidden for the
// others, so a repository written by hand cannot be deleted by a stray click.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Item {
    id: view

    property var mirrorBackend

    // The job the window has to keep, like every other transaction: the backend
    // releases the previous one when it hands over the next.
    signal jobStarted(var startedJob)

    readonly property var repos: parseRepos(mirrorBackend ? mirrorBackend.systemRepos : "[]")
    readonly property bool busy: mirrorBackend !== null && mirrorBackend.applyBusy
    readonly property bool ready: mirrorBackend !== null && mirrorBackend.repoHelperReady
    property string newUrl: ""

    function parseRepos(json) {
        try {
            const parsed = JSON.parse(json)
            return Array.isArray(parsed) ? parsed : []
        } catch (error) {
            return []
        }
    }

    // A file this tool wrote, named by the helper's own prefix.
    function isManaged(repo) {
        return typeof repo.file === "string" && repo.file.indexOf("20-kxbps-") === 0
    }

    function add() {
        if (view.mirrorBackend === null || view.newUrl.trim().length === 0) {
            return
        }
        const job = view.mirrorBackend.addRepository(view.newUrl.trim())
        if (job !== null) {
            view.newUrl = ""
            view.jobStarted(job)
        }
    }

    function remove(repo) {
        if (view.mirrorBackend === null || !view.isManaged(repo)) {
            return
        }
        const job = view.mirrorBackend.removeRepository(repo.file)
        if (job !== null) {
            view.jobStarted(job)
        }
    }

    // The one sentence for a missing privileged half, chosen by the key the
    // backend sent. A disabled button with no reason is the bug this names.
    function helperText(key) {
        switch (key) {
        case "errorMirrorHelperMissing":
            return i18nd("kxbps", "Adding and removing repositories needs %1, which packaging/install.sh installs.",
                         "/usr/libexec/kxbps-repo-apply")
        case "errorMirrorHelperNotExecutable":
            return i18nd("kxbps", "The repository helper at %1 is not executable.", "/usr/libexec/kxbps-repo-apply")
        case "errorMirrorPolicyMissing":
            return i18nd("kxbps", "The polkit policy is missing, so the system cannot ask for a password: run packaging/install.sh.")
        }
        return ""
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // The add form: one URL, one button. The helper decides what is valid,
        // so the field does not pretend to parse what xbps accepts.
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.smallSpacing

            QQC2.TextField {
                id: urlField
                Layout.fillWidth: true
                // An example URL, not copy: a translated host would point the
                // user at a repository that does not exist.
                placeholderText: "https://repo.example.org/current"
                text: view.newUrl
                enabled: view.ready && !view.busy
                onTextChanged: view.newUrl = text
                onAccepted: view.add()
            }

            QQC2.Button {
                text: i18nd("kxbps", "Add repository")
                icon.name: "list-add"
                enabled: view.ready && !view.busy && view.newUrl.trim().length > 0
                onClicked: view.add()
            }
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "A repository added here is written to %1 and read by xbps for every user.",
                        "/etc/xbps.d/20-kxbps-…conf")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            visible: !view.ready && mirrorBackend !== null
                     && mirrorBackend.repoHelperErrorKey.length > 0
            text: view.helperText(mirrorBackend ? mirrorBackend.repoHelperErrorKey : "")
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            visible: mirrorBackend !== null && mirrorBackend.systemLoading
            text: i18nd("kxbps", "Reading repositories…")
            color: Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            visible: !(mirrorBackend !== null && mirrorBackend.systemLoading) && view.repos.length === 0
            text: i18nd("kxbps", "No repository is configured.")
            color: Kirigami.Theme.disabledTextColor
        }

        ListView {
            id: repoList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: view.repos

            QQC2.ScrollBar.vertical: QQC2.ScrollBar {
                id: repoBar
                policy: QQC2.ScrollBar.AsNeeded
            }

            delegate: QQC2.ItemDelegate {
                required property var modelData

                width: repoList.width - (repoBar.visible ? repoBar.width : 0)
                height: Kirigami.Units.gridUnit * 2.6

                Accessible.name: modelData.url
                Accessible.description: modelData.file

                contentItem: RowLayout {
                    spacing: Kirigami.Units.largeSpacing

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.minimumWidth: 0
                        spacing: 0

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: modelData.url
                            font.family: "monospace"
                            elide: Text.ElideMiddle
                        }

                        QQC2.Label {
                            Layout.fillWidth: true
                            text: modelData.file
                            font: Kirigami.Theme.smallFont
                            color: Kirigami.Theme.disabledTextColor
                            elide: Text.ElideMiddle
                        }
                    }

                    QQC2.Label {
                        visible: modelData.signed === true
                        text: i18nd("kxbps", "Signed")
                        font: Kirigami.Theme.smallFont
                        color: Kirigami.Theme.positiveTextColor
                    }

                    QQC2.ToolButton {
                        visible: view.isManaged(modelData)
                        icon.name: "edit-delete"
                        text: i18nd("kxbps", "Remove repository")
                        display: QQC2.AbstractButton.IconOnly
                        enabled: view.ready && !view.busy
                        Accessible.name: i18nd("kxbps", "Remove the repository %1", modelData.url)
                        onClicked: view.remove(modelData)
                    }
                }
            }
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            Layout.topMargin: Kirigami.Units.smallSpacing
            Layout.bottomMargin: Kirigami.Units.smallSpacing
            text: i18ndp("kxbps", "%1 repository", "%1 repositories", view.repos.length)
            font: Kirigami.Theme.smallFont
            color: Kirigami.Theme.disabledTextColor
            elide: Text.ElideRight
        }
    }
}
