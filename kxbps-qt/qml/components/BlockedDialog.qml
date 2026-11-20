// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// The explanation for a removal the store refuses because other packages still
// need the target.
//
// The blocked plan is a C++-parented child of `InstalledBackend`; this dialog
// owns it and releases it on close. "Remove together" asks the backend for the
// cascade plan, releases the blocked one and hands the new plan to `PlanDialog`,
// which owns it from then on.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.ki18n 1.0

Kirigami.Dialog {
    id: dialog

    property var installedBackend
    property var plan: null

    signal cascadeRequested(var cascadePlan)

    title: i18nd("kxbps", "Cannot remove")
    preferredWidth: Kirigami.Units.gridUnit * 40
    maximumHeight: (parent ? parent.height : Kirigami.Units.gridUnit * 36) * 0.7
    padding: Kirigami.Units.largeSpacing

    onClosed: dialog.releasePlan()

    function showPlan(value) {
        if (value === null) {
            return
        }
        if (value !== plan) {
            dialog.releasePlan()
        }
        plan = value
        open()
    }

    function releasePlan() {
        if (plan === null) {
            return
        }
        const old = plan
        plan = null
        installedBackend.release(old)
    }

    function cascade() {
        if (plan === null) {
            return
        }
        const names = plan.names
        const next = installedBackend.openRemoveNames(names, true, plan.recursive)
        dialog.releasePlan()
        close()
        if (next !== null) {
            cascadeRequested(next)
        }
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
            id: cascadeAction
            text: i18nd("kxbps", "Remove together with them")
            onTriggered: dialog.cascade()
        }

        Repeater {
            model: dialog.plan !== null ? dialog.plan.blocked : []

            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                spacing: 0

                QQC2.Label {
                    Layout.fillWidth: true
                    text: modelData.name
                    font.bold: true
                    elide: Text.ElideRight
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    text: i18nd("kxbps", "Needed by: %1", modelData.neededBy.join(", "))
                    color: Kirigami.Theme.disabledTextColor
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    customFooterActions: [cancelAction, cascadeAction]
}
