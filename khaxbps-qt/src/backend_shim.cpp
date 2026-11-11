#include "backend_shim.h"

#include <QtCore/QMetaObject>
#include <QtCore/QVariant>

#include "khaxbps-qt/src/catalog_backend.cxxqt.h"
#include "khaxbps-qt/src/installed_backend.cxxqt.h"
#include "khaxbps-qt/src/job_backend.cxxqt.h"
#include "khaxbps-qt/src/log_model.cxxqt.h"
#include "khaxbps-qt/src/package_model.cxxqt.h"
#include "khaxbps-qt/src/plan_backend.cxxqt.h"
#include "khaxbps-qt/src/queue_model.cxxqt.h"

QString khaxbps_object_string_property(QObject *object, const QString &name)
{
    if (!object) {
        return QString();
    }
    return object->property(name.toUtf8().constData()).toString();
}

void khaxbps_package_model_set_state(QObject *model,
                                     const QString &installedJson,
                                     const QString &queueJson)
{
    static_cast<PackageModel *>(model)->setState(installedJson, queueJson);
}

QObject *khaxbps_create_queue_model(QObject *parent, const QString &queueJson)
{
    return new QueueModel(queueJson, parent);
}

void khaxbps_queue_model_set_state(QObject *model, const QString &queueJson)
{
    static_cast<QueueModel *>(model)->setState(queueJson);
}

QObject *khaxbps_create_log_model(QObject *parent)
{
    auto *model = new LogModel();
    model->setParent(parent);
    return model;
}

void khaxbps_log_model_append(QObject *model, const QString &line)
{
    static_cast<LogModel *>(model)->append(line);
}

QObject *khaxbps_create_plan_backend(QObject *parent, const QString &payloadJson)
{
    return new PlanBackend(payloadJson, parent);
}

QObject *khaxbps_start_job(QObject *owner, const QString &payloadJson)
{
    auto *job = new JobBackend();
    job->setParent(owner);
    job->start(payloadJson, owner);
    return job;
}

void khaxbps_installed_apply_plan(QObject *owner, const QString &itemsJson)
{
    static_cast<InstalledBackend *>(owner)->applyPlanItems(itemsJson);
}

void khaxbps_catalog_backend_set_state(QObject *backend,
                                       const QString &installedJson,
                                       const QString &queueJson)
{
    static_cast<CatalogBackend *>(backend)->setState(installedJson, queueJson);
}

void khaxbps_release_object(QObject *object)
{
    if (object) {
        object->deleteLater();
    }
}

QString khaxbps_installed_mirror(QObject *owner)
{
    if (!owner) {
        return QString();
    }
    return static_cast<InstalledBackend *>(owner)->mirrorValue();
}

void khaxbps_installed_set_mirror(QObject *owner, const QString &url)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setMirrorValue(url);
}

QString khaxbps_installed_prefs(QObject *owner)
{
    if (!owner) {
        return QString();
    }
    return static_cast<InstalledBackend *>(owner)->prefsValue();
}

void khaxbps_installed_set_pref_bool(QObject *owner, const QString &key, bool value)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setPrefBool(key, value);
}

void khaxbps_installed_set_pref_string(QObject *owner,
                                       const QString &key,
                                       const QString &value)
{
    if (!owner) {
        return;
    }
    static_cast<InstalledBackend *>(owner)->setPrefString(key, value);
}
