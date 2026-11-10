// SPDX-License-Identifier: GPL-2.0-or-later
//
// Declaration for the KI18n QML glue in `k18n_shim.cpp`. The CXX bridge in
// `src/lib.rs` includes this header (`include!("k18n_shim.h")`) so the code cxx
// generates has the declaration of `khaxbps_setup_i18n` in scope.

#pragma once

class QQmlEngine;

void khaxbps_setup_i18n(QQmlEngine &engine);
