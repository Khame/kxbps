// SPDX-License-Identifier: GPL-3.0-or-later
//
// The values the About page reports. Qt and KDE Frameworks are the libraries
// this build links against; the platform comes from Qt's own system
// information. Only the raw pieces are read here — the shape of the label is
// decided in Rust, where it can be tested without a QGuiApplication.

#pragma once

#include <QtCore/QString>

QString kxbps_qt_version();
QString kxbps_kf_version();
QString kxbps_platform_pretty();
QString kxbps_platform_plugin();
