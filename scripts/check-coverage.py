# SPDX-License-Identifier: GPL-3.0-or-later
"""Every msgid in the template must exist in pl.po and be translated.

usage: check-coverage.py POT PO

Prints the counts the report quotes and lists each offending msgid on stderr.
Exit status 1 when anything is untranslated, missing, stale or only a
`msgmerge` guess.

A fuzzy entry counts as untranslated on purpose: `msgfmt` drops it, so the
sentence would reach the UI in English while every count still said 100%.
"""
import sys

from po_parse import entries, fuzzy_ids

pot, po = sys.argv[1], sys.argv[2]

pot_entries = [msgid for msgid, _ in entries(pot)]
po_entries = {msgid: forms for msgid, forms in entries(po)}
fuzzy = fuzzy_ids(po)


def translated(forms):
    return bool(forms) and all(value for value in forms.values())


known = set(pot_entries)
untranslated = [msgid for msgid in pot_entries if not translated(po_entries.get(msgid, {}))]
guessed = [msgid for msgid in pot_entries if msgid in fuzzy]
missing = [msgid for msgid in pot_entries if msgid not in known]
stale = [msgid for msgid in po_entries if msgid not in known]

print(f"msgids in pot:        {len(pot_entries)}")
print(
    f"translated in pl.po:  "
    f"{sum(1 for m in known if translated(po_entries.get(m, {})) and m not in fuzzy)}"
)
print(f"untranslated:         {len(untranslated)}")
print(f"fuzzy:                {len(guessed)}")
print(f"missing from pl.po:   {len(missing)}")
print(f"stale in pl.po:       {len(stale)}")

status = 0
for label, values in (
    ("untranslated", untranslated),
    ("fuzzy", guessed),
    ("missing", missing),
    ("stale", stale),
):
    for value in values:
        print(f"{label}: {value}", file=sys.stderr)
        status = 1
sys.exit(status)
