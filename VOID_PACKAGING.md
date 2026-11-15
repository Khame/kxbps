# Void Linux packaging: kxbps maintainer notes

This repository holds the application and the files the package installs. The
`xbps-src` template itself lives in `Khame/void-packages`, not here: it points at
the source tarball of a GitHub release, and the release is cut from this tree.

Users should read [README.md](README.md) and [INSTALL.md](INSTALL.md) instead.

## Where the files live

| Path | What it is |
|---|---|
| `packaging/install.sh` | installs a checkout into `/usr`: binaries, icons, entries, policy, man page, catalogs |
| `packaging/io.github.khame.kxbps.desktop` | launcher entry, `Icon=kxbps` |
| `packaging/io.github.khame.kxbps.metainfo.xml` | AppStream data; carries one `<release>` per version |
| `packaging/io.github.khame.kxbps.policy` | the four polkit actions |
| `packaging/icons/app/` | `kxbps.svg`, `kxbps-updates.svg` |
| `packaging/icons/tray/symbolic/` | the four tray icons |
| `packaging/icons/actions/` | the twelve symbolic glyphs the window draws |
| `man/kxbps.1` | the manual page |
| `po/` | gettext catalogs; `kxbps.pot` plus seven translations |
| `kxbps-core/`, `kxbps-qt/` | the crates; the QML is compiled into the binary |

`packaging/icons/actions/*.svg` are not installed: `kxbps-qt/resources.qrc`
embeds them, and the window reads them from `qrc:`. Only the app and tray icons
are files on disk, because Plasma's icon theme looks them up by name.

## The template

`srcpkgs/kxbps/template` in `Khame/void-packages`, and the same file in a local
void-packages checkout while building:

```
build_style=cargo
hostmakedepends="gettext lld qt6-base-devel"
makedepends="qt6-base-devel qt6-declarative-devel kf6-kirigami-devel
 kf6-ki18n-devel kf6-kstatusnotifieritem-devel"
depends="kf6-kirigami kf6-kstatusnotifieritem polkit qt6-svg xbps"
distfiles="${homepage}/releases/download/v${version}/kxbps-${version}.tar.gz"
```

`QMAKE` is not exported in the template: the tree carries
`.cargo/config.toml` with `QMAKE=/usr/lib/qt6/bin/qmake6`, it travels inside the
tarball, and `cxx-qt-build` reads it from there. A clean chroot build proves the
declared set is enough.

`do_install` is defined because the default one runs `cargo install` for every
binary in the workspace, and the two helpers must land in `/usr/libexec`, the
path the polkit policy names. `vbin`, `vinstall` and `vman` place the rest;
`msgfmt` writes one `.mo` per catalog.

`depends` carries what the automatic scan cannot see: `xbps` for the tools,
`polkit` for `pkexec`, `kf6-kirigami` and `kf6-kstatusnotifieritem` for the QML
modules and the tray, and `qt6-svg`. The last one is easy to miss: the icons are
SVG, the shlib scan sees no dependency, and `QT_DEBUG_PLUGINS=1` is what shows
the app loading `libqsvgicon.so`. Without the package every icon the window draws
comes out blank.

There is no `vlicense`: the Manual requires the license file in the package for
AGPL, MIT, BSD, ISC, X11 and custom licenses, and this one is
`GPL-3.0-or-later`.

No runit service: the app is a session program. No patches.

## Version invariants

The version has to agree in five places at the same time:

| Place | Kept in step by |
|---|---|
| `Cargo.toml` (`workspace.package.version`) | the source of truth |
| `man/kxbps.1` (`.TH` line) | `version_guard::the_manual_page_header_carries_the_version` |
| `packaging/io.github.khame.kxbps.metainfo.xml` | `version_guard::the_metainfo_lists_the_version` |
| `CHANGELOG.md` | by hand |
| `srcpkgs/kxbps/template` (`version`, `checksum`) | by hand, against the tarball |

`kxbps --version` reads `CARGO_PKG_VERSION`, so it cannot drift.

## The source tarball

`git archive` writes exactly the tracked files, so nothing from the working tree
leaks in (`target/`, `screenshots/` are untracked and ignored):

```sh
git archive --format=tar --prefix=kxbps-${version}/ HEAD | gzip -9n \
    > kxbps-${version}.tar.gz
sha256sum kxbps-${version}.tar.gz
```

One top-level directory, `kxbps-<version>`, holding 176 files: the two crates,
`Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`, `man/`, `po/`, `packaging/`,
`scripts/`, the docs and the license. Check it before uploading:

```sh
tar tzf kxbps-${version}.tar.gz |
    grep -iE "screenshots|target/" \
    && echo "leftovers in the tarball"
```

The tarball is attached to the `v<version>` release in
[Khame/kxbps](https://github.com/Khame/kxbps).

## Building locally

For a local build, put the tarball in the source cache so `xbps-src` never
touches the network for it:

```sh
cd ~/workspace/projects/software/khaxbps
git archive --format=tar --prefix=kxbps-0.11.0/ HEAD | gzip -9n \
    > /tmp/kxbps-0.11.0.tar.gz

cd ~/workspace/projects/software/void/void-packages
mkdir -p hostdir/sources/kxbps-0.11.0
cp /tmp/kxbps-0.11.0.tar.gz hostdir/sources/kxbps-0.11.0/

# version, revision and checksum in srcpkgs/kxbps/template must match
sha256sum hostdir/sources/kxbps-0.11.0/kxbps-0.11.0.tar.gz

./xbps-src pkg kxbps
xlint srcpkgs/kxbps/template
```

Sign it, and remove the old signature first:

```sh
rm -f hostdir/binpkgs/kxbps-<version>_<revision>.x86_64.xbps.sig2
xbps-rindex -S --privkey="$HOME/.config/Khame-void-repo-private.pem" \
    --signedby="Khame <khame.dev@protonmail.com>" \
    hostdir/binpkgs/kxbps-<version>_<revision>.x86_64.xbps
```

Install it from a local repository, with the default repository alongside it so
the dependencies resolve:

```sh
mkdir -p /tmp/kxbps-local-repo
cp hostdir/binpkgs/kxbps-<version>_<revision>.x86_64.xbps* /tmp/kxbps-local-repo/
cd /tmp/kxbps-local-repo
xbps-rindex -c . && xbps-rindex -a kxbps-*.x86_64.xbps
xbps-rindex -s --privkey="$HOME/.config/Khame-void-repo-private.pem" \
    --signedby="Khame <khame.dev@protonmail.com>" .

sudo xbps-install -R /tmp/kxbps-local-repo \
    -R https://repo-default.voidlinux.org/current kxbps
```

`xbps-install` refuses a package that is already installed at the same version,
so a rebuild of the same version needs `revision` bumped; `-f` forces it and
leaves the metadata stale in every place you would check later.

## Things that bit me

- `xbps-rindex -S` does not replace an existing `.sig2`. It exits 0 without a
  word and leaves the old signature, which then fails to verify. Remove the file
  first, then check the result against the public keys in `/var/db/xbps/keys/`.
- A local tarball that does not match the release asset makes a clean build fail
  somewhere else. The checksum in the template is the only thing that ties the
  two together, so regenerate the tarball and the checksum in the same step.
- The app renders SVG through Qt's icon-engine plugin. The shlib scan cannot see
  it; `qt6-svg` has to be in `depends` by hand.
- `xbps-query --repository=… -p pkgver` answers from a cached index unless `-M`
  is passed, and a stale answer looks like a failed publication.
- The version is read from the workspace manifest, so a crate that sets its own
  `version` would drift silently. `version_guard` and the CLI test exist for
  that.

## Publishing

1. Set `version` in `Cargo.toml`, `man/kxbps.1`, the metainfo and `CHANGELOG.md`;
   set `revision=1` in the template. Run `cargo test --workspace` — the guard
   tests fail on a mismatch.
2. Create the source tarball and note its SHA-256.
3. Create the `v<version>` tag and release in `Khame/kxbps`, upload the tarball
   as an asset.
4. Update `distfiles` (it follows `${version}`) and `checksum` in the template,
   then build and sign as above.
5. Copy the template into `srcpkgs/kxbps/template` of `Khame/void-packages`.
6. Remove the previous `kxbps` package and signature from `docs/`, copy the new
   `.xbps` and `.sig2` in, then rebuild and sign the index:
   `xbps-rindex -c docs`, `xbps-rindex -a docs/*.xbps`,
   `xbps-rindex -s --privkey=… --signedby=… docs`.
7. Commit and push `Khame/void-packages`, and attach the same `.xbps`, `.sig2`
   and the public key to the release in `Khame/kxbps`.
8. Verify the published result:
   `xbps-query -M --repository=https://khame.github.io/void-packages/ -p pkgver kxbps`
   has to answer the new version.

Only x86_64 is published. Never push to `void-linux/void-packages` or to an
upstream project.

## Author and license

Khame ([@Khame](https://github.com/Khame)),
[khame.dev@protonmail.com](mailto:khame.dev@protonmail.com)

GPL-3.0-or-later, the same as the program.
