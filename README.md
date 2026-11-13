# KXbps

A native KDE Plasma front end for xbps, the Void Linux package manager.

KXbps runs the real tools (`xbps-query`, `xbps-install`, `xbps-remove`) and
shows what they report. There is no bundled catalog and no fake state: a fresh
install starts with an empty store, and every list is read from the system on
demand. Privileged work goes through `pkexec` and the polkit actions in
`packaging/io.github.khame.kxbps.policy`.

## Build

Requirements: Rust 1.99 or newer, Qt 6 base and declarative, KF6 Kirigami and
KI18n. On Void:

```sh
sudo xbps-install rust cargo lld qt6-base-devel qt6-declarative-devel \
    kf6-kirigami-devel kf6-ki18n-devel
cargo build
./target/debug/kxbps
```

`lld` is required: cxx-qt asks the linker to use it, and GNU ld drops the
initializers the generated C++ needs, so the link fails with undefined
references to `cxx_qt_init_crate_*`.

`cargo test --workspace` runs the suite; the Qt side is tested through the
CXX-Qt bridge without a display, and the UI smoke test is

```sh
QT_QPA_PLATFORM=offscreen timeout 5 ./target/debug/kxbps
```

State lives in `~/.local/share/kxbps/` (`queue.json`, `prefs.json`,
`mirror.json`), with mode 0700.

## Install

From a checkout, into `/usr`:

```sh
cargo build --release
sudo DESTDIR= PREFIX=/usr sh packaging/install.sh
```

That installs the window, the two helpers (`kxbps-mirror-apply` and
`kxbps-repo-apply`), the desktop entry, the icon, the metainfo, the polkit
policy and the translations. The QML is compiled into the binary, so there is
no module directory to install. `PREFIX` and `DESTDIR` can point
elsewhere, but the helpers always land in `/usr/libexec`: the polkit actions
name that path.

Adding or removing a repository, applying a mirror and every transaction ask
for the password through polkit; nothing runs as root otherwise.

## License

Copyright © 2026 Łukasz "Khame" Linhard.

KXbps is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. See [LICENSE](LICENSE) for the full text.

The program links Qt 6 and KF6 (LGPL) and uses Rust crates under MIT and
Apache-2.0 licenses; their notices travel with the binary.
