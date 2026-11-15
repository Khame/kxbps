# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses semantic versioning.

## [0.12.0] - 2026-11-15

### Added

- The Updates page has the real sync: `xbps-install -S` through polkit, which
  writes the repository indexes to disk and asks for the password. The toolbar
  Refresh is not shown on that page, because it only re-reads what is already
  there.

### Fixed

- The Available list read the repository index from disk, which nothing in the
  window moved: a package added to a repository stayed invisible until an
  install or a terminal `xbps-install -S`. The listing is now refreshed against
  the repositories on the same clock as the update check, and the footer says
  so while it runs.
- Typing a name on that page asked xbps again, so the fresh listing and the
  search could disagree. A typed word now filters the listing in memory, exact
  name first; only a pattern with regex syntax, or a local match that finds
  nothing, goes to `xbps-query -Rs`.
- The details pane stayed empty for any package whose facts were already
  cached: the pointer that says which package the pane is showing moved only
  when a read started. It now follows every selection, and the change is
  announced so the pane redraws.
- The pane asked for the download size only in the repository take, so a
  package the index does not know at that version showed `—` while the
  installed record had the number. The row now reads download first, then the
  size on disk.

## [0.11.0] - 2026-11-15

### Added

- The mirror table carries the address xbps is pointed at and the tier
  `xmirror` prints, `T1`/`T2`: two mirrors in the same city are no longer two
  rows that read the same, and a community mirror is not mistaken for an
  upstream one.
- The log drawer copies its lines with one button, the way the About page
  copies the library list. A report about a failed transaction usually starts
  by pasting the output somewhere else.

### Changed

- The mirror page uses the window's chrome: the region filter sits on the
  toolbar, the count in the page footer, and the rows are one line with a
  hairline between them.
- The sort heading is `Ping`, in the last column and muted like every other
  heading: the number is what the sweep produces, and the row's own facts come
  first.
- The sidebar and the log draw the KXbps symbolic icon set, and every icon name
  lives in one registry.
- The About page is rebuilt around the real build, and the Help page describes
  the workflow the tool actually has.
- Polish says "pakiet" everywhere; the package column still said "Paczka".

### Fixed

- A mirror row with no country pushed that row's address, region, tier and
  state to the left, because its server cell could not grow.
- The sort direction reached the list but never the heading: the arrow and the
  accessible name kept saying "ascending".
- Hold runs `xbps-pkgdb` instead of flipping a local flag, so the system and the
  store agree.
- The mirror probe covers the rows the filter leaves visible, and Apply stays
  usable while it runs.
- The log button and the mirror badge stay readable on the dark strip.

## [0.10.1] - 2026-11-15

### Changed

- The details pane is a header, a fact grid and one row of actions: the queue
  action on the left, Files and Hold on the right. Dependencies and "required
  by" are one name per token in two aligned columns, so a version constraint
  stays next to the package it belongs to instead of running into a paragraph.
- The file list lives in a dialog behind Files, and the Query page is gone: the
  page repeated what the rest of the window already shows, and the one
  diagnostic it still owned (`-f`) now sits next to the package it describes.
- Buttons hug their label at 85% of the theme font, and the application font is
  capped at 10 pt, so a toolbar row no longer carries a tall empty box around
  every label.
- Buttons are flat. Only Apply and Apply to system keep a frame: the two
  controls that change the machine should not read like the rest.
- A button no longer underlines the letter of its mnemonic; Alt+letter still
  activates it.
- Polish says "pakiet", not "paczka" — the old word means a parcel.

### Fixed

- The details pane asks the repository index for the file list of a package
  that is not installed, instead of answering with exit code 2 and an empty
  detail.
- The Available page answers every repository-listing request. The backend used
  to skip a pattern it had already delivered, so every visit after the first
  waited for an answer that never came.

## [0.10.0] - 2026-11-14

First public release, in beta: the window, the store format and the command
line can still change.

### Added

- Package table with filters for installed, upgradable, available, queued,
  explicit, orphaned and held packages, each read from xbps on demand.
- Queue that shows the dry-run plan and applies it through `pkexec` and polkit,
  with the exact commands in the Log drawer.
- Details pane for one package: installed and repository versions, sizes,
  dependencies and what requires it.
- Mirrors page that lists the mirror catalogue, shows the system mirror and
  switches it, keeping a backup of the previous configuration file.
- Repositories page that lists the files under `/etc/xbps.d` and adds or removes
  the `20-kxbps-*.conf` files KXbps owns.
- Search across the configured repositories, separate from the installed set.
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
- Help page in the window: the queue, the update checks, authorisation and
  where the state lives, translated with the rest of the interface.
- `--help` and `--version` on the command line, answered before Qt starts, so
  they work without a display, and a `kxbps(1)` manual page.
- The update check's own xbps output, and the command it ran, in the Log
  drawer. A check is not a transaction, so its lines get a model of their own.
- Translations into German, Spanish, French, Italian, Dutch, Polish and
  Brazilian Portuguese, Kirigami theme colours throughout, and a window that
  fits the screen at any scale factor.

### Changed

- New icon set: a stack of packages for the application, and a monochrome tray
  icon that switches to the update variant while updates are waiting. The tray
  icon is a theme name, so Plasma recolours it for the panel.
- Settings has the start-time switch back: "Announce available updates on
  startup" keeps a start silent and lets a check run from the window report,
  while "Use desktop notifications" covers the popups as a whole.
- The log drawer is called `Log`. It shows what xbps printed, but the old name
  read as another program's log rather than a part of this window.
- The Log drawer's bar sits on the bottom edge: the log grows upward from the
  status strip, so the control that opened it belongs at that edge.

### Fixed

- The log drawer opens before the first transaction and says there is nothing
  to show yet. The button used to be disabled until a transaction had run,
  which read as a broken control.

### Notes

- The catalogs other than Polish are a first pass and have not been reviewed by
  native speakers; corrections belong in `po/<lang>.po`.
- The mirror and repository helpers run `xbps-install -S` after writing a file,
  because `xbps-query` skips a repository without an index without a word. A
  failed sync leaves the configuration in place and reports a warning.
