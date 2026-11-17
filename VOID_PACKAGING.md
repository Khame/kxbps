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
| `packaging/io.github.khame.kxbps.policy` | the five polkit actions |
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

The template downloads the tag archive from the repository:

```
distfiles="${homepage}/archive/refs/tags/v${version}.tar.gz"
```

GitHub builds that file from the tagged commit, so it holds exactly the tracked
files of the release and nothing from a working tree. The digest comes from the
URL, never from a local copy:

```sh
curl -fsSL https://github.com/Khame/kxbps/archive/refs/tags/v0.12.2.tar.gz |
    sha256sum
```

Two consequences. The checksum cannot be written before the tag exists on
GitHub, so it is the one line that changes in the commit after the push. And
GitHub's own tar metadata differs from a locally built archive, so a digest from
`prepare-local.py` never matches: that script is for local builds and for
checking what the archive contains.

`prepare-local.py` writes `kxbps-<version>.tar.gz` from the current commit, and
refuses a dirty working tree. The file has one top-level directory,
`kxbps-<version>`, holding the two crates, `Cargo.toml`, `Cargo.lock`,
`.cargo/config.toml`, `.gitattributes`, `man/`, `po/`, `packaging/` (without
`void/`), `scripts/`, the docs and the license. It is what a local build seeds
into `hostdir/sources/kxbps-<version>/` when the tag is not there yet:

```sh
./packaging/void/prepare-local.py
```

Nothing from the working tree can leak in, because the archive holds the
tracked files of a commit and nothing else. Check it anyway before a release:
every entry has to be a file `git ls-files` names.

```sh
tar tzf kxbps-<version>.tar.gz | grep -v '/$' | sed 's|^kxbps-<version>/||' |
    sort | comm -23 - <(git ls-files | sort)
```

An empty answer is what a clean release looks like.

The template does not use `FILESDIR`.

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
modules and the tray, `kf6-qqc2-desktop-style` for the style Kirigami asks for,
`kf6-sonnet` for the spell-check module the text fields load, and `qt6-svg`. The
last one is easy to miss: the icons are SVG, the shlib scan sees no dependency,
and `QT_DEBUG_PLUGINS=1` is what shows the window loading `libqsvgicon.so`.
Without the package every icon the window draws comes out blank. The two KF6
QML modules fail the same quiet way: the window starts and a binding logs a
missing import instead of the package being reported.

There is no `vlicense`: the Manual requires the license file in the package for
AGPL, MIT, BSD, ISC, X11 and custom licenses, and this one is
`GPL-3.0-or-later`.

No runit service: the window is a session program, not a daemon. The package
carries no patches.

## Building locally

`xbps-src` lives in a void-packages checkout, not in a package. One checkout
serves every Khame package. `binary-bootstrap` fills its `masterdir/` with the
minimal root the build runs in, and is needed once per checkout:

```sh
git clone --depth=1 https://github.com/void-linux/void-packages.git
cd void-packages
./xbps-src binary-bootstrap
```

`xlint` is not in the tree either: `sudo xbps-install xtools` provides it.

The package directory is a copy, not a link:

```sh
mkdir -p srcpkgs/kxbps
cp ~/workspace/projects/software/khaxbps/packaging/void/template srcpkgs/kxbps/
```

The build downloads the tag archive, so a pushed tag is all it needs. When the
tag is not on GitHub yet, seed the source cache with a local tarball. `xbps-src`
names a cached distfile after the basename of its URL, so the file has to be
`v<version>.tar.gz`; a copy called `kxbps-<version>.tar.gz` sits in the cache
unused and the build goes to the network instead:

```sh
cd ~/workspace/projects/software/khaxbps
./packaging/void/prepare-local.py
mkdir -p ~/workspace/projects/software/void/void-packages/hostdir/sources/kxbps-<version>
cp kxbps-<version>.tar.gz \
    ~/workspace/projects/software/void/void-packages/hostdir/sources/kxbps-<version>/v<version>.tar.gz

cd ~/workspace/projects/software/void/void-packages
./xbps-src pkg kxbps
xlint srcpkgs/kxbps/template
```

Run `xbps-src` as a regular user; it needs no root, and it creates `masterdir/`
and `hostdir/` inside the checkout. The package ends up in `hostdir/binpkgs/`.

The musl variant comes from the same template, with its own masterdir:

```sh
./xbps-src -A x86_64-musl binary-bootstrap
./xbps-src -A x86_64-musl pkg kxbps
./xbps-src -Q -A x86_64-musl pkg kxbps    # test suite inside the chroot
```

It writes `hostdir/binpkgs/kxbps-<version>_<revision>.x86_64-musl.xbps`. Plain
`./xbps-src pkg kxbps` is the glibc build; `-Q` runs the tests the same way for
either architecture.

When the tarball changes but the version does not, run `./xbps-src clean kxbps`
first: `masterdir/builddir/kxbps-<version>` is reused when it exists, and with an
unchanged version number it is indistinguishable from a fresh one.

The shared `masterdir/` is not proof that the declared dependencies are enough:
it keeps packages from earlier builds, and a missing `makedepends` still
resolves there. For that check, use a throwaway masterdir and hostdir:

```sh
export XBPS_MASTERDIR=/var/tmp/kxbps-clean/masterdir
export XBPS_HOSTDIR=/var/tmp/kxbps-clean/hostdir
mkdir -p "$XBPS_HOSTDIR/sources/kxbps-<version>"
cp kxbps-<version>.tar.gz "$XBPS_HOSTDIR/sources/kxbps-<version>/v<version>.tar.gz"
./xbps-src binary-bootstrap
./xbps-src pkg kxbps
```

Seed the cache only while the tag is unpublished; once it is, the build fetches
the archive by itself. A clean build with an empty masterdir installs every
dependency from the repository; it should produce the same package as the shared
one, apart from the ELF build-id and the table order.

`ERROR: Package 'qt6-base-devel-…' already installed` in the log does not mean
the masterdir is dirty: `qt6-base-devel` sits in both `hostmakedepends` and
`makedepends`, so the second install attempt reports it and the build goes on.

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
- A tarball in `hostdir/sources` that does not match the tag archive makes a
  clean build fail somewhere else. The checksum in the template is the only
  thing tying the two together, so regenerate both in the same step.
- `xbps-rindex -a` refuses a musl package on a glibc host with `index: ignoring
  …, unmatched arch`. It picks the architecture from `uname`; export
  `XBPS_ARCH=x86_64-musl` for the musl directory, for `-a` and for `-s` alike.
- The key-import prompt needs a real stdin. Piping the `sudo` password leaves
  the prompt at EOF, xbps abandons the import without failing the sync, and the
  next install dies with `the RSA signature is not valid`. Answer the prompt.
- `git archive` is not reproducible: the commit id travels in a pax global
  header and the commit time sits in every member header. Dropping
  `pax_global_header` by name is not enough — `tarfile` reads the global records
  and hands them to each member, and its writer puts a shared record back into a
  global header. The record has to be removed from the members.
- The window renders SVG through Qt's icon-engine plugin. The shlib scan cannot
  see it; `qt6-svg` has to be in `depends` by hand.
- `xbps-install -n` reads only the indexes already cached in
  `<rootdir>/var/db/xbps/`. In a fresh rootdir that directory does not exist and
  every package answers `not found in repository pool`, ours included. The manual
  says it in the `-n` entry: use `-M` to sync the indexes into memory for the
  operation. `xbps-query -M` works without a cache for the same reason.
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
| `packaging/void/template` (`version`, `checksum`) | by hand, against the tag archive |

`python3 packaging/void/verify-release.py --tarball v<version>.tar.gz
--void-packages <path>` checks all of them and exits nonzero on the first
mismatch; the tarball is the fetched tag archive, not the local build. Run it
before tagging and again after the checksum update.

## Publishing

Two repositories live in `Khame/void-packages`, both under `docs/` and both
served from `https://khame.github.io/void-packages/`:

| Path | Architecture | Repository line |
|---|---|---|
| `docs/` | `x86_64` (glibc) | `https://khame.github.io/void-packages/` |
| `docs/musl/` | `x86_64-musl` | `https://khame.github.io/void-packages/musl` |

xbps reads `<repository>/<arch>-repodata`, so the two indexes never mix and a
musl host cannot be offered a glibc package. Each directory carries its own
packages, its own `.sig2` files and its own index.

To publish a new revision:

1. For a new version, set it in `Cargo.toml`, `man/kxbps.1`, the metainfo and
   `CHANGELOG.md`, and set `revision=1` in the template. Run
   `cargo test --workspace`: the guard tests fail on a mismatch. For a rebuild,
   bump `revision` only.
2. Commit, tag `v<version>`, and push the tag. The template downloads the tag
   archive, so a tree that is only local cannot be built by anyone else.
3. Take the checksum from that archive, put it in `packaging/void/template`, run
   `verify-release.py --void-packages <path>` and commit the result:

   ```sh
   curl -fsSL https://github.com/Khame/kxbps/archive/refs/tags/v<version>.tar.gz |
       sha256sum
   ```

4. Copy the template into `srcpkgs/kxbps/template` in `Khame/void-packages`.
5. Build and test both architectures. Each has its own masterdir, so the musl
   build cannot reuse a glibc object:

   ```sh
   ./xbps-src -A x86_64-musl binary-bootstrap     # once per checkout
   ./xbps-src pkg kxbps
   ./xbps-src -Q pkg kxbps
   ./xbps-src -A x86_64-musl pkg kxbps
   ./xbps-src -Q -A x86_64-musl pkg kxbps
   ```

6. Sign both packages. `xbps-rindex -S` leaves an existing `.sig2` alone and
   exits 0, so remove it first:

   ```sh
   for pkg in hostdir/binpkgs/kxbps-<version>_<revision>.x86_64*.xbps; do
       rm -f "$pkg.sig2"
       xbps-rindex -S --privkey="$HOME/.config/Khame-void-repo-private.pem" \
           --signedby="Khame <khame.dev@protonmail.com>" "$pkg"
   done
   ```

7. Replace the previous package in each directory and rebuild both indexes.
   `xbps-rindex -a` adds to an index, it does not remove what left the tree, so
   the old `.xbps` and `.sig2` have to go first:

   ```sh
   cd <Khame/void-packages>
   rm -f docs/kxbps-*.x86_64.xbps* docs/musl/kxbps-*.x86_64-musl.xbps*
   cp <void-packages>/hostdir/binpkgs/kxbps-<version>_<revision>.x86_64.xbps* docs/
   cp <void-packages>/hostdir/binpkgs/kxbps-<version>_<revision>.x86_64-musl.xbps* docs/musl/
   for dir in docs docs/musl; do
       xbps-rindex -c "$dir"
       xbps-rindex -a "$dir"/*.xbps
       xbps-rindex -s --privkey="$HOME/.config/Khame-void-repo-private.pem" \
           --signedby="Khame <khame.dev@protonmail.com>" "$dir"
   done
   ```

8. Commit and push `Khame/void-packages`. Attach both `.xbps` files with their
   `.sig2` and the public key to the release in `Khame/kxbps`; the file names
   carry the architecture, and the repository stays the install path.
   The key is the `.plist` file from `/var/db/xbps/keys/`, not
   `Khame-void-repo.pub`, which is the PEM copy served by the site. GitHub
   replaces the colons in the asset name with dots, so the `install` step in
   `INSTALL.md` renames the file back to the fingerprint.
9. Verify both indexes, and remember `-M`: without it a query answers from a
   cached index and looks stale.

   ```sh
   xbps-query -M --repository=https://khame.github.io/void-packages/ -p pkgver kxbps
   XBPS_ARCH=x86_64-musl \
       xbps-query -M --repository=https://khame.github.io/void-packages/musl -p pkgver kxbps
   ```

   The second line needs `XBPS_ARCH` on a glibc host: xbps asks for
   `<arch>-repodata` of the machine it runs on, and without the override it
   looks for a file the musl tree does not have.

   The Pages build for the pushed commit has to report `built` in
   `gh api repos/Khame/void-packages/pages/builds/latest`. On a musl host,
   `xbps-install -Su` has to offer the new version.

Both architectures or no release: a version that exists for one of them leaves
the other one stale while `xbps-install -Su` reports nothing to do. Never push
to `void-linux/void-packages` or to an upstream project.

## Author and license

Khame ([@Khame](https://github.com/Khame)),
[khame.dev@protonmail.com](mailto:khame.dev@protonmail.com)

GPL-3.0-or-later, the same as the program.
