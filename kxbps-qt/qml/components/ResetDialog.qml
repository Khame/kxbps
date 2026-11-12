// The destructive confirmation for "Forget the local settings".
//
// It is not a transaction: it rewrites the store in place, so there is no plan
// to review and no job to watch. Nothing installed on the system is touched —
// only the queue, the mirror and the preferences are forgotten. The warning is
// carried by the label in the negative text colour; the buttons themselves stay
// the standard style.
//
// `restored()` is the only way out that touches the store. `Main.qml` listens to
// it to drop the last transaction's result, because that result no longer
// describes the store after a reset. Cancel emits nothing, so a cancel never
// clears it — and it cannot be inferred from `stateRevision` either, since a
// job finishing while the dialog is open advances that too.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property var installedBackend

    // Emitted after the store has been cleared.
    signal restored()

    title: i18nd("kxbps", "Forget the local settings?")
    preferredWidth: Kirigami.Units.gridUnit * 40
    padding: Kirigami.Units.largeSpacing

    function restore() {
        installedBackend.reset()
        restored()
        close()
    }

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 38
        spacing: Kirigami.Units.smallSpacing

        // Referenced by the footer list below.
        Kirigami.Action {
            id: cancelAction
            text: i18nd("kxbps", "Cancel")
            onTriggered: dialog.reject()
        }

        Kirigami.Action {
            id: restoreAction
            text: i18nd("kxbps", "Forget")
            onTriggered: dialog.restore()
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "The queue, the mirror and the preferences are forgotten.")
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "Installed packages are not changed.")
            color: Kirigami.Theme.negativeTextColor
            wrapMode: Text.WordWrap
        }
    }

    customFooterActions: [cancelAction, restoreAction]
}
