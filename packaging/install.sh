#!/bin/sh
# Install KXbps from a source checkout.
#
# Usage:
#   ./packaging/install.sh                      # into /usr
#   PREFIX=/usr/local ./packaging/install.sh    # into /usr/local
#   DESTDIR=/tmp/stage ./packaging/install.sh   # staged package tree
#
# This script does not use sudo; run it as the user who may write to PREFIX,
# or point DESTDIR at a staging directory.
#
# Environment overrides:
#   PREFIX   installation prefix for bin/, share/ and the QML module (default /usr)
#   DESTDIR  staging root prepended to every destination (default empty)
#   QMLDIR   directory that holds `org/kde/kxbps`; otherwise asked from qmake6
#   PO_DIR   directory holding the gettext catalogs (default <repo>/po)
#   CARGO    cargo executable (default `cargo` from PATH)

set -eu

die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

require_file() {
    [ -f "$1" ] || die "missing file: $1"
}

require_dir() {
    [ -d "$1" ] || die "missing directory: $1"
}

# Install one file, creating parent directories, and record it for the summary.
install_file() {
    src=$1
    mode=$2
    dst=$3
    require_file "$src"
    install -Dm"$mode" "$src" "$dst"
    installed="${installed}
$((installed_count + 1)). $dst"
    installed_count=$((installed_count + 1))
}

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/.." && pwd)

PREFIX=${PREFIX:-/usr}
DESTDIR=${DESTDIR:-}
PO_DIR=${PO_DIR:-$repo/po}
CARGO=${CARGO:-cargo}

installed=
installed_count=0

# The helper and the polkit actions name absolute paths, so the helper always
# goes to /usr/libexec regardless of PREFIX; only the policy's exec.path has to
# match it, which it does.
LIBEXECDIR=/usr/libexec

# --- tools ----------------------------------------------------------------

command -v "$CARGO" >/dev/null 2>&1 || die "cargo not found (set CARGO=/path/to/cargo)"
command -v msgfmt >/dev/null 2>&1 ||
    die "msgfmt not found: install the gettext package, otherwise the translations would be skipped"

# Qt's own QML import path, which is where the module has to land for
# `import org.kde.kxbps` to resolve. Void's qmake6 is not on PATH, so prefer
# the value .cargo/config.toml uses.
qml_dir=${QMLDIR:-}
if [ -z "$qml_dir" ]; then
    qmake=${QMAKE:-}
    if [ -z "$qmake" ]; then
        for candidate in /usr/lib/qt6/bin/qmake6 /usr/lib64/qt6/bin/qmake6 qmake6; do
            if command -v "$candidate" >/dev/null 2>&1; then
                qmake=$candidate
                break
            fi
        done
    fi
    if [ -n "$qmake" ]; then
        qml_dir=$("$qmake" -query QT_INSTALL_QML 2>/dev/null) || qml_dir=
    fi
fi
[ -n "${qml_dir:-}" ] || qml_dir=$PREFIX/lib/qt6/qml

# --- build ----------------------------------------------------------------

printf 'Building release binaries in %s\n' "$repo"
(cd "$repo" && "$CARGO" build --release --workspace)

app_bin=$repo/target/release/kxbps
helper_bin=$repo/target/release/kxbps-mirror-apply
repo_helper_bin=$repo/target/release/kxbps-repo-apply
qml_src=$repo/target/cxxqt/qml_modules/org/kde/kxbps
icon=$here/io.github.khame.kxbps.svg
desktop=$here/io.github.khame.kxbps.desktop
metainfo=$here/io.github.khame.kxbps.metainfo.xml
policy=$here/io.github.khame.kxbps.policy
notifyrc=$here/io.github.khame.kxbps.notifyrc

require_file "$app_bin"
require_file "$helper_bin"
require_file "$repo_helper_bin"
require_dir "$qml_src"
require_file "$icon"
require_file "$desktop"
require_file "$metainfo"
require_file "$policy"
require_file "$notifyrc"

# --- install --------------------------------------------------------------

install_file "$app_bin" 755 "$DESTDIR$PREFIX/bin/kxbps"
install_file "$helper_bin" 755 "$DESTDIR$LIBEXECDIR/kxbps-mirror-apply"
install_file "$repo_helper_bin" 755 "$DESTDIR$LIBEXECDIR/kxbps-repo-apply"

# The module tree: the generated registration files plus every QML file the
# `qmldir` declares, taken from the checkout. `target/cxxqt` keeps copies of
# components that have since been deleted (`cxx-qt-build` copies the
# `.qml_file` list and never removes a file that leaves it), so the build tree
# is not the source of truth for what gets installed.
qml_dest=$DESTDIR$qml_dir/org/kde/kxbps
rm -rf "$qml_dest"
mkdir -p "$qml_dest"
install -Dm644 "$qml_src/qmldir" "$qml_dest/qmldir"
install -Dm644 "$qml_src/plugin.qmltypes" "$qml_dest/plugin.qmltypes"
install -Dm644 "$qml_src/plugin_init.o" "$qml_dest/plugin_init.o"
qml_files=$(awk '$3 ~ /\.qml$/ { print $3 }' "$qml_src/qmldir")
[ -n "$qml_files" ] || die "no QML files listed in $qml_src/qmldir"
for qml_file in $qml_files; do
    require_file "$repo/kxbps-qt/$qml_file"
    install -Dm644 "$repo/kxbps-qt/$qml_file" "$qml_dest/$qml_file"
done
installed="${installed}
$((installed_count + 1)). $qml_dest/ (QML module org.kde.kxbps, $(printf '%s\n' $qml_files | wc -l) QML files)"
installed_count=$((installed_count + 1))
install_file "$icon" 644 \
    "$DESTDIR$PREFIX/share/icons/hicolor/scalable/apps/io.github.khame.kxbps.svg"
install_file "$desktop" 644 \
    "$DESTDIR$PREFIX/share/applications/io.github.khame.kxbps.desktop"
install_file "$metainfo" 644 \
    "$DESTDIR$PREFIX/share/metainfo/io.github.khame.kxbps.metainfo.xml"
install_file "$policy" 644 \
    "$DESTDIR$PREFIX/share/polkit-1/actions/io.github.khame.kxbps.policy"
install_file "$notifyrc" 644 \
    "$DESTDIR$PREFIX/share/knotifications6/kxbps.notifyrc"

# Translations: one .mo per po/<lang>.po at the gettext path KI18n reads.
if [ -d "$PO_DIR" ]; then
    for po in "$PO_DIR"/*.po; do
        [ -f "$po" ] || continue
        lang=$(basename "$po" .po)
        mo_dir=$DESTDIR$PREFIX/share/locale/$lang/LC_MESSAGES
        install -d "$mo_dir"
        msgfmt -c -o "$mo_dir/kxbps.mo" "$po" ||
            die "msgfmt rejected $po"
        installed="${installed}
$((installed_count + 1)). $mo_dir/kxbps.mo"
        installed_count=$((installed_count + 1))
    done
else
    printf 'No %s directory: no translations installed\n' "$PO_DIR" >&2
fi

# --- summary --------------------------------------------------------------

printf '\nInstalled %d items:\n%s\n' "$installed_count" "$installed"
printf '\nQML import path: %s\n' "$qml_dir"
if [ -n "$DESTDIR" ]; then
    printf 'Staged under DESTDIR=%s\n' "$DESTDIR"
fi
