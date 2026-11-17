// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// A button that hugs its label.
//
// The Breeze style sizes a button from the grid unit — about two of them — and
// not from the text inside. With the application font capped at 10 pt that left
// a tall empty box around every label, which is what made the toolbars and the
// details pane read as oversized. This keeps the height close to the text and
// the padding small; everything else still comes from the instance.
//
// Buttons are flat. The window is a tool: a framed chip on every toolbar item,
// on every row of the mirror list and on every action of the details pane is
// noise, not affordance, and one framed button among flat ones reads as a
// different kind of control rather than as the same one. Flat is also what
// Kirigami's own toolbars use, and the style still paints a hover and a focus
// highlight, so a control never stops looking clickable. The one accent action
// of a page — Apply, Apply to system — asks for a frame with `flat: false`,
// because it is the only control that changes the machine.

import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.Button {
    // The style sizes the box from the grid unit and hands the button the
    // application font. On a toolbar a label a step below the body text reads
    // better, and the step is derived from the theme rather than fixed, so a
    // desktop that scales up still gets a proportional button.
    font.pointSize: Math.max(8, Math.round(Kirigami.Theme.defaultFont.pointSize * 0.85))
    implicitHeight: Kirigami.Units.gridUnit * 1.6
    padding: Kirigami.Units.smallSpacing
    flat: true
    // One icon size for every control. The sidebar draws its own icons at
    // `iconSizes.small`; leaving the button to the style produced a second,
    // slightly different size for the same kind of glyph.
    icon.width: Kirigami.Units.iconSizes.small
    icon.height: Kirigami.Units.iconSizes.small
}
