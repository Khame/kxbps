# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses semantic versioning.

## [0.13.0_3] - 2026-11-25

Packaging revision: the `v0.13.0` tag plus a backport patch.

### Fixed

- The update list reloads when a transaction ends. Applied rows stayed on
  screen until the next check or a restart.
- The reload also runs when xbps exits non-zero after doing the work, as in a
  `replaces` cleanup.

## [0.13.0] - 2026-11-21

### Added

- Details pane: homepage link, maintainer, architecture and install date.
- A row checkbox queues that row's action without opening it.
- The Orphaned page counts the packages and can mark them all for removal.
- Busy indicator while the package list loads.
- Removal also drops unneeded dependencies (`-R`).
- About links the project website.

### Fixed

- New dependencies of an update no longer appear in the update list.
- The log keeps following new lines.
- A local search stays inside the active filter.

### Changed

- Translations for the new strings; the non-Polish catalogs had lost some.

### Notes

- The package homepage points at the project site.

## [0.12.3] - 2026-11-18

### Removed

- The "Minimize to the tray" switch. KWin on Wayland does not report a
  compositor minimize, so it did nothing there. Closing the window still hides
  it to the tray.

### Notes

- A `prefs.json` from 0.12.2 still loads; the retired `minimizeToTray` key is
  dropped on the next save.

## [0.12.2] - 2026-11-17

### Added

- The Mirrors page names the repository xbps reads, tail included:
  `xbps reads https://repo-default.voidlinux.org/current/musl`.
- Settings has a "Minimize to the tray" switch, off by default.

### Fixed

- Switching mirrors on musl wrote a glibc repository. The helper now keeps the
  path tail (`musl`, `multilib/nonfree`) from the file it replaces.
- The Mirrors page reported a missing `/etc/xbps.d/00-repository-main.conf` on
  a fresh install, where xbps runs on the packaged file in `/usr/share/xbps.d`.

## [0.12.1] - 2026-11-17

### Fixed

- Apply needed a second click on a queue of updates. xbps prints "already
  installed" for a package it took as a dependency and lists the same package
  as an update; the plan read that as "nothing to do" and dropped the entry.
- The details pane kept describing a package the list no longer showed after a
  filter or search change.
- The log drawer set a fixed height inside a column, so a short window pushed
  the bar past the bottom edge.
- The polkit dialog showed English in a Polish session. Each action now carries
  the seven languages `po/` ships.
- `build.rs` watches `qml/`, so a QML edit reaches the binary.

### Changed

- The sidebar entry reads "Aktualizacje", the same word as the page title.
- The help page is a centred column with rules between sections.

## [0.12.0] - 2026-11-15

### Added

- The Updates page syncs: `xbps-install -S` through polkit, writing the indexes
  to disk.

### Fixed

- The Available list read the index from disk, so a new package stayed
  invisible until a sync. It now refreshes against the repositories on the same
  clock as the update check.
- A typed name filters the listing in memory, so the fresh listing and the
  search no longer disagree.
- The details pane stayed empty for packages whose facts were cached; it now
  follows every selection.
- Download size is read from the repository first, then from the installed
  record.

## [0.11.0] - 2026-11-15

### Added

- The mirror table shows the address xbps uses and the tier `xmirror` prints.
- One button copies the log lines.

### Changed

- The mirror page uses the window chrome: region filter on the toolbar, count
  in the footer, one line per row.
- The sort heading is `Ping`, muted like the others.
- The sidebar and the log use the KXbps icon set from one registry.
- The About page describes the real build; the Help page the real workflow.
- Polish says "pakiet" everywhere.

### Fixed

- A mirror row without a country no longer pushes its cells left.
- The sort arrow and the accessible name follow the sort direction.
- Hold runs `xbps-pkgdb`, so the system and the store agree.
- The probe covers the rows the filter leaves visible; Apply stays usable.
- The log button and the mirror badge stay readable on the dark strip.

## [0.10.1] - 2026-11-15

### Changed

- The details pane is a header, a fact grid and one row of actions.
- Dependencies and "required by" are one name per token in two aligned columns.
- The file list is a dialog behind Files; the Query page is gone.
- Buttons hug their label, and the font is capped at 10 pt.
- Buttons are flat except Apply and Apply to system.
- A button no longer underlines its mnemonic letter.
- Polish says "pakiet", not "paczka".

### Fixed

- The details pane asks the repository index for the file list of a package
  that is not installed.
- The Available page answers every repository-listing request.

## [0.10.0] - 2026-11-14

First public release, in beta: the window, the store format and the command
line can still change.

### Added

- Package table with filters for installed, upgradable, available, queued,
  explicit, orphaned and held packages, read from xbps on demand.
- Queue with the dry-run plan, applied through `pkexec` and polkit, commands
  shown in the log.
- Details pane: versions, sizes, dependencies and what requires the package.
- Mirrors page: catalogue, system mirror, switching, backup of the old file.
- Repositories page: files under `/etc/xbps.d`, adding and removing the
  `20-kxbps-*.conf` files.
- Search across the configured repositories.
- Update-check interval in Settings, from 30 minutes to 24 hours.
- Fresh checks through `xbps-install -M -un`, which writes nothing; the status
  bar names a failed check and the age of the last good list.
- Desktop notification for available updates, one per set and one reminder
  after three hours.
- Tray icon in two states, drawn from the theme so the panel recolours it.
- Dock badge with the update count through the Unity `LauncherEntry`.
- Help page in the window.
- `--help` and `--version` before Qt starts, plus a `kxbps(1)` page.
- The update check's xbps output in the log.
- Translations into German, Spanish, French, Italian, Dutch, Polish and
  Brazilian Portuguese.

### Changed

- New icon set, including the tray icon that switches while updates wait.
- Settings: "Announce available updates on startup" and "Use desktop
  notifications".
- The log drawer is called `Log`.
- The log bar sits on the bottom edge.

### Fixed

- The log drawer opens before the first transaction and says there is nothing
  to show yet.

### Notes

- Catalogs other than Polish are a first pass, not reviewed by native speakers.
- The mirror and repository helpers run `xbps-install -S` after writing a file;
  a failed sync leaves the configuration and reports a warning.
