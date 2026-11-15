// SPDX-License-Identifier: GPL-3.0-or-later
//
// The versions the About page reports. Qt and KDE Frameworks are compile-time
// or runtime values of the libraries KXbps links against; the platform name
// comes from Qt's own system information, not from a hard-coded string.

#pragma once

#include <QtCore/QString>

QString kxbps_qt_version();
QString kxbps_kf_version();
QString kxbps_platform_name();
