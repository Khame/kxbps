# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""Every i18nd* msgid in QML must reach the template from the same file.

usage: check-refs.py POT ROOT FILES_LIST

The check compares msgids, not line numbers: xgettext's `#: file:line` points at
the line it happened to be on (the call, an element's opening line, or a line
inside a comment block), so a line comparison reports false misses and can also
hide one when two calls share a line. What matters is whether the string the
call uses made it into the template at all, attributed to that file.
"""
import os
import re
import sys

# Calls that name a program are dropped from the template on purpose.
IDENTIFIERS = {"KXbps", "kxbps", "xbps", "Void Linux", "pkexec"}
CALL_RE = re.compile(r'\bi18nd[a-z]?\(\s*"kxbps"\s*,\s*"((?:[^"\\]|\\.)*)"', re.S)

pot, root, listing = sys.argv[1], sys.argv[2], sys.argv[3]

# msgid -> set of files that reference it
reached = {}
for block in open(pot, encoding="utf-8").read().split("\n\n"):
    match = re.search(r'^msgid "(?!")((?:[^"\\]|\\.)*)"', block, re.M)
    if not match:
        continue
    files = reached.setdefault(match.group(1), set())
    for ref_match in re.finditer(r'^#: (.*)$', block, re.M):
        for ref in ref_match.group(1).split():
            path, _, _ = ref.rpartition(":")
            files.add(path)

missing = []
for name in open(listing, encoding="utf-8").read().split():
    rel = os.path.relpath(name, root)
    text = open(name, encoding="utf-8").read()
    # A call inside a comment is not a call.
    stripped = re.sub(r'/\*.*?\*/', '', text, flags=re.S)
    stripped = re.sub(r'//[^\n]*', '', stripped)
    for match in CALL_RE.finditer(stripped):
        msgid = match.group(1)
        if msgid in IDENTIFIERS:
            continue
        if rel not in reached.get(msgid, set()):
            line = stripped.count("\n", 0, match.start()) + 1
            missing.append(f"{rel}:{line}: {msgid}")

# One report line per unique (file, msgid): a literal used twice in one file
# fails once, the first time it appears.
seen = set()
for entry in missing:
    key = entry.split(": ", 1)
    if key[0] in seen:
        continue
    seen.add(key[0])
    print(f"not extracted: {entry}", file=sys.stderr)

sys.exit(1 if missing else 0)
