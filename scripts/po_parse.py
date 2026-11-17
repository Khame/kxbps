# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
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


def _parse_block(block):
    """One PO block as `(msgid, {form: text}, fuzzy)`, or `None` for the header.

    A block's flags live in a `#,` comment, and `msgmerge` sets `fuzzy` on every
    translation it guesses: `msgfmt` then leaves that entry out of the `.mo`, so
    the string ships in English. The flag is read here because a caller that
    only sees msgids cannot tell a guessed translation from a checked one.
    """
    fields = {}
    current = None
    fuzzy = False
    for line in block.splitlines():
        if line.startswith("#"):
            # `#, fuzzy` or `#, fuzzy, c-format`: flags are a comma list after
            # `#,`, and only the exact word `fuzzy` counts.
            if line.startswith("#,") and any(
                flag.strip() == "fuzzy" for flag in line[2:].split(",")
            ):
                fuzzy = True
            current = None
            continue
        if not line.strip():
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
        return None
    msgid = unescape(msgid)
    if not msgid:
        return None

    forms = {}
    for (name, form), value in fields.items():
        if name.startswith("msgstr"):
            forms[form] = unescape(value)
    return msgid, forms, fuzzy


def entries(path):
    """Every message as `(msgid, {form: text})`, `form` None for a singular.

    The header block (empty msgid) is skipped, as are comments. Obsolete
    entries start with `#~` and are ignored by the comment filter, which is what
    a stale entry should be: not part of the catalog any more.
    """
    for msgid, forms, _ in _all_blocks(path):
        yield msgid, forms


def fuzzy_ids(path):
    """The msgids whose translation is still a `msgmerge` guess."""
    return {msgid for msgid, _, fuzzy in _all_blocks(path) if fuzzy}


def _all_blocks(path):
    with open(path, encoding="utf-8") as handle:
        text = handle.read()

    for block in re.split(r"\n\s*\n", text):
        parsed = _parse_block(block)
        if parsed is not None:
            yield parsed
