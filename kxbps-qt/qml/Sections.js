// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

.pragma library

// The pages the window can show, named once.
//
// `section` is a plain string property, so a typo in a comparison is a page
// that never highlights. The list in `navigate()` and every `switch` read these
// constants instead of retyping the words.

var packages = "packages"
var mirrors = "mirrors"
var repositories = "repositories"
var help = "help"
var settings = "settings"
