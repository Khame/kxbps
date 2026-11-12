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

} // namespace

QString khaxbps_run_xbps(const QString &argvJson, int timeoutMs)
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

    QProcess process;
    // Explicit program and argument list: no shell, no word splitting, so a
    // search pattern with spaces or a package name with a quote reaches xbps
    // exactly as the user typed it.
    process.setProgram(argv.first());
    process.setArguments(argv.mid(1));
    // Separate channels: stdout is data for the parsers, stderr is the error
    // text the UI shows when the command fails.
    process.setProcessChannelMode(QProcess::SeparateChannels);
    // Nothing here ever writes to xbps. Without this the child inherits an open
    // stdin pipe that the parent never closes, so a prompt would block until
    // the timeout instead of reading EOF.
    process.setStandardInputFile(QProcess::nullDevice());

    const int timeout = timeoutMs > 0 ? timeoutMs : DEFAULT_TIMEOUT_MS;
    QElapsedTimer elapsed;
    elapsed.start();

    process.start();
    if (!process.waitForStarted(timeout)) {
        return request_error(process.errorString());
    }

    const int remaining = timeout - static_cast<int>(elapsed.elapsed());
    if (remaining <= 0 || !process.waitForFinished(remaining)) {
        // The read commands touch only the local database and indexes, so a
        // hard kill cannot leave the system half-configured. Killing a real
        // transaction is Task 19's problem, not this path's.
        process.kill();
        process.waitForFinished(KILL_GRACE_MS);
        const QString out = QString::fromUtf8(process.readAllStandardOutput());
        const QString err = QString::fromUtf8(process.readAllStandardError());
        return reply(-1, out, err, true);
    }

    const QString out = QString::fromUtf8(process.readAllStandardOutput());
    const QString err = QString::fromUtf8(process.readAllStandardError());
    const int exitCode =
        process.exitStatus() == QProcess::NormalExit ? process.exitCode() : -1;
    return reply(exitCode, out, err, false);
}
