# SPDX-License-Identifier: GPL-3.0-or-later
"""Every msgid in the template must exist in pl.po and be translated.

usage: check-coverage.py POT PO

Prints the counts the report quotes and lists each offending msgid on stderr.
Exit status 1 when anything is untranslated, missing or stale.
"""
import sys

from po_parse import entries

pot, po = sys.argv[1], sys.argv[2]

pot_entries = [msgid for msgid, _ in entries(pot)]
po_entries = {msgid: forms for msgid, forms in entries(po)}


def translated(forms):
    return bool(forms) and all(value for value in forms.values())


known = set(pot_entries)
untranslated = [msgid for msgid in pot_entries if not translated(po_entries.get(msgid, {}))]
missing = [msgid for msgid in pot_entries if msgid not in po_entries]
stale = [msgid for msgid in po_entries if msgid not in known]

print(f"msgids in pot:        {len(pot_entries)}")
print(f"translated in pl.po:  {sum(1 for m in known if translated(po_entries.get(m, {})))}")
print(f"untranslated:         {len(untranslated)}")
print(f"missing from pl.po:   {len(missing)}")
print(f"stale in pl.po:       {len(stale)}")

status = 0
for label, values in (("untranslated", untranslated), ("missing", missing), ("stale", stale)):
    for value in values:
        print(f"{label}: {value}", file=sys.stderr)
        status = 1
sys.exit(status)
