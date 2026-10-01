#!/usr/bin/env python3
"""Bump the workspace version and its path packages without resolving dependencies."""

import argparse
from pathlib import Path
import re
import sys
import tomllib


def bump_version(root: Path, bump: str) -> str:
    manifest_path = root / "Cargo.toml"
    manifest_text = manifest_path.read_text()
    workspace = tomllib.loads(manifest_text)["workspace"]
    current = workspace["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", current):
        raise ValueError(f"workspace version must be a stable semantic version: {current}")
    if bump == "current":
        return current
    major, minor, patch = map(int, current.split("."))
    versions = {
        "patch": (major, minor, patch + 1),
        "minor": (major, minor + 1, 0),
        "major": (major + 1, 0, 0),
    }
    version = ".".join(map(str, versions[bump]))

    excluded = {
        path.resolve()
        for pattern in workspace.get("exclude", [])
        for path in root.glob(pattern)
    }
    names = set()
    for pattern in workspace["members"]:
        for member in root.glob(pattern):
            if member.resolve() in excluded:
                continue
            package = tomllib.loads((member / "Cargo.toml").read_text())["package"]
            if package.get("version") == {"workspace": True}:
                names.add(package["name"])
    if not names:
        raise ValueError("no workspace packages inherit the release version")

    lock_path = root / "Cargo.lock"
    lock_text = lock_path.read_text()
    packages = tomllib.loads(lock_text)["package"]
    for name in sorted(names):
        entries = [entry for entry in packages if entry["name"] == name and "source" not in entry]
        if len(entries) != 1 or entries[0]["version"] != current:
            raise ValueError(f"Cargo.lock does not have exactly one {name} path package at {current}")

    chunks = re.split(r"(?m)(?=^\[\[package\]\]$)", lock_text)
    for index, chunk in enumerate(chunks):
        if not chunk.startswith("[[package]]"):
            continue
        package = tomllib.loads(chunk)["package"][0]
        if package["name"] in names and "source" not in package:
            chunks[index] = re.sub(
                r'(?m)^(version\s*=\s*)"[^"]+"',
                lambda match: f'{match[1]}"{version}"',
                chunk,
                count=1,
            )
    updated_lock = "".join(chunks)
    # Cargo qualifies dependencies by version when a package name is ambiguous.
    # Registry-qualified entries remain untouched, even at the same old version.
    for name in names:
        updated_lock = updated_lock.replace(f'"{name} {current}"', f'"{name} {version}"')

    section = re.search(r"(?m)^\[workspace\.package\]\s*\n([\s\S]*?)(?=^\[|\Z)", manifest_text)
    if section is None:
        raise ValueError("Cargo.toml is missing the workspace.package section")
    body, count = re.subn(
        r'(?m)^(version\s*=\s*)"[^"]+"',
        lambda match: f'{match[1]}"{version}"',
        section[1],
        count=1,
    )
    if count != 1:
        raise ValueError("Cargo.toml is missing the workspace version assignment")
    updated_manifest = manifest_text[:section.start(1)] + body + manifest_text[section.end(1):]
    # Validate both edits before writing either file.
    tomllib.loads(updated_manifest)
    tomllib.loads(updated_lock)
    manifest_path.write_text(updated_manifest)
    lock_path.write_text(updated_lock)
    return version


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bump", choices=("patch", "minor", "major", "current"))
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    try:
        print(bump_version(args.root, args.bump))
    except (KeyError, OSError, ValueError) as error:
        print(f"bump-version: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
