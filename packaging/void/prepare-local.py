#!/usr/bin/env python3
"""Create the source tarball for a KXbps release and print its SHA-256.

The tarball is what the `xbps-src` template downloads, so it has to contain
exactly the tracked files of a commit: `git archive` writes those and nothing
else, which keeps `target/`, `screenshots/` and the rest of the
working tree out of the release.

usage: prepare-local.py [--output DIR]

The file is written as `kxbps-<version>.tar.gz`; the version comes from
`Cargo.toml`, the same place the binary reads it from.
"""

import argparse
import gzip
import hashlib
import io
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def version() -> str:
    for line in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("version") and "=" in line:
            return line.split("=", 1)[1].strip().strip('"')
    raise SystemExit("Cargo.toml has no workspace version")


def _normalise(archive: bytes) -> bytes:
    """Drop the global pax header and pin every timestamp.

    `git archive` writes a `pax_global_header` naming the commit and stamps each
    member with the commit time. Both make two archives of the same tree differ,
    so they are removed here.
    """
    source = tarfile.open(fileobj=io.BytesIO(archive))
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as target:
        for member in source.getmembers():
            if member.name == "pax_global_header":
                continue
            member.mtime = 0
            target.addfile(member, source.extractfile(member))
    return buffer.getvalue()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default=str(ROOT), help="directory for the tarball")
    args = parser.parse_args()

    dirty = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain", "-uall"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    if dirty:
        print("the working tree is not clean; the tarball would not match a tag:", file=sys.stderr)
        print(dirty, file=sys.stderr)
        return 1

    version_ = version()
    name = f"kxbps-{version_}.tar.gz"
    out = Path(args.output) / name

    # `git archive` honours `export-ignore` in .gitattributes, so the maintainer
    # notes and packaging/void stay out of the release. Its tar also records the
    # commit id and the commit time, which would change the digest on every
    # commit even when the tree did not change — and a checksum that invalidates
    # itself in the commit that writes it is worse than none. The archive is
    # rewritten with a fixed timestamp and no global header, so the digest
    # depends on the tree alone.
    archive = subprocess.run(
        ["git", "-C", str(ROOT), "archive", "--format=tar", f"--prefix=kxbps-{version_}/", "HEAD"],
        capture_output=True,
        check=True,
    ).stdout
    out.write_bytes(gzip.compress(_normalise(archive), compresslevel=9, mtime=0))

    with tarfile.open(out) as tar:
        members = tar.getmembers()
    prefix = f"kxbps-{version_}"
    if not all(m.name == prefix or m.name.startswith(prefix + "/") for m in members):
        print(f"{out} has more than one top-level directory", file=sys.stderr)
        return 1
    files = [m for m in members if m.isfile()]

    digest = hashlib.sha256(out.read_bytes()).hexdigest()
    print(out)
    print(f"files: {len(files)}")
    print(f"sha256: {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
