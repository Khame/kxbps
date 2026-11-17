#!/bin/sh
# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regenerate the gettext template and check every catalog against it.
#
# Usage:
#   scripts/extract-i18n.sh            check only; exit 1 when the tree uses an
#                                      msgid the catalogs miss
#   scripts/extract-i18n.sh --write    rewrite po/kxbps.pot and merge it into
#                                      po/pl.po
#
# Options:
#   --include FILE   also read translation keys from FILE (repeatable); use it
#                    for a Rust key file that is not in TRUSTED_FILES yet.
#
# What is read, and how
# ---------------------
# QML (kxbps-qt/qml/**/*.qml) is read by xgettext in JavaScript mode with the
# three KI18n helpers the UI calls:
#
#   i18nd("kxbps", "msgid", args…)        msgid is the second argument
#   i18ndc("kxbps", "context", "msgid")   --keyword=i18ndc:2c,3
#   i18ndp("kxbps", "one", "many", n)     --keyword=i18ndp:2,3
#
# Identifiers rather than sentences (KXbps, kxbps, xbps, Void Linux, pkexec)
# are dropped: a program name is not translated.
#
# Rust never holds a finished sentence. It holds a key that QML turns into one
# (`i18nd("kxbps", …)` inside a `switch (plan.titleKey)`), so those keys have to
# be in the template too. scripts/i18n-rust-keys.py reads the `key()` functions
# of the files in TRUSTED_FILES and keeps every camelCase value they return,
# resolving `pub const` constants (`plan_keys::ERROR_…`) on the way. Paths,
# argv and polkit action ids fail the camelCase test and drop out.
#
# What is NOT read, and what happens then
# ---------------------------------------
# The QML side has no known gap: on gettext 0.22.5 every `i18nd*` call in this
# tree reaches the template, including the ones inside a `switch` before the
# first `case`, in a ternary, or on a continuation line. scripts/check-refs.py
# still compares every call site with the template and prints
# `not extracted: file:line: msgid` for a miss, so a future gettext change or a
# construct it cannot parse is caught instead of shipping an English string in
# a Polish build. The Rust side only sees `key()`; a key produced any other way
# (`let key = "…"; payload.key = key`) is not found — the file for it belongs in
# TRUSTED_FILES once such a function exists.
#
# This script never edits QML or Rust: a miss is reported, not repaired.
#
# Requirements: xgettext, msgfmt, msguniq (Void `gettext`), python3.
# packaging/install.sh needs `msgfmt` from the same package.

set -eu

DOMAIN=kxbps
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
QML_DIR=$ROOT/kxbps-qt/qml
POT=$ROOT/po/$DOMAIN.pot
PL=$ROOT/po/pl.po
# pl.po is the reference catalog: check-coverage.py compares the pot against it
WORK=$(mktemp -d "${TMPDIR:-/tmp}/kxbps-i18n.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

# Every Rust file whose `key()` returns an i18n key QML renders.
TRUSTED_FILES="
kxbps-core/src/probe.rs
kxbps-core/src/mirror_config.rs
kxbps-core/src/mirror_apply.rs
kxbps-core/src/store.rs
kxbps-core/src/transaction.rs
kxbps-qt/src/plan_backend.rs
kxbps-qt/src/job_backend.rs
kxbps-qt/src/mirror_backend.rs
kxbps-qt/src/xbps_snapshot.rs
kxbps-qt/src/xbps_plan.rs
"

write=no
extra_files=

while [ $# -gt 0 ]; do
    case $1 in
        --write) write=yes ;;
        --include)
            if [ $# -lt 2 ]; then
                echo "--include needs a file" >&2
                exit 2
            fi
            extra_files="$extra_files $2"
            shift
            ;;
        -h | --help)
            sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
    shift
done

for tool in xgettext msgfmt msguniq python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$tool not found: install gettext and python3" >&2
        exit 1
    fi
done

# --- 1. QML entries ---------------------------------------------------------

find "$QML_DIR" -name '*.qml' -print | LC_ALL=C sort >"$WORK/qml-files"
if [ ! -s "$WORK/qml-files" ]; then
    echo "no QML files under $QML_DIR" >&2
    exit 1
fi

# shellcheck disable=SC2046 # paths come from find and hold no whitespace
xgettext --language=JavaScript --from-code=UTF-8 \
    --keyword=i18nd:2 --keyword=i18ndc:2c,3 --keyword=i18ndp:2,3 \
    --package-name="$DOMAIN" --add-comments --no-wrap \
    --directory="$ROOT" \
    --output="$WORK/qml.pot" $(sed "s|^$ROOT/||" "$WORK/qml-files")

# --- 1b. C++ entries --------------------------------------------------------
#
# The tray menu is built in C++ (`tray_item.h`) and calls the same KI18n
# helpers as QML. Without this step the next `--write` would drop those two
# labels and the plural from the catalogs, and the tray would fall back to
# English in every language.

find "$ROOT/kxbps-qt/src" \( -name '*.h' -o -name '*.cpp' \) -print \
    | LC_ALL=C sort >"$WORK/cpp-files"
# shellcheck disable=SC2046 # paths come from find and hold no whitespace
xgettext --language=C++ --from-code=UTF-8 \
    --keyword=i18nd:2 --keyword=i18ndc:2c,3 --keyword=i18ndp:2,3 \
    --package-name="$DOMAIN" --add-comments --no-wrap \
    --directory="$ROOT" \
    --output="$WORK/cpp.pot" $(sed "s|^$ROOT/||" "$WORK/cpp-files")

# One template for both languages: msguniq merges the duplicates later.
msgcat --no-wrap --output-file="$WORK/strings.pot" "$WORK/qml.pot" "$WORK/cpp.pot"
cat "$WORK/qml-files" "$WORK/cpp-files" >"$WORK/source-files"

# --- 2. Rust keys -----------------------------------------------------------

rust_files=
for file in $TRUSTED_FILES $extra_files; do
    if [ -f "$ROOT/$file" ]; then
        rust_files="$rust_files $ROOT/$file"
    else
        echo "warning: $file missing, its keys are not read" >&2
    fi
done
# shellcheck disable=SC2086 # the file list is expanded on purpose
python3 "$ROOT/scripts/i18n-rust-keys.py" $rust_files >"$WORK/rust-keys.txt"

# --- 3. Assemble and order --------------------------------------------------

python3 "$ROOT/scripts/assemble-pot.py" \
    "$WORK/strings.pot" "$WORK/rust-keys.txt" "$WORK/$DOMAIN.unsorted.pot"
# msguniq merges duplicate msgids (reference lists are concatenated); it has no
# stable sort of its own, so assemble-pot.py already wrote the entries sorted.
msguniq --no-wrap --output-file="$WORK/$DOMAIN.pot" "$WORK/$DOMAIN.unsorted.pot"

# --- 4. Checks --------------------------------------------------------------

fail=0

if ! python3 "$ROOT/scripts/check-refs.py" "$WORK/$DOMAIN.pot" "$ROOT" "$WORK/source-files"; then
    fail=1
fi

if ! python3 "$ROOT/scripts/check-coverage.py" "$WORK/$DOMAIN.pot" "$PL"; then
    fail=1
fi

for file in "$ROOT"/po/*.po; do
    [ -f "$file" ] || continue
    if ! python3 "$ROOT/scripts/check-placeholders.py" "$file"; then
        fail=1
    fi
    if ! python3 "$ROOT/scripts/check-catalogs.py" "$WORK/$DOMAIN.pot" "$file"; then
        fail=1
    fi
done

# --- 5. Write ---------------------------------------------------------------

if [ "$write" = yes ]; then
    cp "$WORK/$DOMAIN.pot" "$POT"
    if [ -f "$PL" ]; then
        msgmerge --quiet --no-wrap --update --backup=none "$PL" "$POT"
    else
        msginit --quiet --no-wrap --input="$POT" --locale=pl --output-file="$PL"
    fi
    python3 "$ROOT/scripts/po-header.py" "$PL" "$ROOT"
    msgfmt --check --output=/dev/null "$PL"
    echo "wrote po/$DOMAIN.pot and merged po/pl.po"
fi

if [ "$fail" -ne 0 ]; then
    exit 1
fi
