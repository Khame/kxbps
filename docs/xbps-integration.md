# How KXbps talks to xbps

KXbps runs the xbps tools already on the system and shows what they print.
Everything on screen comes from their output, parsed as text: the plan is xbps'
dry run, the sizes come from `xbps-query`, and the installed map is a cache of
the last `xbps-query -l`.

This page records which command runs for which screen, what is read from its
output, and the exit codes the program relies on. It was written against xbps
0.60.7 on Void Linux; the parsers come from captures of that version.

## Commands the read layer runs

None of these needs root, and none of them changes a file.

| Command | Screen | What is parsed |
| --- | --- | --- |
| `xbps-query -l` | package list, installed state | `ii`/`uu`, `name-version_revision`, description |
| `xbps-install -un` | updates, update count | `<pkgver> <action> <arch> <repository> <installed size> <download size>` |
| `xbps-query -m` | "explicit" flag | one `name-version_revision` per line |
| `xbps-query -H` | "held" flag | one `name-version_revision` per line |
| `xbps-query -O` | orphans | one `name-version_revision` per line |
| `xbps-query -Rs <pattern>` | search, all repositories | `[*]`/`[-]`, `name-version_revision`, description |
| `xbps-query -s <pattern>` | search, installed only | same shape |
| `xbps-query -L` | Repositories tab | count, URL, `(RSA signed)` / `(RSA maybe-signed)` / no note |
| `xbps-query -p <props> <pkg>` | details pane | one raw value per line, by name |
| `xbps-query -R -p <props> <pkg>` | details pane, repository facts | same |
| `xbps-query -x <pkg>` | details pane, dependencies | one pkgver per line |
| `xbps-query -X <pkg>` | details pane, required by | one pkgver per line |
| `xbps-query -p installed_size -s ''` | sizes | `<pkgver>: <humanised size>` |
| `xbps-query -R -p filename-size -s ''` | download column | `<pkgver>: <humanised size> (<repository>)` |

The bulk size takes are the one place where a failed command does not fail the
screen: an index that was never synced leaves the size columns empty and the
package list stays. Every other read is all or nothing, because a half-read list
would show a state xbps did not report. A line the parser does not understand
becomes an error with its number and text, never a skipped row. KXbps reads the
size fields and downloads nothing itself; the packages are fetched by
`xbps-install` during a transaction.

## Commands the plan and the transaction run

`xbps-install -n …` and `xbps-remove -n …` are the dry runs. They print the same
six-field table and change nothing. KXbps shows that table as the plan; it does
not compute the steps, order them, or add a package xbps did not print.

The real transaction is always `pkexec /usr/bin/xbps-install -y <pkgvers>` or
`pkexec /usr/bin/xbps-remove -y <pkgvers>`, built as an argument list and never
as a shell string. The mirror helper writes `/etc/xbps.d/00-repository-main.conf`
and nothing else; it does not sync the repository index, because the URL is the
only thing a mirror change decides. `00-repository-main.conf` is the only file
KXbps ever writes outside its own state directory.

## Exit codes

Read commands:

| Code | Meaning to KXbps |
| --- | --- |
| `0` | the output is parsed; a plan may still be empty |
| `-1` sent by the runner | the process could not start (xbps missing) |
| any other | xbps' own error, shown with its code and the last stderr line |

The runner reports a timeout separately from a non-zero exit, because both
arrive as `-1` from the process and only the timeout flag says which happened.

Package transaction, as the job dialog reads it:

| Code | Meaning |
| --- | --- |
| `0` | done |
| `126` | the polkit dialog was dismissed, or polkit could not start the program |
| `127` | not authorized |
| `-1` | `pkexec` is missing or not executable |
| `19` | on a removal dry run: the transaction is blocked; the stderr lines name the packages that would break |
| `2` | on a dry run: the package is unknown, or (removal) not installed |

Mirror helper (`/usr/libexec/kxbps-mirror-apply`):

| Code | Meaning |
| --- | --- |
| `0` | written and verified |
| `2` | URL refused before any write (plain HTTP, malformed, not a base URL) |
| `3` | the configuration could not be written |
| `4` | `xbps-query -L` does not list the new URL; the previous file was restored |
| `5` | the verification failed and so did the revert |

`126`, `127` and `-1` keep their `pkexec` meaning for the helper as well.

## Locks

xbps takes a lock on the package database and on each repository index. A second
xbps run does not fail on it: it prints `package database locked, waiting...` or
`repository locked: <url>: waiting...` and waits for the holder. So a lock is a
slow start, not an error, and the window must not present it as a failure.

The holder is not necessarily KXbps. It can be a terminal `xbps-install`, a
package build, or another user's root transaction; nothing in the message names
the owner. KXbps' own single-transaction slot is a separate concern: it stops
this application from starting two of its own transactions, and it cannot see
anybody else's.

`kxbps_core::plan::lock_waited_on` recognises both phrasings. The Qt layer does
not consume it yet, so today a lock shows up as a job that appears stuck; see
the gap list in the Task 22 report.

## When xbps is not there

If `xbps-query` cannot be started, every read fails with the same key and the
window says which program is missing. Nothing from a previous run is cached to
hide it: a stale list that looks live is worse than an empty one.
