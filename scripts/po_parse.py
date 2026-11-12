# SPDX-License-Identifier: GPL-3.0-or-later
"""Read the subset of PO syntax the pipeline needs.

gettext tools re-wrap long strings, so a msgid or a msgstr may span several
lines: the first line carries the directive and each continuation is a bare
quoted string. Every parser here joins those continuations before looking at
the text, otherwise a re-wrapped catalog reads as untranslated.

The module is deliberately small: the pipeline only needs the msgid, the
msgstr forms and the placeholders, and pulling in a PO library for that would
add a dependency to a script that runs on a bare Void install.
"""
import re

_DIRECTIVE = re.compile(r'^(msgid_plural|msgid|msgstr(?:\[(\d+)\])?)\s+"(.*)"\s*$')
_CONTINUATION = re.compile(r'^"(.*)"\s*$')


def unescape(text):
    """Undo the C escapes gettext writes, without touching the UTF-8 bytes."""
    out = []
    index = 0
    while index < len(text):
        char = text[index]
        if char == "\\" and index + 1 < len(text):
            escaped = text[index + 1]
            out.append({"n": "\n", "t": "\t", "r": "\r"}.get(escaped, escaped))
            index += 2
        else:
            out.append(char)
            index += 1
    return "".join(out)


def entries(path):
    """Every message as `(msgid, {form: text})`, `form` None for a singular.

    The header block (empty msgid) is skipped, as are comments. Obsolete
    entries start with `#~` and are ignored by the comment filter, which is what
    a stale entry should be: not part of the catalog any more.
    """
    with open(path, encoding="utf-8") as handle:
        text = handle.read()

    for block in re.split(r"\n\s*\n", text):
        fields = {}
        current = None
        for line in block.splitlines():
            if not line.strip() or line.startswith("#"):
                current = None
                continue
            match = _DIRECTIVE.match(line)
            if match:
                name, form, value = match.group(1), match.group(2), match.group(3)
                key = (name, form)
                fields[key] = value
                current = key
                continue
            match = _CONTINUATION.match(line)
            if match and current is not None:
                fields[current] += match.group(1)
                continue
            current = None

        msgid = fields.get(("msgid", None))
        if msgid is None:
            continue
        msgid = unescape(msgid)
        if not msgid:
            continue

        forms = {}
        for (name, form), value in fields.items():
            if name.startswith("msgstr"):
                forms[form] = unescape(value)
        yield msgid, forms
