// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Synchronous runner for one xbps process.
//
// The command is given as an argv list — never a shell string — and the call
// blocks the calling thread until the process exits or the timeout kills it.
// The read path calls this from a worker thread, so the GUI thread never waits
// on xbps; nothing in this file touches the GUI thread or decides what to do
// with the output.
//
// The same call runs a privileged transaction (`pkexec /usr/bin/xbps-install
// -y …`): the program is whatever the argv list names, and the streaming
// variant hands every output line to the caller while the process runs.
#pragma once

#include <QtCore/QString>

#include "rust/cxx.h"

// Runs `argvJson`, a JSON array of strings with the program first, and returns
// a JSON object:
//
//   {"exitCode": int, "stdout": string, "stderr": string, "timedOut": bool}
//
// `exitCode` is -1 when the program could not be started or died from a
// signal; `timedOut` is true when the timeout killed it. The reply is always a
// valid object: a malformed request or a failed start is reported through
// `exitCode` and `stderr`, never by throwing or returning an empty string.
QString kxbps_run_xbps(const QString &argvJson, int timeoutMs);

// The two hooks of the streaming call, as plain function pointers plus their
// environment, because `rust::Fn` is a value the bridge passes in: an
// `extern "C++"` function may take it, but it cannot be stored or forwarded
// without the trampoline it carries.
typedef void (*KxbpsLineFn)(const QString &line, void *env);
typedef void (*KxbpsSpawnedFn)(void *env);

// Runs `argvJson` like `kxbps_run_xbps`, with two hooks:
//
//   `line`     is called with every whole output line (stdout and stderr, in
//              the order the process wrote them, empty lines included except a
//              leading one). The two channels are read separately, so a line
//              still in a buffer when the process exits is delivered at the
//              end. Passing a null callback keeps every line out of the caller.
//   `spawned`  is called once the process exists and before it is waited on,
//              which is the moment a transaction stops being cancellable.
//   `cancelJson` is the JSON text of the shared cancel flag; `"true"` before
//              the process was spawned kills it without running anything.
//
// The reply is the documented object, and its `stdout`/`stderr` are the same
// text the callbacks saw.
QString kxbps_run_xbps_streaming_hooks(const QString &argvJson,
                                       int timeoutMs,
                                       const QString &cancelJson,
                                       KxbpsLineFn line,
                                       void *lineEnv,
                                       KxbpsSpawnedFn spawned,
                                       void *spawnedEnv);

// The bridge-facing form of the call: cxx maps the two `fn` items of the
// Rust declaration to `rust::Fn`, so this is the signature the generated C++
// will call. It adapts them to the hooks above and runs the same body.
//
// Not `inline`: the generated bridge C++ takes the address of this function.
QString kxbps_run_xbps_streaming(const QString &argvJson,
                                 int timeoutMs,
                                 const QString &cancelJson,
                                 rust::Fn<void(const QString &)> line,
                                 rust::Fn<void()> spawned);
