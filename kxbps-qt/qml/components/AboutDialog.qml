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

        Image {
            source: "qrc:/qt/qml/org/kde/kxbps/icons/kxbps.svg"
            sourceSize.width: Kirigami.Units.iconSizes.huge
            sourceSize.height: Kirigami.Units.iconSizes.huge
            Layout.preferredWidth: Kirigami.Units.iconSizes.huge
            Layout.preferredHeight: Kirigami.Units.iconSizes.huge
            fillMode: Image.PreserveAspectFit
            Accessible.name: i18nd("kxbps", "KXbps")
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
            // Legal notice: a name and a licence identifier do not translate.
            text: "Copyright © 2026 Łukasz \"Khame\" Linhard · GPL-3.0-or-later"
            color: Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            text: i18nd("kxbps", "A graphical front end for xbps, the Void Linux package manager.")
            wrapMode: Text.WordWrap
        }

        QQC2.Label {
            Layout.fillWidth: true
            text: i18nd("kxbps", "It runs xbps and shows exactly what xbps reports.")
            color: Kirigami.Theme.disabledTextColor
            wrapMode: Text.WordWrap
        }
    }

    customFooterActions: [closeAction]
}
