#include "backend_shim.h"
#include "notification_watcher.h"

#include <QtCore/QMetaObject>
#include <QtCore/QVariant>
#include <QtDBus/QDBusConnection>
#include <QtDBus/QDBusMessage>
#include <QtDBus/QDBusPendingCallWatcher>
#include <QtDBus/QDBusPendingReply>
#include <QtGui/QGuiApplication>
#include <QtWidgets/QSystemTrayIcon>

#include "kxbps-qt/src/query_backend.cxxqt.h"
#include "kxbps-qt/src/installed_backend.cxxqt.h"
#include "kxbps-qt/src/job_backend.cxxqt.h"
#include "kxbps-qt/src/log_model.cxxqt.h"
#include "kxbps-qt/src/mirror_backend.cxxqt.h"
#include "kxbps-qt/src/package_model.cxxqt.h"
#include "kxbps-qt/src/plan_backend.cxxqt.h"
#include "kxbps-qt/src/queue_model.cxxqt.h"

QString kxbps_object_string_property(QObject *object, const QString &name)
{
    if (!object) {
        return QString();
    }
    return object->property(name.toUtf8().constData()).toString();
}

void kxbps_package_model_set_state(QObject *model,
                                     const QString &installedJson,
                                     const QString &queueJson)
{
    static_cast<PackageModel *>(model)->setState(installedJson, queueJson);
}

void kxbps_package_model_set_snapshot(QObject *model,
                                        const QString &snapshotJson,
                                        const QString &queueJson)
{
    static_cast<PackageModel *>(model)->setSnapshot(snapshotJson, queueJson);
}

void kxbps_package_model_set_search_results(QObject *model,
                                              const QString &hitsJson,
                                              const QString &errorKey,
                                              const QStringList &errorArgs)
{
    static_cast<PackageModel *>(model)->setSearchResults(hitsJson, errorKey, errorArgs);
}

QObject *kxbps_create_queue_model(QObject *parent, const QString &queueJson)
{
    return new QueueModel(queueJson, parent);
}

void kxbps_queue_model_set_state(QObject *model, const QString &queueJson)
{
    static_cast<QueueModel *>(model)->setState(queueJson);
}

QObject *kxbps_create_log_model(QObject *parent)
{
    auto *model = new LogModel();
    model->setParent(parent);
    return model;
}

void kxbps_log_model_append(QObject *model, const QString &line)
{
    static_cast<LogModel *>(model)->append(line);
}

QObject *kxbps_create_plan_backend(QObject *parent, const QString &payloadJson)
{
    return new PlanBackend(payloadJson, parent);
}

QObject *kxbps_start_job(QObject *owner, const QString &payloadJson)
{
    // The job parses the payload itself; `new` takes the parent so Qt destroys
    // it with the installed backend if QML never releases it.
    auto *job = new JobBackend(payloadJson, owner);
    // The constructor only builds the command list. Without this call nothing
    // runs: the job keeps its `jobStart` label, the strip shows a bar that never
    // moves and the log stays empty, because no process was ever spawned.
    job->start(payloadJson, owner);
    return job;
}

QObject *kxbps_start_mirror_job(QObject *owner, const QString &payloadJson)
{
    // A distinct name for the mirror view: two Rust bridges cannot both declare
    // `kxbps_start_job` without emitting the same CXX thunk twice.
    return kxbps_start_job(owner, payloadJson);
}

bool kxbps_tray_available()
{
    return QSystemTrayIcon::isSystemTrayAvailable();
}

void kxbps_set_badge(int64_t count)
{
    // One call: Qt owns the `com.canonical.Unity.LauncherEntry` signal, the
    // object path built from the desktop file name, and the `count-visible`
    // flag Plasma reads. Zero clears the badge instead of drawing a zero.
    //
    // `setBadgeNumber` is an instance slot, not a static, so it goes through
    // the application object; the backend is only ever created after it exists.
    if (auto *app = qobject_cast<QGuiApplication *>(QCoreApplication::instance())) {
        app->setBadgeNumber(count);
    }
}

void kxbps_notify(const QString &summary,
                  const QString &body,
                  const QString &icon,
                  const QString &action,
                  QObject &owner)
{
    // The `Notify` call, sent without waiting: blocking the GUI thread on a
    // hung notification server would freeze the window. `desktop-entry` is
    // what lets the notification settings page group the popup under KXbps
    // instead of a nameless sender.
    QDBusMessage message = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("/org/freedesktop/Notifications"),
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("Notify"));
    QVariantMap hints;
    hints.insert(QStringLiteral("desktop-entry"),
                 QStringLiteral("io.github.khame.kxbps"));
    // One button, and clicking the body activates `default`; both arrive as
    // `ActionInvoked` with the key `default`.
    message << QStringLiteral("KXbps") << 0u << icon << summary << body
            << QStringList{QStringLiteral("default"), action}
            << hints << 10000;
    // The watch is set up right before the first popup, when the notification
    // service is certainly there; a later call is a no-op.
    kxbps_notification_watch_actions(owner);
    QDBusPendingCall call = QDBusConnection::sessionBus().asyncCall(message);
    auto *watcher = new QDBusPendingCallWatcher(call, &owner);
    QObject::connect(watcher,
                     &QDBusPendingCallWatcher::finished,
                     &owner, [owner = &owner](QDBusPendingCallWatcher *finished) {
                         // The id is the only way to tell which popup an
                         // `ActionInvoked` belongs to; `none` when the reply
                         // failed, which no action can match.
                         const QDBusPendingReply<uint> reply = *finished;
                         finished->deleteLater();
                         QString id;
                         if (!reply.isError()) {
                             id = QString::number(reply.value());
                         }
                         QMetaObject::invokeMethod(owner,
                                                   "rememberNotificationId",
                                                   Qt::DirectConnection,
                                                   Q_ARG(QString, id));
                     });
}

void kxbps_notification_watch_actions(QObject &owner)
{
    // Once per process: a second watcher would deliver every click twice.
    static QObject *watched = nullptr;
    if (watched != nullptr) {
        return;
    }
    auto *watcher = new NotificationWatcher(owner);
    if (!watcher->watch()) {
        qWarning("kxbps: notification actions: %s",
                 qUtf8Printable(QDBusConnection::sessionBus().lastError().message()));
        delete watcher;
        return;
    }
    watched = &owner;
}

void kxbps_installed_apply_plan(QObject *owner, const QString &itemsJson)
{
    static_cast<InstalledBackend *>(owner)->applyPlanItems(itemsJson);
}

void kxbps_installed_hold_landed(QObject *owner)
{
    static_cast<InstalledBackend *>(owner)->holdLanded();
}

bool kxbps_job_begin(QObject *job)
{
    return static_cast<JobBackend *>(job)->beginTransaction();
}

void kxbps_job_spawned(QObject *job)
{
    static_cast<JobBackend *>(job)->markSpawned();
}

void kxbps_installed_watch_mirror_job(QObject *owner, QObject *backend)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->watchMirrorJob(backend);
}

void kxbps_installed_unwatch_mirror_job(QObject *owner)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->unwatchMirrorJob();
}

void kxbps_installed_mirror_job_landed(QObject *owner)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->mirrorJobLanded();
}

void kxbps_mirror_apply_landed(QObject *backend)
{
    if (!backend) {
        return;
    }
    static_cast<MirrorBackend *>(backend)->finishApply();
}


void kxbps_query_backend_set_state(QObject *backend,
                                   const QString &installedJson,
                                   const QString &queueJson,
                                   const QString &updatesJson,
                                   const QString &orphansJson)
{
    static_cast<QueryBackend *>(backend)->setState(installedJson,
                                                   queueJson,
                                                   updatesJson,
                                                   orphansJson);
}

void kxbps_release_object(QObject *object)
{
    if (object) {
        object->deleteLater();
    }
}

QString kxbps_installed_mirror(QObject *owner)
{
    if (!owner) {
        return QString();
    }
    return static_cast<InstalledBackend *>(owner)->mirrorValue();
}

void kxbps_installed_set_mirror(QObject *owner, const QString &url)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setMirrorValue(url);
}

QString kxbps_installed_prefs(QObject *owner)
{
    if (!owner) {
        return QString();
    }
    return static_cast<InstalledBackend *>(owner)->prefsValue();
}

void kxbps_installed_set_pref_bool(QObject *owner, const QString &key, bool value)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setPrefBool(key, value);
}

void kxbps_installed_set_pref_string(QObject *owner,
                                       const QString &key,
                                       const QString &value)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setPrefString(key, value);
}

void kxbps_installed_set_pref_int(QObject *owner, const QString &key, int value)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setPrefInt(key, value);
}
