// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

.pragma library

// The one place a glyph of the KXbps icon set is named.
//
// `KxbpsIcon` turns a name into `qrc:/…/kxbps-<name>-symbolic.svg`, so a view
// never spells a path. Renaming an icon means changing the file and the entry
// here, and every view follows.

var log = "log"
var installed = "installed"
var updates = "updates"
var available = "available"
var selected = "selected"
var manual = "manual"
var orphaned = "orphaned"
var locked = "locked"
var mirrors = "mirrors"
var repositories = "repositories"
var help = "help"
var settings = "settings"
var coffee = "coffee"

// Theme glyphs, drawn by the icon theme and set through `icon.name`. They live
// here so a toolbar and a row cannot spell the same glyph differently, and the
// comment above `KxbpsIcon` is what tells the two groups apart: a name from the
// first half becomes a `qrc:` path, a name from this half is looked up in the
// theme.
var refresh = "view-refresh"
var apply = "dialog-ok-apply"
var clear = "edit-clear"
