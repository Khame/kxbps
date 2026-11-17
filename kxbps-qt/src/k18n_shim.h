// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Declaration for the KI18n QML glue in `k18n_shim.cpp`. The CXX bridge in
// `src/lib.rs` includes this header (`include!("k18n_shim.h")`) so the code cxx
// generates has the declaration of `kxbps_setup_i18n` in scope.

#pragma once

class QQmlEngine;

void kxbps_setup_i18n(QQmlEngine &engine);

/// Caps the application font at 10 pt. The desktop's default (12 pt here) is
/// sized for documents; a table of versions and sizes reads oversized next to
/// the panel and the window titles at that size. A smaller system setting is
/// left alone.
void kxbps_set_compact_font();
