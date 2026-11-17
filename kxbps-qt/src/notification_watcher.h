// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// A click on the notification popup, delivered to the backend that sent it.
//
// The notification server reports a click as the `ActionInvoked(uint, QString)`
// signal. `QDBusConnection::connect` needs a real slot for that, and a CXX-Qt
// method is only `Q_INVOKABLE`, so this tiny QObject exists to own the slot:
// moc runs over this header (cxx-qt-build does that for headers listed in
// `build.rs`), and the slot forwards to the Rust method that decides whether
// the id belongs to this process.
//
// Kept out of the bridge on purpose: the bridge has no slot support in CXX-Qt
// 0.10, and a QML `Connections` element cannot bind to a signal that only
// appears in a dynamic meta-object.

#pragma once

#include <QDBusConnection>
#include <QObject>
#include <QString>

class NotificationWatcher : public QObject
{
    Q_OBJECT

public:
    // `owner` is the CXX-Qt backend; the watcher is parented to it and never
    // outlives it.
    explicit NotificationWatcher(QObject &owner)
        : QObject(&owner)
        , m_owner(owner)
    {
    }

    // Subscribes to the signal once per process. False when the session bus
    // refused the connection, which the caller reports.
    bool watch()
    {
        return QDBusConnection::sessionBus().connect(
            QStringLiteral("org.freedesktop.Notifications"),
            QStringLiteral("/org/freedesktop/Notifications"),
            QStringLiteral("org.freedesktop.Notifications"),
            QStringLiteral("ActionInvoked"),
            QStringLiteral("us"),
            this,
            SLOT(onActionInvoked(uint, QString)));
    }

private Q_SLOTS:
    void onActionInvoked(uint id, const QString &key)
    {
        // Queued: the handler shows and raises the window, which does not
        // belong inside the D-Bus dispatch.
        QMetaObject::invokeMethod(&m_owner,
                                  "onNotificationAction",
                                  Qt::QueuedConnection,
                                  Q_ARG(uint, id),
                                  Q_ARG(QString, key));
    }

private:
    QObject &m_owner;
};
