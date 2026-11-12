// The destructive confirmation for "Restore base image".
//
// Restoring is not a transaction: it rewrites the store in place, so there is
// no plan to review and no job to watch. The warning is carried by the label in
// the negative text colour; the buttons themselves stay the standard style.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property var installedBackend

    title: i18nd("khaxbps", "Restore the base image?")
    preferredWidth: Kirigami.Units.gridUnit * 40
    padding: Kirigami.Units.largeSpacing

    function restore() {
        installedBackend.reset()
        close()
    }

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 38
        spacing: Kirigami.Units.smallSpacing

        // Referenced by the footer list below.
        Kirigami.Action {
            id: cancelAction
            text: i18nd("khaxbps", "Cancel")
            onTriggered: dialog.reject()
        }

        Kirigami.Action {
            id: restoreAction
            text: i18nd("khaxbps", "Restore")
            onTriggered: dialog.restore()
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("khaxbps", "The queue and the installed packages go back to the seed image.")
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("khaxbps", "This cannot be undone.")
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }
    }

    customFooterActions: [cancelAction, restoreAction]
}
