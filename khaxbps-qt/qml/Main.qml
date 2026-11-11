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

    // Query helpers over the catalog (`search`, `get`).
    CatalogBackend {
        id: catalogBackend
    }

    // Owns the store: the package model, the queue, planning and applying.
    InstalledBackend {
        id: installedBackend
        // The query façade answers `search`/`get` from the same snapshot; without
        // this wiring its results would report every package as not installed.
        queryBackend: catalogBackend
    }

    // The mirror list, its probe batch and the persisted selection. `owner` is
    // the store owner, so the chosen mirror is saved through it.
    MirrorBackend {
        id: mirrorBackend
        owner: installedBackend
    }


    pageStack.initialPage: Kirigami.Page {
        title: i18nd("khaxbps", "Packages")

        ColumnLayout {
            anchors.fill: parent

            QQC2.Label { text: i18nd("khaxbps", "Version %1", appBackend.version()) }
            QQC2.Label { text: i18nd("khaxbps", "Updates: %1", installedBackend.updatesCount) }

            // A catalog that failed to load must be visible, not silent.
            QQC2.Label {
                Layout.fillWidth: true
                visible: catalogBackend.error.length > 0
                color: Kirigami.Theme.negativeTextColor
                text: i18nd("khaxbps", "Could not load the package catalog: %1", catalogBackend.error)
                wrapMode: Text.WordWrap
            }

            // A store that could not be loaded or saved keeps the session going
            // in memory; the reason is technical and wrapped here.
            QQC2.Label {
                Layout.fillWidth: true
                visible: installedBackend.error.length > 0
                color: Kirigami.Theme.neutralTextColor
                text: i18nd("khaxbps", "Could not save the system state: %1", installedBackend.error)
                wrapMode: Text.WordWrap
            }

            QQC2.Label {
                text: i18nd("khaxbps", "%1 packages installed, %2 queued",
                            installedBackend.installedCount, installedBackend.queueCount)
            }

            // Proof of the plan API; Task 10 replaces this with the real toolbar.
            RowLayout {
                QQC2.Button {
                    text: i18nd("khaxbps", "System plan")
                    onClicked: {
                        const plan = installedBackend.openSystemPlan()
                        if (!plan) {
                            systemPlanLabel.text = i18nd("khaxbps", "Nothing to do.")
                        } else {
                            systemPlanLabel.text = i18nd("khaxbps", "%1 packages, %2 to download",
                                                         plan.count, plan.downloadText)
                        }
                    }
                }
                QQC2.Label {
                    id: systemPlanLabel
                    text: i18nd("khaxbps", "No plan opened yet.")
                }
            }

            QQC2.Label {
                text: i18nd("khaxbps", "%1 packages", installedBackend.packages.count)
            }

            // Proof of the mirror API; Task 10 replaces this with the Mirrors view.
            RowLayout {
                QQC2.Button {
                    text: i18nd("khaxbps", "Probe all mirrors")
                    enabled: !mirrorBackend.probing
                    onClicked: mirrorBackend.probeAll()
                }
                QQC2.Label {
                    text: i18nd("khaxbps", "%1 of %2 mirrors probed",
                                mirrorBackend.finishedCount, mirrorBackend.model.count)
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    opacity: 0.7
                    text: i18nd("khaxbps", "Current mirror: %1", mirrorBackend.currentMirror)
                }
            }

            // No styling yet: Task 10 replaces this with PackageDelegate.
            ListView {
                id: packageList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: installedBackend.packages
                delegate: QQC2.ItemDelegate {
                    width: ListView.view.width
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.smallSpacing
                        QQC2.Label { text: model.name }
                        QQC2.Label { text: model.version; opacity: 0.7 }
                        Item { Layout.fillWidth: true }
                        QQC2.Label { text: model.repo; opacity: 0.7 }
                    }
                }
            }
        }
    }
}
