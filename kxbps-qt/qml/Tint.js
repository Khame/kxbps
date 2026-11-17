// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

.pragma library

// The wash a flat row gets from the theme's highlight colour.
//
// One row is selected, another is under the pointer, and both are drawn the same
// way wherever they appear: the sidebar, the package table, the mirror list, the
// queue and the log drawer. Two alphas, one definition — a component that
// invents a third value has drifted from the rest of the window.

/// The selected row.
var selection = 0.2

/// The row under the pointer.
var hover = 0.1

/// `base` at `alpha`: the expression every caller needs from `Qt.rgba`, so the
/// colour is never rebuilt by hand.
function wash(base, alpha) {
    return Qt.rgba(base.r, base.g, base.b, alpha)
}
