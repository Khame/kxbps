// SPDX-License-Identifier: GPL-3.0-or-later

#include "about_shim.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QSysInfo>
#include <QtGui/QGuiApplication>

#include <ki18n_version.h>

QString kxbps_qt_version()
{
    return QString::fromLatin1(qVersion());
}

QString kxbps_kf_version()
{
    return QString::fromLatin1(KI18N_VERSION_STRING);
}

QString kxbps_platform_name()
{
    return QSysInfo::prettyProductName() + QStringLiteral(" (")
        + QGuiApplication::platformName() + QStringLiteral(")");
}
