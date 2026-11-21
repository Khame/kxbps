#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later

"""Check that the version and the source checksum agree everywhere.

The same release fact is written in five places: the workspace manifest, the
manual page, the AppStream metadata, the changelog, and the `xbps-src` template.
A tag cut while one of them lags produces a package nobody can build from a clean
tree, so this exits nonzero on the first mismatch.

usage: verify-release.py [--void-packages DIR] [--tarball FILE]

Without `--tarball` the checksum check is skipped: the file is not in the
repository, only the template's copy of its digest is.
"""

import argparse
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    print(f"FAIL {message}", file=sys.stderr)
    raise SystemExit(1)


def check(label: str, actual: str, expected: str) -> None:
    if actual != expected:
        fail(f"{label}: {actual!r}, expected {expected!r}")
    print(f"ok   {label}: {actual}")


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version = "([^"]+)"$', text, re.MULTILINE)
    if not match:
        fail("Cargo.toml has no workspace version")
    return match.group(1)


def man_version() -> str:
    first = (ROOT / "man/kxbps.1").read_text(encoding="utf-8").splitlines()[0]
    match = re.search(r'"(kxbps [^"]+)"', first)
    if not match:
        fail("man/kxbps.1 has no version in its .TH line")
    return match.group(1).removeprefix("kxbps ")


def metainfo_has(version: str) -> str:
    text = (ROOT / "packaging/io.github.khame.kxbps.metainfo.xml").read_text(encoding="utf-8")
    match = re.search(r'<release version="([^"]+)"', text)
    return match.group(1) if match else ""


def changelog_has(version: str) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    return version if f"## [{version}]" in text else ""


def template_values(path: Path) -> dict[str, str]:
    values = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.match(r"^(version|revision|checksum)=(\S+)$", line)
        if match:
            values[match.group(1)] = match.group(2)
    missing = {"version", "revision", "checksum"} - values.keys()
    if missing:
        fail(f"{path} has no {', '.join(sorted(missing))}")
    return values


def template_depends(path: Path) -> list[str]:
    """depends from the template; INSTALL.md repeats it and can drift."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r'^depends="([^"]+)"', text, re.MULTILINE | re.DOTALL)
    if not match:
        fail(f"{path} has no depends")
    return match.group(1).split()


def undocumented(depends: list[str]) -> list[str]:
    text = (ROOT / "INSTALL.md").read_text(encoding="utf-8")
    return [name for name in depends if name not in text]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--void-packages", help="a void-packages checkout with srcpkgs/kxbps")
    parser.add_argument("--tarball", help="the source tarball to hash")
    args = parser.parse_args()

    version = workspace_version()
    print(f"version {version}")

    check("Cargo.toml", version, version)
    check("man/kxbps.1", man_version(), version)
    check("metainfo", metainfo_has(version), version)
    check("CHANGELOG.md", changelog_has(version), version)

    template = ROOT / "packaging/void/template"
    values = template_values(template)
    check("template version", values["version"], version)

    depends = template_depends(template)
    missing = undocumented(depends)
    if missing:
        fail(f"INSTALL.md does not name: {', '.join(missing)}")
    print(f"ok   INSTALL.md names every template dependency: {len(depends)} packages")

    if args.tarball:
        tarball = Path(args.tarball)
        if not tarball.is_file():
            fail(f"{tarball} does not exist")
        digest = hashlib.sha256(tarball.read_bytes()).hexdigest()
        check("tarball checksum", digest, values["checksum"])
        expected = f"kxbps-{version}.tar.gz"
        check("tarball name", tarball.name, expected)
    else:
        print("skip tarball checksum: no --tarball given")

    if args.void_packages:
        copy = Path(args.void_packages) / "srcpkgs/kxbps/template"
        if not copy.is_file():
            fail(f"{copy} does not exist")
        other = template_values(copy)
        check("void-packages version", other["version"], values["version"])
        check("void-packages revision", other["revision"], values["revision"])
        check("void-packages checksum", other["checksum"], values["checksum"])
    else:
        print("skip void-packages copy: no --void-packages given")

    print("all checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
