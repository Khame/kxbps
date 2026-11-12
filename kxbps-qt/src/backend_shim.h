// C++ factories and forwarding helpers for the installed backend.
//
// CXX-Qt 0.10 cannot construct a QObject from Rust and cannot call a generated
// method on a `QObject*` it only holds as a pointer, so the two directions of
// every child-object relationship cross this file: the backend asks C++ to
// `new` a child, and later hands the pointer back to a small forwarder that
// casts it to the generated class and calls the Rust method.
//
// Nothing here contains application logic; every decision stays in Rust.
#pragma once

#include <QtCore/QObject>
#include <QtCore/QString>

// Dynamic property `name` of `object` as a string ("" when it is missing or
// not a string). Used to read the plan a dialog was built from.
QString kxbps_object_string_property(QObject *object, const QString &name);

// Replaces the package model's installed snapshot and queue map. Emits
// `dataChanged` for the rows that moved, or a full reset when the visible set
// changes (for example the `installed` filter gaining a row).
void kxbps_package_model_set_state(QObject *model,
                                     const QString &installedJson,
                                     const QString &queueJson);

// Queue model: one row per queued `{name, action}`.
QObject *kxbps_create_queue_model(QObject *parent, const QString &queueJson);
void kxbps_queue_model_set_state(QObject *model, const QString &queueJson);

// Log model: append-only lines shown by the job dialog.
QObject *kxbps_create_log_model(QObject *parent);
void kxbps_log_model_append(QObject *model, const QString &line);

// A fully populated plan dialog model, built from one JSON payload.
QObject *kxbps_create_plan_backend(QObject *parent, const QString &payloadJson);

// Creates the job, starts its worker thread and returns it. `owner` is the
// installed backend the job reports back to when the simulated transaction
// finishes.
QObject *kxbps_start_job(QObject *owner, const QString &payloadJson);

// Called on the GUI thread when a job finishes: applies the plan to the store,
// saves it and refreshes the models.
void kxbps_installed_apply_plan(QObject *owner, const QString &itemsJson);

// Pushes the installed/queue snapshot into the query façade, so `search()` and
// `get()` answer `installedVersion`/`isUpdate` from the same state as the model.
void kxbps_catalog_backend_set_state(QObject *backend,
                                       const QString &installedJson,
                                       const QString &queueJson);

// Schedules `object` for deletion on the next event-loop pass; null-safe.
// The backend keeps parent ownership of every plan and job it creates, but a
// QML-referenced child is never freed by JavaScript, so a closed dialog must
// call `InstalledBackend::release` or the object outlives the dialog.
void kxbps_release_object(QObject *object);

// The selected mirror. The store owns it, the Mirrors view presents it; the
// mirror backend reads it at startup and writes it back on every change, so a
// restart remembers the choice (`mirror.json`).
QString kxbps_installed_mirror(QObject *owner);
void kxbps_installed_set_mirror(QObject *owner, const QString &url);

// The preference block (`prefs.json`). The preferences backend reads the whole
// block once when it is attached to the store owner and writes each change back
// by key, so no second copy of the store exists.
QString kxbps_installed_prefs(QObject *owner);
void kxbps_installed_set_pref_bool(QObject *owner, const QString &key, bool value);
void kxbps_installed_set_pref_string(QObject *owner,
                                       const QString &key,
                                       const QString &value);
