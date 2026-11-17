# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check that every translated entry keeps its placeholders and no more.

usage: check-placeholders.py PO
"""
import re
import sys

from po_parse import entries

path = sys.argv[1]
problems = 0
for msgid, forms in entries(path):
    want = sorted(set(re.findall(r"%\d|%%|%n", msgid)))
    for form, value in forms.items():
        got = sorted(set(re.findall(r"%\d|%%|%n", value)))
        if got != want:
            problems += 1
            print(f"placeholder mismatch [{form}] {msgid!r} -> {value!r} (want {want}, got {got})")
        # An entry must not add a placeholder the source does not have, must not
        # drop one, and must not carry a stray percent sign.
        if "%" in re.sub(r"%\d|%%|%n", "", value):
            problems += 1
            print(f"stray percent [{form}] {msgid!r} -> {value!r}")
print(f"placeholder problems: {problems}")
sys.exit(1 if problems else 0)
