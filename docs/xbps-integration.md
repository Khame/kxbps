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
| `xbps-query -M -Rs <pattern>` | the same search, fresh indexes | same shape |
| `xbps-query -s <pattern>` | search, installed only | same shape |
| `xbps-query -L` | Repositories tab | count, URL, `(RSA signed)` / `(RSA maybe-signed)` / no note |
| `xbps-query -p <props> <pkg>` | details pane | one raw value per line, by name |
| `xbps-query -R -p <props> <pkg>` | details pane, repository facts | same |
| `xbps-query -x <pkg>` | details pane, dependencies | one pkgver per line |
| `xbps-query -X <pkg>` | details pane, required by | one pkgver per line |
| `xbps-query -p installed_size -s ''` | sizes | `<pkgver>: <humanised size>` |
| `xbps-query -R -p filename-size -s ''` | download column | `<pkgver>: <humanised size> (<repository>)` |

The two property takes describe the same package from two places, and they do
not always agree: the plain take reads the installed record, the `-R` take reads
the repository index. Every field is taken from the installed record first and
from the index only where that record is silent, because the installed record is
what is on the system. The download size is read from both, so a package the
index does not know at that version — a local build, a removed repository — still
shows it; the repository field follows the same rule and therefore names where
the package came from, not where it would come from today. A version the index
knows only newer does not replace the installed one.

The bulk size takes are the one place where a failed command does not fail the
screen: an index that was never synced leaves the size columns empty and the
package list stays. Every other read is all or nothing, because a half-read list
would show a state xbps did not report. A line the parser does not understand
becomes an error with its number and text, never a skipped row. KXbps reads the
size fields and downloads nothing itself; the packages are fetched by
`xbps-install` during a transaction.

## The index on disk and the index in memory

`xbps-query -Rs` answers from the repository index cached in `/var/db/xbps`,
which only moves when something syncs it. A package added to a repository is
therefore invisible to that read until an install (which runs `-S` first) or a
terminal `xbps-install -S`.

Two reads close that gap without root and without writing:

| Command | Where | What it changes |
| --- | --- | --- |
| `xbps-install -M -un` | the update check | nothing; indexes fetched into memory |
| `xbps-query -M -Rs <pattern>` | the Available listing | nothing; same, for the package list |

Both run on the same clock (`AUTO_GAP_MS`, five minutes, plus the interval in
Settings), and neither is doubled while one is in flight. The listing keeps the
disk answer on screen until the fresh one lands, and a failed download leaves
the clock alone so the next visit tries again.

The one action that writes the disk index is the button on the Updates page:
`pkexec /usr/bin/xbps-install -S` under the install action's authorization. After
it, `xbps-query -Rs` in a terminal sees the same packages the window does.

A typed pattern is answered from that same listing while it is in memory: the
model matches names and descriptions as text, exact name first, and asks
`xbps-query -Rs` only when the pattern carries regex syntax or the local match
finds nothing. So a package added to a repository is found by name without a
second process, and a wildcard search still gets xbps' own answer.

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

That file carries one line, `repository=<mirror>/current<tail>`. The tail after
`/current` names the repository set of the system rather than of the mirror:
`musl` on a musl install, `multilib/nonfree` where those are enabled, nothing on
plain glibc. A mirror switch rewrites the host and keeps the tail, read from the
file it replaces or from `/usr/share/xbps.d/00-repository-main.conf` when `/etc`
has none. A tail that is not a plain path is refused before any write. The read
side follows the same merge rule: `/etc/xbps.d` wins over `/usr/share/xbps.d`,
and the packaged file answers when the local one is absent.

One dry-run line is not what it looks like. xbps names a package it has already
taken as a dependency of an earlier argument with ``Package `x' already
installed.`` on stderr, and prints the same package as an `update` row at the
same time. KXbps treats that sentence as satisfied only for the names the table
does not carry, so the entry stays in the queue. A non-zero exit counts as a
notice only when every line on stderr is that sentence; an aborted transaction
carries the same sentence, and reading it as "nothing to do" would hide the
failure and drop the entry it named.

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
