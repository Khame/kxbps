import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0
import org.kde.khaxbps

Kirigami.ApplicationWindow {
    id: root
    width: 900; height: 640
    // QQmlApplicationEngine does not show the root window on its own.
    visible: true
    title: i18nd("khaxbps", "Khaxbps")

    AppBackend {
        id: appBackend
    }

    pageStack.initialPage: Kirigami.Page {
        title: i18nd("khaxbps", "Packages")
        ColumnLayout {
            anchors.centerIn: parent
            QQC2.Label { text: i18nd("khaxbps", "Version %1", appBackend.version()) }
            QQC2.Label { text: i18nd("khaxbps", "Updates: %1", appBackend.updateCount) }
        }
    }
}
