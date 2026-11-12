# SPDX-License-Identifier: GPL-3.0-or-later
"""Print the translation keys the listed Rust files hand to QML.

A key function (`fn key(&self) -> &'static str`) returns one of:
  - a camelCase string literal  -> kept
  - a `pub const X: &str` name  -> resolved to its value, kept if camelCase
  - anything else (path, argv, polkit id, lowercase token) -> dropped
"""
import re
import sys

CONST_RE = re.compile(r'pub const ([A-Z0-9_]+): &(?:\'static )?str = "((?:[^"\\]|\\.)*)";')
KEY_SHAPE = re.compile(r'^[a-z][A-Za-z0-9_]*$')
FN_RE = re.compile(r'\n\s*(?:pub )?(?:const )?fn ([a-z_0-9]+)\([^)]*\)[^{;]*\{')


def bodies(text):
    """Every function body in the file, by name (in source order)."""
    out = {}
    for match in FN_RE.finditer(text):
        depth = 1
        i = match.end()
        while i < len(text) and depth:
            if text[i] == '{':
                depth += 1
            elif text[i] == '}':
                depth -= 1
            i += 1
        out.setdefault(match.group(1), []).append(text[match.end():i - 1])
    return out


def main():
    keys = {}
    for path in sys.argv[1:]:
        try:
            text = open(path, encoding="utf-8").read()
        except OSError as error:
            print(f"cannot read {path}: {error}", file=sys.stderr)
            continue
        consts = {name: value for name, value in CONST_RE.findall(text)}
        for body in bodies(text).get("key", []):
            # A `match` arm is one `A => B,`; splitting at `=>` keeps the arms
            # apart, so an arm's literal cannot pick up the next arm's.
            for arm in body.split("=>"):
                for literal in re.findall(r'"((?:[^"\\]|\\.)*)"', arm):
                    value = consts.get(literal, literal)
                    # A key is one camelCase word (`jobXbpsFailed`); paths,
                    # argv, polkit ids and lowercase tokens are not keys.
                    if KEY_SHAPE.match(value) and any(c.isupper() for c in value):
                        keys.setdefault(value, set()).add(path)
    for key in sorted(keys):
        print(key)


main()
