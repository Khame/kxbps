# Void Linux packaging: kxbps maintainer notes

This repository holds the application, the files the package installs, and the
Void template. The package directory in a void-packages tree is copied from
`packaging/void/`, and the template downloads the source tarball from a GitHub
release.

Users should read [README.md](README.md) and [INSTALL.md](INSTALL.md) instead.

## Where the files live

| Path | What it is |
|---|---|
| `packaging/void/template` | the `xbps-src` template: version, revision, dependencies, `distfiles`, `checksum`, `do_install` |
| `packaging/void/prepare-local.py` | creates the source tarball for a release and prints its SHA-256 |
| `packaging/void/verify-release.py` | checks the version and checksum across `Cargo.toml`, the man page, the metainfo, `CHANGELOG.md`, the template, the tarball and a void-packages checkout |
| `packaging/install.sh` | installs a checkout into `/usr`: binaries, icons, entries, policy, man page, catalogs |
| `packaging/io.github.khame.kxbps.desktop` | launcher entry, `Icon=kxbps` |
| `packaging/io.github.khame.kxbps.metainfo.xml` | AppStream data; carries one `<release>` per version |
| `packaging/io.github.khame.kxbps.policy` | the four polkit actions |
| `packaging/icons/app/` | `kxbps.svg`, `kxbps-updates.svg` |
| `packaging/icons/tray/symbolic/` | the four tray icons |
| `packaging/icons/actions/` | the twelve symbolic glyphs the window draws |
| `man/kxbps.1` | the manual page |
| `po/` | gettext catalogs: `kxbps.pot` and seven translations |
| `kxbps-core/`, `kxbps-qt/` | the crates; the QML is compiled into the binary |

`packaging/icons/actions/*.svg` are not installed: `kxbps-qt/resources.qrc`
embeds them and the window reads them from `qrc:`. Only the app and tray icons
are files on disk, because Plasma's icon theme looks them up by name.

`packaging/void/` and this file carry `export-ignore` in `.gitattributes`, so
neither reaches the source tarball.

## The source tarball

`prepare-local.py` writes the tarball the template downloads. It refuses a dirty
working tree, uses `git archive` so only tracked files are inside, and prints
the digest the template needs:

```sh
./packaging/void/prepare-local.py
```

The file is `kxbps-<version>.tar.gz`, with one top-level directory,
`kxbps-<version>`, holding 151 files: the two crates, `Cargo.toml`,
`Cargo.lock`, `.cargo/config.toml`, `.gitattributes`, `man/`, `po/`,
`packaging/` (without `void/`), `scripts/`, the docs and the license.

Nothing from the working tree can leak in, because it is not tracked:
`target/` and `screenshots/` are ignored. Check anyway before uploading:

```sh
tar tzf kxbps-<version>.tar.gz |
    grep -iE "screenshots|target/|VOID_PACKAGING"
```

The tarball is attached to the `v<version>` release in
[Khame/kxbps](https://github.com/Khame/kxbps). The template does not use
`FILESDIR`.

## About the template

`build_style=cargo`, with `do_install` instead of the default one: the default
runs `cargo install` for every binary in the workspace, and the two helpers have
to land in `/usr/libexec`, the path the polkit policy names. `vbin`,
`vinstall` and `vman` place the rest; `msgfmt` writes one `.mo` per catalog.

`hostmakedepends="gettext lld qt6-base-devel"`: `msgfmt` for the catalogs, `lld`
because GNU ld drops the initializers the generated C++ needs, and the Qt tools
`cxx-qt-build` runs. `QMAKE` is not exported in the template: the tree carries
`.cargo/config.toml` with `QMAKE=/usr/lib/qt6/bin/qmake6`, it travels inside the
tarball, and the build reads it from there.

`depends` carries what the automatic scan cannot see: `xbps` for the tools,
`polkit` for `pkexec`, `kf6-kirigami` and `kf6-kstatusnotifieritem` for the QML
modules and the tray, and `qt6-svg`. The last one is easy to miss: the icons are
SVG, the shlib scan sees no dependency, and `QT_DEBUG_PLUGINS=1` is what shows
the window loading `libqsvgicon.so`. Without the package every icon the window
draws comes out blank.

There is no `vlicense`: the Manual requires the license file in the package for
AGPL, MIT, BSD, ISC, X11 and custom licenses, and this one is
`GPL-3.0-or-later`.

No runit service: the window is a session program, not a daemon. The package
carries no patches.

## Building locally

Copy the template into a void-packages tree. For a release build it downloads
the tarball from the GitHub release; for a local build, generate the tarball
first and put it in the source cache:

```sh
cd ~/workspace/projects/software/khaxbps
./packaging/void/prepare-local.py
mkdir -p ~/workspace/projects/software/void/void-packages/hostdir/sources/kxbps-0.11.0
cp kxbps-0.11.0.tar.gz \
    ~/workspace/projects/software/void/void-packages/hostdir/sources/kxbps-0.11.0/

cd ~/workspace/projects/software/void/void-packages
cp ~/workspace/projects/software/khaxbps/packaging/void/template srcpkgs/kxbps/

./xbps-src pkg kxbps
xlint srcpkgs/kxbps/template
```

Run `xbps-src` as a regular user. The package ends up in `hostdir/binpkgs/`.

Sign it, and remove the old signature first:

```sh
pkg=$(ls -t hostdir/binpkgs/kxbps-*.x86_64.xbps | head -1)
rm -f "$pkg.sig2"
xbps-rindex -S --privkey="$HOME/.config/Khame-void-repo-private.pem" \
    --signedby="Khame <khame.dev@protonmail.com>" "$pkg"
```

Install it from a local repository, with the default repository alongside it so
the dependencies resolve:

```sh
mkdir -p /tmp/kxbps-local-repo
cp hostdir/binpkgs/kxbps-*.x86_64.xbps* /tmp/kxbps-local-repo/
cd /tmp/kxbps-local-repo
xbps-rindex -c . && xbps-rindex -a kxbps-*.x86_64.xbps
xbps-rindex -s --privkey="$HOME/.config/Khame-void-repo-private.pem" \
    --signedby="Khame <khame.dev@protonmail.com>" .

sudo xbps-install -R /tmp/kxbps-local-repo \
    -R https://repo-default.voidlinux.org/current kxbps
```

`xbps-install` refuses a package that is already installed at the same version,
so a rebuild needs `revision` bumped. `-f` forces it and leaves the installed
metadata stale in every place you would check later.

## Things that bit me

- `xbps-rindex -S` does not replace an existing `.sig2`. It exits 0 without a
  word and leaves the old signature, which then fails to verify. Remove the file
  first, then check the result against the public keys in `/var/db/xbps/keys/`.
- A tarball in `hostdir/sources` that does not match the release asset makes a
  clean build fail somewhere else. The checksum in the template is the only
  thing tying the two together, so regenerate both in the same step.
- The window renders SVG through Qt's icon-engine plugin. The shlib scan cannot
  see it; `qt6-svg` has to be in `depends` by hand.
- `xbps-query --repository=… -p pkgver` answers from a cached index unless `-M`
  is passed, and a stale answer looks like a failed publication.
- The version is read from the workspace manifest, so a crate that sets its own
  `version` would drift silently. `version_guard` in `kxbps-qt/src/lib.rs` and
  `cli::tests::the_version_line_carries_the_package_version` exist for that.
- Bumping `revision` is not a version bump. 0.11.0 was first built as a
  revision of 0.10.1; the version has to move when the code does.

## Release invariants

The version has to agree in five places at the same time:

| Place | Kept in step by |
|---|---|
| `Cargo.toml` (`workspace.package.version`) | the source of truth |
| `man/kxbps.1` (`.TH` line) | `version_guard::the_manual_page_header_carries_the_version` |
| `packaging/io.github.khame.kxbps.metainfo.xml` | `version_guard::the_metainfo_lists_the_version` |
| `CHANGELOG.md` | by hand |
| `packaging/void/template` (`version`, `checksum`) | by hand, against the tarball |

`python3 packaging/void/verify-release.py --tarball kxbps-0.11.0.tar.gz
--void-packages <path>` checks all of them and exits nonzero on the first
mismatch. Run it before tagging and again after the checksum update.

## Publishing

Packages are published for x86_64 in `Khame/void-packages` under `docs/`, served
at `https://khame.github.io/void-packages/`. Install documentation and release
notes live in this repository.

To publish a new revision:

1. For a new version, set it in `Cargo.toml`, `man/kxbps.1`, the metainfo and
   `CHANGELOG.md`, and set `revision=1` in the template. Run
   `cargo test --workspace`: the guard tests fail on a mismatch. For a rebuild,
   bump `revision` only.
2. Create the source tarball with `./packaging/void/prepare-local.py` and note
   its SHA-256.
3. Create or update the `v<version>` release in `Khame/kxbps` and upload the
   tarball as an asset.
4. Put the checksum in `packaging/void/template` and run
   `verify-release.py --tarball … --void-packages <path>`.
5. Copy the template into `srcpkgs/kxbps/template` in `Khame/void-packages`.
6. Build and sign as above.
7. Remove the previous `kxbps` package and signature from `docs/`, copy the new
   `.xbps` and `.sig2` in, then rebuild and sign the index:
   `xbps-rindex -c docs`, `xbps-rindex -a docs/*.xbps`,
   `xbps-rindex -s --privkey=… --signedby=… docs`.
8. Commit and push `Khame/void-packages`, and attach the same `.xbps`, `.sig2`
   and the public key to the release in `Khame/kxbps`. The key is the `.plist`
   file from `/var/db/xbps/keys/`, not `Khame-void-repo.pub`, which is the PEM
   copy served by the site. GitHub replaces the colons in the asset name with
   dots, so the `install` step in `INSTALL.md` renames the file back to the
   fingerprint.
9. Verify the published result:
   `xbps-query -M --repository=https://khame.github.io/void-packages/ -p pkgver kxbps`
   has to answer the new version. Without `-M` the query can answer from a
   cached index and look stale. The Pages build for the pushed commit has to
   report `built` in `gh api repos/Khame/void-packages/pages/builds/latest`.

Only x86_64 is published. Never push to `void-linux/void-packages` or to an
upstream project.

## Author and license

Khame ([@Khame](https://github.com/Khame)),
[khame.dev@protonmail.com](mailto:khame.dev@protonmail.com)

GPL-3.0-or-later, the same as the program.
