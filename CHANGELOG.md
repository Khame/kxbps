# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses semantic versioning.

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
- Polish translation, Kirigami theme colours throughout, and a window that fits
  the screen at any scale factor.
- Tray icon and an optional notification when updates are available.

### Notes

- The mirror and repository helpers run `xbps-install -S` after writing a file,
  because `xbps-query` skips a repository without an index without a word. A
  failed sync leaves the configuration in place and reports a warning.
