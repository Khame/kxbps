// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// One glyph from the KXbps symbolic set.
//
// The set is drawn as a mask: every shape uses `currentColor`, so the colour
// comes from Kirigami rather than from the SVG. That is what keeps the sidebar
// and the log button on the same palette in light and dark mode.

import QtQuick
import org.kde.kirigami as Kirigami

Kirigami.Icon {
    id: root

    // A name from `packaging/icons/actions`, without the `kxbps-` prefix and
    // the `-symbolic.svg` suffix, for example `installed` or `log`.
    property string symbolicName: ""

    source: symbolicName.length > 0
            ? "qrc:/qt/qml/org/kde/kxbps/icons/actions/kxbps-" + symbolicName + "-symbolic.svg"
            : ""
    isMask: true
    color: Kirigami.Theme.textColor
}
