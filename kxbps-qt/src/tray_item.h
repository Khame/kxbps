// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The tray QObject itself. moc runs over this header (it is listed in
// `build.rs`), which is what gives QML real signals to connect to. Only
// `tray_shim.cpp` includes it.

#pragma once

#include <QAction>
#include <QMenu>
#include <QObject>
#include <QPoint>
#include <QString>

#include <KLocalizedString>
#include <KStatusNotifierItem>

namespace kxbps_tray
{
constexpr auto APP_TITLE = "KXbps";
constexpr auto ITEM_ID = "io.github.khame.kxbps";

/// The icon name the packaging installs, in both states.
inline QString icon_name(bool waiting)
{
    return waiting ? QStringLiteral("kxbps-tray-updates-symbolic")
                   : QStringLiteral("kxbps-tray-symbolic");
}
} // namespace kxbps_tray

/// The tray icon and its two-entry context menu.
///
/// The window owns what the entries mean: `showRequested` and `quitRequested`
/// are only requests. The update count is pushed from QML and changes the icon
/// and the tooltip, which is what makes the panel say what the window says.
class KxbpsTray : public QObject
{
    Q_OBJECT
    Q_PROPERTY(int updateCount READ updateCount WRITE setUpdateCount NOTIFY updateCountChanged)

public:
    explicit KxbpsTray(QObject *parent = nullptr)
        : QObject(parent)
        , m_item(new KStatusNotifierItem(QString::fromUtf8(kxbps_tray::ITEM_ID), this))
    {
        // The standard Quit/Configure entries belong to an application with its
        // own menu; this one has exactly two entries, and the window decides
        // what they mean.
        m_item->setStandardActionsEnabled(false);
        m_item->setCategory(KStatusNotifierItem::ApplicationStatus);
        m_item->setStatus(KStatusNotifierItem::Active);
        m_item->setTitle(QString::fromUtf8(kxbps_tray::APP_TITLE));

        QMenu *menu = m_item->contextMenu();
        QAction *open = menu->addAction(i18nd("kxbps", "Open KXbps"));
        QAction *quit = menu->addAction(i18nd("kxbps", "Quit"));
        connect(open, &QAction::triggered, this, &KxbpsTray::showRequested);
        connect(quit, &QAction::triggered, this, &KxbpsTray::quitRequested);
        // A left click is an activation, not a menu: the host reports it here.
        connect(m_item,
                &KStatusNotifierItem::activateRequested,
                this,
                [this](bool active, const QPoint &) {
                    if (active) {
                        Q_EMIT showRequested();
                    }
                });

        refreshIcon();
    }

    int updateCount() const
    {
        return m_count;
    }

    /// The same count the window shows; zero is the plain icon.
    void setUpdateCount(int count)
    {
        if (count == m_count) {
            return;
        }
        m_count = count;
        refreshIcon();
        Q_EMIT updateCountChanged();
    }

Q_SIGNALS:
    /// The icon was activated, or "Open KXbps" was chosen.
    void showRequested();
    /// "Quit" was chosen.
    void quitRequested();
    void updateCountChanged();

private:
    void refreshIcon()
    {
        const bool waiting = m_count > 0;
        m_item->setIconByName(kxbps_tray::icon_name(waiting));
        // `i18ndp` substitutes the count into `%1` itself.
        const QString subtitle = waiting
            ? i18ndp("kxbps", "%1 package can be updated.", "%1 packages can be updated.", m_count)
            : QString();
        m_item->setToolTip(QString(), QString::fromUtf8(kxbps_tray::APP_TITLE), subtitle);
    }

    KStatusNotifierItem *m_item;
    int m_count = 0;
};
