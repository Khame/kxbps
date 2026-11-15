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

#include <cstdint>

#include <QtCore/QObject>
#include <QtCore/QString>

// Dynamic property `name` of `object` as a string ("" when it is missing or
// not a string). Used to read the plan a dialog was built from.
QString kxbps_object_string_property(QObject *object, const QString &name);

// Replaces the package model's installed set and queue map. Emits
// `dataChanged` for the rows that moved, or a full reset when the visible set
// changes (for example the `installed` filter gaining a row).
void kxbps_package_model_set_state(QObject *model,
                                     const QString &installedJson,
                                     const QString &queueJson);

// Replaces the package model's whole state with a `Snapshot`: the installed
// records, the updates and the explicit/held/orphan names.
void kxbps_package_model_set_snapshot(QObject *model,
                                        const QString &snapshotJson,
                                        const QString &queueJson);

// Shows the ranked repository answer of the search field: the hits as JSON,
// plus the failure the read reported (a key and its arguments, empty on
// success). An empty `hitsJson` is an empty search or a failed one.
void kxbps_package_model_set_search_results(QObject *model,
                                              const QString &hitsJson,
                                              const QString &errorKey,
                                              const QStringList &errorArgs);

// Queue model: one row per queued `{name, action}`.
QObject *kxbps_create_queue_model(QObject *parent, const QString &queueJson);
void kxbps_queue_model_set_state(QObject *model, const QString &queueJson);

// Log model: append-only lines shown by the job dialog.
QObject *kxbps_create_log_model(QObject *parent);
void kxbps_log_model_append(QObject *model, const QString &line);

// A fully populated plan dialog model, built from one JSON payload.
QObject *kxbps_create_plan_backend(QObject *parent, const QString &payloadJson);

// Creates the job, starts its worker thread and returns it. `owner` is the
// installed backend the job reports back to when the transaction finishes.
// Whether the session has a system tray host. `Qt.labs.platform`'s
// `SystemTrayIcon` is backed by Qt Widgets' `QSystemTrayIcon`; creating it on a
// platform without a tray prints two Qt warnings, so QML asks first.
bool kxbps_tray_available();

// Hands one notification to the desktop's notification service.
//
// KNotification's `sendEvent()` drops an event whose component it cannot
// resolve, and the only answer it gives is an immediate `closed`; the session
// bus does not lie, so this calls `org.freedesktop.Notifications.Notify`
// directly. `summary`, `body` and `action` arrive already translated from QML:
// the action is the button label, and translating it here would need a catalog
// in C++.
//
// The popup carries one `default` action. The call is asynchronous; the reply's
// id is remembered, and an `ActionInvoked` for that id calls `owner`'s
// `notificationActivated`, which is how a click brings the window back.
void kxbps_notify(const QString &summary,
                  const QString &body,
                  const QString &icon,
                  const QString &action,
                  QObject &owner);

// Publishes the update count as the dock badge, or clears it when `count` is
// zero.
//
// `QGuiApplication::setBadgeNumber` sends the Unity `LauncherEntry.Update`
// signal on the session bus, keyed by the desktop file name; Plasma's task
// manager listens for it and draws the number on the task button. Stock Qt, no
// custom D-Bus and no window: a hidden `--tray` instance still has no task
// button to badge, which is why the tray icon stays the indicator there.
//
// `int64_t` rather than `qint64`: cxx declares the Rust `i64` argument as
// `::std::int64_t`, and `qint64` is `long long` — a different type on LP64,
// which would be a conflicting declaration.
void kxbps_set_badge(int64_t count);

// Connects `owner` to the notification server's `ActionInvoked` signal once per
// process, so a click on the popup reaches the backend that sent it. Called
// right before the first popup; the Rust method it invokes decides whether the
// id and the action are this process's.
void kxbps_notification_watch_actions(QObject &owner);

// The one entry point a job is created through; `owner` parents it and QML
// releases it when the dialog closes.
QObject *kxbps_start_job(QObject *owner, const QString &payloadJson);

// The mirror view's entry point; forwards to `kxbps_start_job` under a name of
// its own, because two Rust bridges cannot declare one C++ function.
QObject *kxbps_start_mirror_job(QObject *owner, const QString &payloadJson);

// Called on the GUI thread when a job finishes: applies the plan to the store,
// saves it and refreshes the models.
void kxbps_installed_apply_plan(QObject *owner, const QString &itemsJson);

// Tells the owner that a hold job ended, so it re-reads the installed set.
void kxbps_installed_hold_landed(QObject *owner);

// The privileged transaction has started: ask the store owner to stop any
// other job, then take the single-transaction slot. `true` when this job owns
// it; a `false` answer means another job is running and this one must not
// start. Returns whether the process may be spawned.
bool kxbps_job_begin(QObject *job);

// The process exists: the job is no longer cancellable.
void kxbps_job_spawned(QObject *job);

// Records the mirror backend waiting for the next job to end. Called on the GUI
// thread right after `kxbps_start_job` created a mirror job; the owner keeps it
// by address and forwards the end through `kxbps_mirror_apply_landed`.
void kxbps_installed_watch_mirror_job(QObject *owner, QObject *backend);

// Forgets the waiting mirror backend because that object is being destroyed:
// the owner holds it by raw address only, so a backend whose page is torn down
// while its job still runs must clear the slot or the end would call a freed
// object. Null-safe; a no-op when nothing is waiting.
void kxbps_installed_unwatch_mirror_job(QObject *owner);

// One apply job ended: the mirror backend that started it clears its busy flag
// and re-reads the system state. Called on the GUI thread by the store owner,
// which is the only object a job is created with.
void kxbps_mirror_apply_landed(QObject *backend);

// One mirror apply ended, whatever its outcome: the store owner forwards it to
// the mirror backend that started the job, which clears its busy flag and
// re-reads `/etc/xbps.d` and `xbps-query -L`. A package transaction never calls
// this.
void kxbps_installed_mirror_job_landed(QObject *owner);


// Pushes the installed/queue snapshot into the query façade, so `search()` and
// `get()` answer `installedVersion`/`isUpdate` from the same state as the model.
// `updatesJson` and `orphansJson` are the lists xbps reported.
void kxbps_query_backend_set_state(QObject *backend,
                                   const QString &installedJson,
                                   const QString &queueJson,
                                   const QString &updatesJson,
                                   const QString &orphansJson);

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
void kxbps_installed_set_pref_int(QObject *owner, const QString &key, int value);
