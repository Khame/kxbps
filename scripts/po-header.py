# SPDX-License-Identifier: GPL-3.0-or-later
"""Make po/pl.po ready to commit after msgmerge.

usage: po-header.py PL ROOT

* strips the absolute build path from every `#:` reference, so the file says
  `kxbps-qt/qml/…` no matter where the tree was checked out;
* takes `Project-Id-Version` and `POT-Creation-Date` from the template beside
  it, so the catalog cannot claim a version the pot does not;
* fills the header fields msgfmt warns about (`PO-Revision-Date`,
  `Last-Translator`) when they are missing;
* keeps `Plural-Forms` exactly as it was.
"""
import datetime
import pathlib
import re
import sys

pl_path, root = sys.argv[1], sys.argv[2]
text = open(pl_path, encoding="utf-8").read()

text = text.replace(root + "/", "")

header, _, body = text.partition("\n\n")
if not header.lstrip().startswith("#"):
    sys.exit(f"{pl_path}: no comment/header block")

pot_path = pathlib.Path(pl_path).parent / "kxbps.pot"
if pot_path.is_file():
    pot_header = pot_path.read_text(encoding="utf-8").split("\n\n")[0]
    for field in ("Project-Id-Version", "POT-Creation-Date"):
        value = re.search(rf'^"{field}: (.*)\\n"$', pot_header, re.M)
        if not value:
            continue
        header = re.sub(
            rf'^"{field}: .*\\n"$',
            lambda _match: f'"{field}: {value.group(1)}\\n"',
            header,
            flags=re.M,
        )

now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M+0000")
translator = "KXbps translation contributors <khame.dev@protonmail.com>"

fields = []
for line in header.split("\n"):
    fields.append(line)

joined = "\n".join(fields)
if "PO-Revision-Date:" not in joined:
    joined = joined.replace(
        '"Language-Team:', f'"PO-Revision-Date: {now}\\n"\n"Language-Team:', 1
    )
if "Last-Translator:" not in joined:
    joined = joined.replace(
        '"Language-Team:', f'"Last-Translator: {translator}\\n"\n"Language-Team:', 1
    )

open(pl_path, "w", encoding="utf-8").write(joined + "\n\n" + body)
