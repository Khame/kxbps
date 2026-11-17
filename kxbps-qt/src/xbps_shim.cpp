// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

#include "xbps_shim.h"

#include <QtCore/QElapsedTimer>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QJsonValue>
#include <QtCore/QProcess>
#include <QtCore/QStringList>

namespace {

// xbps prints package descriptions in UTF-8 regardless of the locale, so
// decoding as UTF-8 keeps non-ASCII text intact under `LANG=C` as well.
constexpr int DEFAULT_TIMEOUT_MS = 30000;

// How long a killed process gets to be reaped before the reply is built anyway.
constexpr int KILL_GRACE_MS = 2000;

// How long one wait for output lasts. Short enough that a Cancel requested on
// the GUI thread is noticed promptly, long enough that an idle child does not
// cost a busy loop.
constexpr int POLL_MS = 250;

QString reply(int exitCode, const QString &out, const QString &err, bool timedOut)
{
    QJsonObject object;
    object.insert(QStringLiteral("exitCode"), exitCode);
    object.insert(QStringLiteral("stdout"), out);
    object.insert(QStringLiteral("stderr"), err);
    object.insert(QStringLiteral("timedOut"), timedOut);
    return QString::fromUtf8(QJsonDocument(object).toJson(QJsonDocument::Compact));
}

// A request that cannot be turned into a process is still a reply, so the Rust
// side has one shape to decode.
QString request_error(const QString &reason)
{
    return reply(-1, QString(), reason, false);
}

// Splits whatever arrived into whole lines and hands each one to `line`.
//
// A line can be split across two reads of the pipe, so the tail after the last
// newline stays in `buffer` until the rest of it arrives. Returns the decoded
// text for the final reply, which is the concatenation of every chunk.
QString take_chunk(const char *data, qint64 size, QString &buffer,
                   KxbpsLineFn line, void *lineEnv, bool &initial)
{
    const QString text = QString::fromUtf8(data, static_cast<int>(size));
    buffer += text;

    int start = 0;
    int newline = buffer.indexOf(QLatin1Char('\n'), start);
    // A leading blank line is dropped, as a bare `\n` would otherwise show as
    // an empty row at the top of the log.
    while (newline >= 0) {
        const QString piece = buffer.mid(start, newline - start);
        if (line) {
            if (!piece.isEmpty() || !initial) {
                line(piece, lineEnv);
            }
            initial = false;
        }
        start = newline + 1;
        newline = buffer.indexOf(QLatin1Char('\n'), start);
    }
    buffer.remove(0, start);
    return text;
}

} // namespace

QString kxbps_run_xbps_streaming_hooks(const QString &argvJson,
                                       int timeoutMs,
                                       const QString &cancelJson,
                                       KxbpsLineFn line,
                                       void *lineEnv,
                                       KxbpsSpawnedFn spawned,
                                       void *spawnedEnv)
{
    const QJsonDocument document = QJsonDocument::fromJson(argvJson.toUtf8());
    if (!document.isArray()) {
        return request_error(QStringLiteral("xbps request is not a JSON array"));
    }

    QStringList argv;
    const QJsonArray array = document.array();
    for (const QJsonValue &value : array) {
        if (!value.isString()) {
            return request_error(QStringLiteral("xbps argv contains a non-string"));
        }
        argv.append(value.toString());
    }
    if (argv.isEmpty() || argv.first().isEmpty()) {
        return request_error(QStringLiteral("xbps argv has no program"));
    }

    // The shared cancel flag, as its JSON value: the string "true" means the
    // GUI asked to stop while the process was still being started. Once the
    // process exists a cancel no longer stops anything — a half-applied xbps
    // transaction is worse than a slow one.
    const bool cancelled = cancelJson == QStringLiteral("true");

    QProcess process;
    // Explicit program and argument list: no shell, no word splitting, so a
    // package name with a quote reaches xbps exactly as the plan spelled it.
    process.setProgram(argv.first());
    process.setArguments(argv.mid(1));
    // Separate channels: stdout is the progress report, stderr carries the
    // reason a transaction failed. Both go to the log line by line.
    process.setProcessChannelMode(QProcess::SeparateChannels);
    // Nothing here ever writes to xbps. Without this the child inherits an open
    // stdin pipe that the parent never closes, so a key-trust prompt would
    // block until the timeout instead of reading EOF.
    process.setStandardInputFile(QProcess::nullDevice());

    const int timeout = timeoutMs > 0 ? timeoutMs : DEFAULT_TIMEOUT_MS;
    QElapsedTimer elapsed;
    elapsed.start();

    process.start();
    // pkexec prints its own diagnostics on stderr and exits with the code the
    // policy decided; nothing here interprets them. A missing program is
    // reported through the reply, never thrown.
    if (!process.waitForStarted(timeout)) {
        return request_error(process.errorString());
    }
    if (spawned) {
        spawned(spawnedEnv);
    }
    if (cancelled) {
        // The job was cancelled before the process existed; nothing has run
        // yet, so killing it here cannot leave the system half-configured.
        process.kill();
        process.waitForFinished(KILL_GRACE_MS);
        return reply(-1, QString(), process.readAllStandardError(), false);
    }

    QString out;
    QString err;
    QString outBuffer;
    QString errBuffer;
    bool outInitial = true;
    bool errInitial = true;
    bool timedOut = false;

    while (process.state() != QProcess::NotRunning) {
        const int remaining = timeout - static_cast<int>(elapsed.elapsed());
        if (remaining <= 0) {
            timedOut = true;
            break;
        }
        const int wait = qMin(remaining, POLL_MS);
        process.waitForReadyRead(wait);
        const QByteArray chunkOut = process.readAllStandardOutput();
        out += take_chunk(chunkOut.constData(), chunkOut.size(), outBuffer, line, lineEnv, outInitial);
        const QByteArray chunkErr = process.readAllStandardError();
        err += take_chunk(chunkErr.constData(), chunkErr.size(), errBuffer, line, lineEnv, errInitial);
    }

    if (timedOut) {
        // Killing a real transaction is not this path's decision: the Rust
        // side treats a timed-out transaction as a failure and leaves the
        // store alone. The process is still killed so it cannot outlive the
        // reply.
        process.kill();
        process.waitForFinished(KILL_GRACE_MS);
    }

    // Whatever the child wrote without a trailing newline.
    const QByteArray tailOut = process.readAllStandardOutput();
    out += take_chunk(tailOut.constData(), tailOut.size(), outBuffer, line, lineEnv, outInitial);
    const QByteArray tailErr = process.readAllStandardError();
    err += take_chunk(tailErr.constData(), tailErr.size(), errBuffer, line, lineEnv, errInitial);
    if (line) {
        if (!outBuffer.isEmpty()) {
            line(outBuffer, lineEnv);
        }
        if (!errBuffer.isEmpty()) {
            line(errBuffer, lineEnv);
        }
    }

    const int exitCode =
        process.exitStatus() == QProcess::NormalExit ? process.exitCode() : -1;
    return reply(timedOut ? -1 : exitCode, out, err, timedOut);
}

namespace {

// The two adapters that turn the bridge's `rust::Fn` back into a C callback.
//
// `env` is the address of the `rust::Fn` local in
// `kxbps_run_xbps_streaming`, which outlives the whole shim call: nothing is
// copied or decoded here, so no layout of `rust::Fn` is assumed. Calling it
// through its own `operator()` is the only sanctioned entry point.
void bridge_line(const QString &text, void *env)
{
    (*static_cast<rust::Fn<void(const QString &)> *>(env))(text);
}

void bridge_spawned(void *env)
{
    (*static_cast<rust::Fn<void()> *>(env))();
}

} // namespace

QString kxbps_run_xbps_streaming(const QString &argvJson,
                                 int timeoutMs,
                                 const QString &cancelJson,
                                 rust::Fn<void(const QString &)> line,
                                 rust::Fn<void()> spawned)
{
    // The bridge passes each hook as a value; the adapters above need a stable
    // address for the duration of the call, which is exactly this function.
    return kxbps_run_xbps_streaming_hooks(argvJson,
                                          timeoutMs,
                                          cancelJson,
                                          bridge_line,
                                          &line,
                                          bridge_spawned,
                                          &spawned);
}

QString kxbps_run_xbps(const QString &argvJson, int timeoutMs)
{
    // The read path never cares about partial output: it wants the whole text
    // once the command is done, which is exactly what the streaming call
    // answers when nobody listens to the lines.
    return kxbps_run_xbps_streaming_hooks(argvJson,
                                          timeoutMs,
                                          QStringLiteral("false"),
                                          nullptr,
                                          nullptr,
                                          nullptr,
                                          nullptr);
}
