// What this application is, in one screen and without a marketing page.
//
// The version comes from `AppBackend.version()` in Rust; the second paragraph
// is the honest part — the package image is local, so nothing here touches the
// real system.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property string version: ""

    title: i18nd("kxbps", "About KXbps")
    preferredWidth: Kirigami.Units.gridUnit * 40
    padding: Kirigami.Units.largeSpacing

    ColumnLayout {
        implicitWidth: Kirigami.Units.gridUnit * 38
        spacing: Kirigami.Units.smallSpacing

        // Referenced by the footer list below.
        Kirigami.Action {
            id: closeAction
            text: i18nd("kxbps", "Close")
            onTriggered: dialog.close()
        }

        Kirigami.Heading {
            Layout.fillWidth: true
            level: 2
            text: i18nd("kxbps", "KXbps")
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "Version %1", dialog.version)
            color: Kirigami.Theme.disabledTextColor
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "A package manager for Void Linux, backed by xbps.")
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "The package image is local; no xbps calls are made on your system.")
            color: Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }
    }

    customFooterActions: [closeAction]
}
