#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later

"""Print a GitHub release body from CHANGELOG.md.

The changelog is the only source: the file in the tree and the text on the
release page cannot disagree, and a version that never shipped is still
described by the section it has.

usage:
    scripts/release-notes.py 0.10.0
    scripts/release-notes.py 0.11.0 0.10.0          # two versions at once
    scripts/release-notes.py 0.10.0 --unreleased    # plus what landed since
    scripts/release-notes.py --intro "text" 0.10.0  # with an opening paragraph

Before tagging, the [Unreleased] section holds the fixes that arrived during
the test phase. Fold them into the version about to ship, then regenerate the
body: the release page and the tree stay one text.
"""

import argparse
import re
import sys
from pathlib import Path

CHANGELOG = Path(__file__).resolve().parent.parent / "CHANGELOG.md"
HEADING = re.compile(r"^## \[(?P<version>[^\]]+)\](?: - (?P<date>.+))?$")


def sections(text):
    """The changelog's sections, keyed by the version inside the brackets."""
    found = {}
    current = None
    for line in text.splitlines():
        heading = HEADING.match(line)
        if heading:
            current = heading.group("version")
            found[current] = {"date": heading.group("date"), "body": []}
            continue
        if line.startswith("## "):
            current = None
            continue
        if current is not None:
            found[current]["body"].append(line)
    return found


def trimmed(lines):
    """The body without the blank lines the section starts and ends with."""
    start = 0
    while start < len(lines) and not lines[start].strip():
        start += 1
    end = len(lines)
    while end > start and not lines[end - 1].strip():
        end -= 1
    return lines[start:end]


def render(section, version):
    date = section["date"]
    title = f"## {version}" + (f" — {date}" if date else "")
    return "\n".join([title, ""] + trimmed(section["body"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("versions", nargs="*", help="versions, newest first")
    parser.add_argument(
        "--unreleased",
        action="store_true",
        help="also print the Unreleased section, for the fixes that landed "
        "after the last build",
    )
    parser.add_argument("--intro", default="", help="a paragraph before the sections")
    args = parser.parse_args()

    if not args.versions and not args.unreleased:
        parser.error("name at least one version, or --unreleased")

    found = sections(CHANGELOG.read_text(encoding="utf-8"))
    blocks = []
    for version in args.versions:
        if version not in found:
            sys.exit(f"CHANGELOG.md has no section for {version}")
        blocks.append(render(found[version], version))
    if args.unreleased:
        body = trimmed(found.get("Unreleased", {"body": []})["body"])
        if body:
            blocks.append("\n".join(["## Unreleased", ""] + body))

    out = []
    if args.intro:
        out.extend([args.intro, ""])
    out.append("\n\n".join(blocks))
    print("\n".join(out))


if __name__ == "__main__":
    main()
