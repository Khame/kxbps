// C++ side of the mirror view: the model factory and the latency transport.
//
// `cxx-qt-lib` 0.10 ships no binding for `QNetworkAccessManager`, `QNetworkRequest`
// or `QNetworkReply`, so the `HEAD` request the core's probe rules describe is
// performed here. One manager serves a whole probe batch, and every request is
// driven by a local `QEventLoop` on the calling thread — which is the Rust
// worker thread, so the GUI thread never waits on a socket.
//
// Nothing here decides anything: the plain-HTTP rule, the timeout and the
// translation keys all come from `kxbps_core::probe`; this file only reports
// which of the three outcomes happened.
#pragma once

#include <cstddef>
#include <cstdint>

#include <QtCore/QObject>
#include <QtCore/QString>

// Creates the mirror model as a child of `parent`. Its rows are the core's
// static mirror list, one row per entry, in list order.
QObject *kxbps_create_mirror_model(QObject *parent);

// Marks every row as `running` with one `dataChanged` over the whole range, so
// the view keeps its scroll position when a batch starts.
void kxbps_mirror_model_set_all_running(QObject *model);

// Returns the rows a cancelled batch left `running` to `idle`, so a stopped
// sweep does not leave spinners behind.
void kxbps_mirror_model_finish_running(QObject *model);

// Writes one row's probe result and emits `dataChanged` for that row only.
// `stateKey` is `idle`/`running`/`ok`/`fail`, `latencyMs` is only meaningful
// for `ok`, and `errorKey` is a core probe key (`plainHttp`, `timeout`,
// `noResponse`) that is only meaningful for `fail`.
void kxbps_mirror_model_set_result(QObject *model,
                                     int row,
                                     const QString &stateKey,
                                     std::uint32_t latencyMs,
                                     const QString &errorKey);

// Probe transport. `begin` creates the manager the batch reuses; it must be
// called on the thread that runs the batch, and `end` must run on that same
// thread, after the last `kxbps_probe_head`.
std::size_t kxbps_probe_batch_begin();
void kxbps_probe_batch_end(std::size_t handle);

// One `HEAD` request, blocked until the mirror answers or `timeoutMs` elapse.
// Returns 0 when the mirror answered (any HTTP status counts, like `fetch`
// resolving on a 404), 1 on timeout, 2 when nothing answered (DNS, TLS,
// connection refused, ...) and 3 when the application announced shutdown and
// the request was abandoned. A handle of 0 reports 2 instead of crashing.
int kxbps_probe_head(std::size_t handle, const QString &url, std::uint32_t timeoutMs);

// True once the application is shutting down (`QCoreApplication::aboutToQuit`).
// A batch polls this between mirrors and inside a request, so closing the window
// stops the sweep instead of waiting out 34 mirrors at 6.5 s each.
bool kxbps_probe_shutdown_requested();
