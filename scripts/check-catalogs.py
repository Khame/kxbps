# SPDX-License-Identifier: GPL-3.0-or-later
"""Check gettext catalogs for completeness against the template.

usage: check-catalogs.py POT PO [PO ...]

Every catalog after the template must carry the same number of entries as the
template, no entry may have an empty msgstr and no entry may be fuzzy. A fuzzy
entry matters because msgfmt drops it and the user then sees English.

Prints one line per catalog in the form `<file>: <n> entries, <k> empty, <f> fuzzy`
and exits 1 when any counter is wrong.
"""
import sys

from po_parse import entries, fuzzy_ids


def main():
    if len(sys.argv) < 3:
        print("usage: check-catalogs.py POT PO [PO ...]", file=sys.stderr)
        return 2
    pot = dict(entries(sys.argv[1]))
    status = 0
    for po_path in sys.argv[2:]:
        catalog = dict(entries(po_path))
        fuzzy = fuzzy_ids(po_path)
        empty = sum(
            1 for forms in catalog.values() if not forms or not any(forms.values())
        )
        missing = sorted(pot.keys() - catalog.keys())
        extra = sorted(catalog.keys() - pot.keys())
        print(f"{po_path}: {len(catalog)} entries, {empty} empty, {len(fuzzy)} fuzzy")
        for msgid in missing:
            print(f"{po_path}: missing: {msgid}")
        for msgid in extra:
            print(f"{po_path}: not in the template: {msgid}")
        if len(catalog) != len(pot) or empty or fuzzy or missing or extra:
            status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
