# Installing KXbps

Two routes, a package and a checkout. Both put the same files in the same
places.

## From a package

KXbps is published for x86_64 in Khame's signed repository. Point xbps at it
once, then install:

```sh
sudo sh -c 'echo "repository=https://khame.github.io/void-packages/" > /etc/xbps.d/10-khame.conf'
sudo xbps-install -S      # answer Y to import the repository key
sudo xbps-install kxbps
```

On the first sync xbps asks whether to import the signing key. Compare the
fingerprint before answering:

```
35:2b:06:7c:52:04:76:b1:4a:de:a0:8b:49:9c:e2:66
```

The published key gives the same fingerprint:

```sh
curl -fsSO https://khame.github.io/void-packages/Khame-void-repo.pub
ssh-keygen -E md5 -lf <(ssh-keygen -i -m PKCS8 -f Khame-void-repo.pub)
```

`sudo xbps-remove kxbps` takes it out again. The state in
`~/.local/share/kxbps/` stays, because it belongs to the user, not to the
package.

## From a checkout

Install the build dependencies first:

```sh
sudo xbps-install rust cargo lld qt6-base-devel qt6-declarative-devel \
    kf6-kirigami-devel kf6-ki18n-devel
```

Build once as your user, then install the result:

```sh
cargo build --release
sudo DESTDIR= PREFIX=/usr sh packaging/install.sh
```

The script builds the release binaries itself. `CARGO` points it at another
cargo, and `CARGO=true` skips the build when `target/release` is already
current.

It installs these files, plus one catalog per language under
`/usr/share/locale`:

| Path | What |
| --- | --- |
| `/usr/bin/kxbps` | the window |
| `/usr/libexec/kxbps-mirror-apply` | the mirror helper polkit runs |
| `/usr/libexec/kxbps-repo-apply` | the repository helper polkit runs |
| `/usr/share/applications/io.github.khame.kxbps.desktop` | launcher entry |
| `/usr/share/icons/hicolor/scalable/apps/kxbps.svg` | the window's icon |
| `/usr/share/icons/hicolor/scalable/apps/kxbps-updates.svg` | the icon with the update arrow |
| `/usr/share/icons/hicolor/scalable/status/kxbps-tray*.svg` | the tray icons: `kxbps-tray`, `kxbps-tray-updates` and their `-symbolic` forms |
| `/usr/share/metainfo/io.github.khame.kxbps.metainfo.xml` | AppStream data |
| `/usr/share/polkit-1/actions/io.github.khame.kxbps.policy` | the five actions |
| `/usr/share/man/man1/kxbps.1` | the manual page |
| `/usr/share/locale/<lang>/LC_MESSAGES/kxbps.mo` | translations: `de`, `es`, `fr`, `it`, `nl`, `pl`, `pt_BR` |

`PREFIX` and `DESTDIR` move `bin/` and `share/`. The two helpers always land in
`/usr/libexec`, because the policy names that path and polkit resolves it as
root. The QML is compiled into the binary, so there is no module directory to
install.

### Runtime dependencies

`qt6-base`, `qt6-declarative`, `kf6-kirigami`, `kf6-ki18n`, `polkit` (for
`pkexec`), `xbps` (for the tools the window runs). Build dependencies: the
`xbps-install` line above.

## Checking the install

```sh
xbps-query -p pkgver kxbps                  # only when installed as a package
test -x /usr/libexec/kxbps-mirror-apply && echo mirror helper ok
test -x /usr/libexec/kxbps-repo-apply && echo repository helper ok
grep -c 'action id=' /usr/share/polkit-1/actions/io.github.khame.kxbps.policy
QT_QPA_PLATFORM=offscreen timeout 5 kxbps
```

The `grep` prints `5`. The last command should end on exit status 124 with
nothing on stderr; the timeout is what stops it.

## Removing a checkout install

```sh
sudo rm -f /usr/bin/kxbps \
    /usr/libexec/kxbps-mirror-apply \
    /usr/libexec/kxbps-repo-apply \
    /usr/share/applications/io.github.khame.kxbps.desktop \
    /usr/share/icons/hicolor/scalable/apps/kxbps.svg \
    /usr/share/icons/hicolor/scalable/apps/kxbps-updates.svg \
    /usr/share/icons/hicolor/scalable/status/kxbps-tray.svg \
    /usr/share/icons/hicolor/scalable/status/kxbps-tray-updates.svg \
    /usr/share/icons/hicolor/scalable/status/kxbps-tray-symbolic.svg \
    /usr/share/icons/hicolor/scalable/status/kxbps-tray-updates-symbolic.svg \
    /usr/share/metainfo/io.github.khame.kxbps.metainfo.xml \
    /usr/share/polkit-1/actions/io.github.khame.kxbps.policy \
    /usr/share/man/man1/kxbps.1 \
    /usr/share/locale/pl/LC_MESSAGES/kxbps.mo
```

## Troubleshooting

The package lists come from `xbps-query`, which skips a repository that has no
local index and says nothing about it. After a mirror switch the indexes are
fetched by the helper itself. If a list still looks short, compare `xbps-query
-L` with `xbps-query -Rs '' | wc -l`: a `-1` in the first column means that
repository has no index, and `sudo xbps-install -S` fetches it.

The Repositories page disables its buttons and prints a sentence when it cannot
find a helper or the policy file. Both are installed by the package; a missing
one means the installation did not finish.

Polkit asks for the password of an administrative user. If `pkexec` is not
installed, no privileged action can start, and the job reports that instead of
running anything.
