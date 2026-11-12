// Synchronous runner for one xbps process.
//
// The command is given as an argv list — never a shell string — and the call
// blocks the calling thread until the process exits or the timeout kills it.
// Task 17b calls this from a worker thread, so the GUI thread never waits on
// xbps; nothing in this file touches the GUI thread or decides what to do with
// the output.
//
// This is the read path only. A privileged transaction (Task 19) streams its
// output while it runs and is not run through here.
#pragma once

#include <QtCore/QString>

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
