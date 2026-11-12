# SPDX-License-Identifier: GPL-3.0-or-later
"""Assemble po/kxbps.pot from the QML template and the Rust keys.

usage: assemble-pot.py QML_POT RUST_KEYS OUT

Rust-key entries come first and carry no `#:` reference; QML entries follow in
msgid order and keep every source position.
"""
import datetime
import re
import sys

# Program names, not sentences: a bare one is not worth a translation. A msgid
# that merely contains one ("About KXbps") is a sentence and stays.
IDENTIFIERS = {"KXbps", "kxbps", "xbps", "Void Linux", "pkexec"}

qml_pot, rust_keys, out_path = sys.argv[1], sys.argv[2], sys.argv[3]

keys = [
    key
    for key in open(rust_keys, encoding="utf-8").read().split("\n")
    if key and key not in IDENTIFIERS
]
key_set = set(keys)

blocks = [b for b in open(qml_pot, encoding="utf-8").read().split("\n\n") if b.strip()]
header, entries = blocks[0], blocks[1:]
if not header.lstrip().startswith("#") or 'msgid ""' not in header:
    sys.exit(f"{qml_pot}: first block is not a gettext header")

merged = {}
order = []
for block in entries:
    # `(?!"")` skips an untranslated header if one ever shows up mid-file.
    match = re.search(r'^msgid "(?!")((?:[^"\\]|\\.)*)"', block, re.M)
    if not match:
        continue
    msgid = match.group(1)
    if msgid in IDENTIFIERS and "msgid_plural" not in block:
        continue
    if msgid not in merged:
        merged[msgid] = {"refs": [], "block": block}
        order.append(msgid)
    for ref_match in re.finditer(r'^#: (.*)$', block, re.M):
        for ref in ref_match.group(1).split():
            if ref not in merged[msgid]["refs"]:
                merged[msgid]["refs"].append(ref)

now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M+0000")

out = [
    "# Translation template for KXbps.",
    "# Copyright (C) 2026 KXbps contributors",
    "# This file is distributed under the same license as the KXbps package.",
    "#",
    '# Entries without a "file:line" reference are Rust keys: QML maps each to',
    "# a sentence with i18nd(). scripts/extract-i18n.sh lists the files read.",
    'msgid ""',
    'msgstr ""',
]
for field in (
    "Project-Id-Version: kxbps 0.1.0",
    "Report-Msgid-Bugs-To: ",
    f"POT-Creation-Date: {now}",
    "PO-Revision-Date: YEAR-MO-DA HO:MI+ZONE",
    "Last-Translator: FULL NAME <EMAIL@ADDRESS>",
    "Language-Team: English",
    "Language: en",
    "MIME-Version: 1.0",
    "Content-Type: text/plain; charset=UTF-8",
    "Content-Transfer-Encoding: 8bit",
):
    out.append(f'"{field}\\n"')
out.append("")

for key in keys:
    out.append("#. key emitted by Rust, mapped to a sentence in QML")
    out.append(f'msgid "{key}"')
    out.append('msgstr ""')
    out.append("")

for msgid in sorted(order):
    if msgid in key_set:
        continue  # already written above, from the Rust side
    entry = merged[msgid]
    block = re.sub(r'^#: .*\n', "", entry["block"], flags=re.M).strip("\n")
    out.append("".join(f"#: {ref}\n" for ref in entry["refs"]) + block)
    out.append("")

open(out_path, "w", encoding="utf-8").write("\n".join(out).rstrip("\n") + "\n")
