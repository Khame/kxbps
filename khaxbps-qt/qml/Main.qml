import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.khaxbps

Kirigami.ApplicationWindow {
    id: root
    width: 900; height: 640
    // QQmlApplicationEngine does not show the root window on its own.
    visible: true
    title: "Khaxbps"

    AppBackend {
        id: appBackend
    }

    pageStack.initialPage: Kirigami.Page {
        title: "Packages"
        ColumnLayout {
            anchors.centerIn: parent
            // TODO(i18n): switch to i18nd("khaxbps", ...) once KLocalization::setupLocalizedContext
            // is wired in C++ (needs kf6-ki18n-devel); bare i18nd() is not defined without it.
            QQC2.Label { text: "Version " + appBackend.version() }
            QQC2.Label { text: "Updates: " + appBackend.updateCount }
        }
    }
}
