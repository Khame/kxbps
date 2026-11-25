# KXbps

A KDE Plasma front end for xbps, the package manager of Void Linux.

KXbps runs the tools that are already on the system (`xbps-query`,
`xbps-install`, `xbps-remove`) and shows what they report. There is no bundled
catalog: a fresh install starts with an empty store, and every list is read from
xbps when a screen asks for it. The one cache is the installed map from the last
`xbps-query -l`.

![KXbps showing the installed packages, three queued for update](https://github.com/Khame/kxbps/releases/download/v0.12.3/en-dark-queue.png)

## What the window has

The sidebar filters the package table:

| Filter | Read from |
| --- | --- |
| Installed | `xbps-query -l` |
| Upgradable | `xbps-install -un`, then `xbps-install -M -un` on a check |
| Available | `xbps-query -M -Rs ''` at start-up, then filtered in memory while you type; `xbps-query -Rs <text>` for regex syntax or when nothing matches locally |
| Marked | the queue file, `~/.local/share/kxbps/queue.json` |
| Explicit | `xbps-query -m` |
| Orphans | `xbps-query -O` |
| On hold | `xbps-query -H` |

The tools below the filters open their own pages: Mirrors (the mirror list and
the system mirror), Repositories (the files under `/etc/xbps.d`), Help and
Settings.

The mirror page writes one line, `repository=<mirror>/current<tail>`. The part
after `/current` belongs to the system rather than to the mirror: `musl` on a
musl install, `multilib/nonfree` when those repositories are in use. A mirror
switch keeps it, taken from the file it replaces or from the packaged
`/usr/share/xbps.d/00-repository-main.conf` when the local file does not exist.

A row adds a package to the queue or takes it out. The bar at the bottom of the
window carries the queue size, the Apply button and the state of the last job.
The `Log` button opens the raw output of the running or last transaction and of
the update checks and mirror probes of this session, and the drawer's header
copies those lines to the clipboard. Before any of them has run the drawer says
there is nothing to show yet.

## Update checks

`xbps-install -un` reads the local index, so the Upgradable list is as old as the
last repository sync. The timer in Settings (30 minutes, 1, 3, 6, 12 or 24
hours), the start of the app and the moment the window is shown again run
`xbps-install -M -un` instead: xbps fetches the repository indexes into memory,
writes nothing and needs no root. The Available list is refreshed the same way
and on the same clock, so a package added to a repository appears without a
sync.

The **Check for updates** button on that page is the real sync: `xbps-install -S`
through polkit, which writes the indexes to disk and asks for the password. It is
the only read that needs one, and after it a terminal sees the same packages the
window does.

A check that cannot read a repository leaves the list as it was and says why,
together with the age of the last successful check; it never claims the system
is current on stale data. With no network connection the automatic check does
not start, and the app says so in the status bar.

## Tray and notifications

The tray icon is a theme name, `kxbps-tray` or `kxbps-tray-updates`, so Plasma
recolours it for the panel; the update variant adds an arrow and the tooltip
carries the count. The icon is there whether the window is open or not.

Closing the window hides it into the tray when a status notifier host exists,
and the icon brings it back in the state it was left in.

One desktop notification announces a set of updates. A single reminder follows
after three hours, and after that only the icon keeps reporting. Clicking the
popup opens the window on the Upgradable list. Two Settings switches decide what
is announced: "Announce available updates on startup" keeps a start silent while
a check you run from the window still reports, and "Use desktop notifications"
turns the popups off altogether. Neither touches the tray icon.

The dock button for the running window carries the number of updates as a badge.
The app publishes the Unity `LauncherEntry` count through Qt, which the Plasma
task manager already reads; zero clears it. A `--tray` instance has no window, so
there is no dock button to badge and the tray icon carries the news alone.

## Privileges

Reading needs no root. Every write goes through `pkexec` and one of five polkit
actions:

| Action | What it does |
| --- | --- |
| `io.github.khame.kxbps.install` | `xbps-install` for the queue |
| `io.github.khame.kxbps.remove` | `xbps-remove` for the queue |
| `io.github.khame.kxbps.hold` | `xbps-pkgdb -m hold` or `unhold` for one package |
| `io.github.khame.kxbps.mirror` | writes `/etc/xbps.d/00-repository-main.conf` and syncs the indexes |
| `io.github.khame.kxbps.repo` | adds or removes one `20-kxbps-*.conf` file and syncs the indexes |

The helpers behind the last two actions live in `/usr/libexec`. The window
starts them; there is nothing to run by hand.

## Requirements

Runtime: Qt 6 base and declarative, the `qt6-svg` imageformat plugin, the
`qt6-plugin-tls-openssl` TLS backend, KF6 Kirigami, KI18n, Sonnet and the
desktop style for Qt Quick Controls, the StatusNotifierItem module for the
tray, polkit, xbps.

Build: Rust 1.99 or newer, and `lld`. cxx-qt asks the linker to use lld, and GNU
ld drops the initializers the generated C++ needs, so without lld the link fails
with undefined references to `cxx_qt_init_crate_*`.

## Install

Khame's signed repository carries the package for `x86_64` and `x86_64-musl`:
[INSTALL.md](INSTALL.md) has the repository line, the key fingerprint and the
commands. There is no package in the official Void repository.

The musl tree is a separate repository, `…/void-packages/musl`, with the same
key and the same package. xbps reads `<repository>/<arch>-repodata`, so the two
indexes never mix.

It also builds from a checkout; INSTALL.md has the dependencies, the install
script and what it puts where.

## Run

```sh
kxbps
```

`kxbps --section mirrors` opens one page directly (`packages`, `mirrors`,
`repositories`, `settings`, `help`), and `kxbps --filter queued`
opens the package table on one filter (`installed`, `updates`, `all`, `queued`,
`explicit`, `orphans`, `held`).

`kxbps --help` prints the same list without starting the window, `kxbps
--version` prints the version, and `man kxbps` carries both. Neither needs a
display.

State lives in `~/.local/share/kxbps/` (`queue.json`, `prefs.json`,
`mirror.json`), mode 0700.

## Build and test

The dependencies and the install script are in [INSTALL.md](INSTALL.md).

```sh
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The UI smoke test needs no display:

```sh
QT_QPA_PLATFORM=offscreen timeout 5 ./target/debug/kxbps
```

The timeout kills it after five seconds and nothing is written to stderr.

## Troubleshooting

A repository without a local index is skipped by `xbps-query` without a word, so
every list comes back short. KXbps syncs the indexes after a mirror or
repository change, and `sudo xbps-install -S` does the same by hand. `xbps-query
-L` prints the package count of each repository, and `-1` means that repository
has no index.

If the Repositories page says the helper is missing, the installation did not
finish. `/usr/libexec/kxbps-repo-apply` and the policy file have to be on disk.

## Translations

The interface follows the system locale (`LANGUAGE`, `LC_*`). There is no
language switch inside the application.

The polkit password dialog follows the same locale. A `.policy` file has no
gettext catalog, so every action carries its description and message for each
language below next to the English element that stays the default.

| Language | Catalog |
| --- | --- |
| German | `po/de.po` |
| Spanish | `po/es.po` |
| French | `po/fr.po` |
| Italian | `po/it.po` |
| Dutch | `po/nl.po` |
| Polish | `po/pl.po` |
| Brazilian Portuguese | `po/pt_BR.po` |

`scripts/extract-i18n.sh` rebuilds `po/kxbps.pot` from the QML and Rust sources
and checks every catalog for missing, empty, fuzzy and untranslated entries and
for placeholder mismatches. `--write` also merges the template into `po/pl.po`;
the other catalogs are merged with `msgmerge` by hand.

A new language needs one `po/<lang>.po` with its own `Plural-Forms` header. The
build installs every catalog it finds, so nothing else changes.

## Documentation

The Help page in the window describes the queue, the update checks,
authorisation and where the state lives; it is translated with the rest of the
interface. `man kxbps` and `kxbps --help` cover the command line.

[docs/xbps-integration.md](docs/xbps-integration.md) lists which xbps command
feeds which screen and what is parsed from its output.
[VOID_PACKAGING.md](VOID_PACKAGING.md) is the maintainer's note: the template,
the version invariants, the source tarball and the steps of a release.

`scripts/release-notes.py` prints the body of a GitHub release from
`CHANGELOG.md`, so the release page and the tree carry one text.

## License

Copyright © 2026 Łukasz "Khame" Linhard.

KXbps is free software: you can redistribute it and/or modify it under the terms
of the GNU General Public License as published by the Free Software Foundation,
either version 3 of the License, or (at your option) any later version. See
[LICENSE](LICENSE) for the full text.

The program links Qt 6 and KF6 (LGPL) and uses Rust crates under MIT and
Apache-2.0 licenses; their notices travel with the binary.
