#include "backend_shim.h"

#include <QtCore/QMetaObject>
#include <QtCore/QVariant>
#include <QtDBus/QDBusConnection>
#include <QtDBus/QDBusMessage>
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

void kxbps_notify(const QString &summary, const QString &body, const QString &icon)
{
    // The spec's `Notify` call, sent fire-and-forget: the reply is a number the
    // window has no use for, and waiting on it would block the GUI thread.
    // `desktop-entry` is what lets the notification settings page group the
    // popup under KXbps instead of a nameless sender.
    QDBusMessage message = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("/org/freedesktop/Notifications"),
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("Notify"));
    QVariantMap hints;
    hints.insert(QStringLiteral("desktop-entry"),
                 QStringLiteral("io.github.khame.kxbps"));
    message << QStringLiteral("KXbps") << 0u << icon << summary << body
            << QStringList() << hints << 10000;
    QDBusConnection::sessionBus().send(message);
}

void kxbps_installed_apply_plan(QObject *owner, const QString &itemsJson)
{
    static_cast<InstalledBackend *>(owner)->applyPlanItems(itemsJson);
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
