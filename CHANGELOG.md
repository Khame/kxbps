# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses semantic versioning.

## [Unreleased]

### Added

- Update-check interval in Settings: 30 minutes, 1, 3, 6, 12 or 24 hours,
  default 6 hours. The tray re-check follows the same preference.
- Fresh update checks through `xbps-install -M -un`, which fetches the
  repository indexes into memory without root and writes nothing. The
  Upgradable page has a Check for updates button, and the status bar names a
  failed check and the age of the last good list instead of claiming the system
  is current.
- Desktop notification when updates are available: one per set of updates, one
  reminder after three hours, then only the tray icon. Clicking the popup opens
  the window on the Upgradable list, and two Settings switches decide what is
  announced.
- Tray icon in two states: the plain stack and the stack with an arrow while
  updates wait. The icon is a theme name, so the panel recolours it, and the
  tooltip carries the count.
- The dock icon carries the number of available updates. The app publishes the
  Unity `LauncherEntry` badge through stock Qt, and Plasma's task manager draws
  it on the task button; zero clears it. A windowless `--tray` instance has no
  task button, so the tray icon stays the indicator there.
- The new update-check and notification strings are translated in the German,
  Spanish, French, Italian, Dutch, Polish and Brazilian Portuguese catalogs.

### Changed

- New icon set: a stack of packages for the application, and a monochrome tray
  icon that switches to the update variant while updates are waiting. The tray
  icon is a theme name, so Plasma recolours it for the panel.
- Settings has the start-time switch back: "Announce available updates on
  startup" keeps a start silent and lets a check run from the window report,
  while "Use desktop notifications" covers the popups as a whole.

## [1.0.0] - 2026-11-13

First release.

### Added

- Package table with filters for installed, upgradable, available, queued,
  explicit, orphaned and held packages, each read from xbps on demand.
- Queue that shows the dry-run plan and applies it through `pkexec` and polkit,
  with the exact commands in the log drawer.
- Details pane for one package: installed and repository versions, sizes,
  dependencies and what requires it.
- Mirrors page that lists the mirror catalogue, shows the system mirror and
  switches it, keeping a backup of the previous configuration file.
- Repositories page that lists the files under `/etc/xbps.d` and adds or removes
  the `20-kxbps-*.conf` files KXbps owns.
- Search across the configured repositories, separate from the installed set.
- Translations into German, Spanish, French, Italian, Dutch, Polish and
  Brazilian Portuguese, Kirigami theme colours throughout, and a window that
  fits the screen at any scale factor.
- Tray icon and an optional notification when updates are available.

### Notes

- The catalogs other than Polish are a first pass and have not been reviewed by
  native speakers; corrections belong in `po/<lang>.po`.
- The mirror and repository helpers run `xbps-install -S` after writing a file,
  because `xbps-query` skips a repository without an index without a word. A
  failed sync leaves the configuration in place and reports a warning.
