#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Check a pull request's Sherpa version (Python 3.15+, standard library only).

Every crate inherits the workspace version, and Cargo.lock has the workspace crates at it. A pull
request leaves the version alone, unless it is a bump pull request (`mise run release:bump`):
one that changes only Cargo.toml, Cargo.lock and CHANGELOG.md, sets a version higher than the
last v* tag, and has a CHANGELOG.md section for it. See docs/architecture/RELEASE.md.

Run from the root of a checkout of the pull request's merge commit:

    check_version.py BASE_REV

BASE_REV is the commit the pull request merges into.
"""

import re
import subprocess
import sys
import tomllib

BUMP_FILES = {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
# A version's CHANGELOG.md section starts with `## [X.Y.Z] - YYYY-MM-DD`.
SECTION = re.compile(r"^## \[(?P<version>[^\]]+)\]", re.MULTILINE)


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def toml_at(rev, path):
    """Parse a TOML file at a revision, or None if the file is not there."""
    try:
        return tomllib.loads(git("show", f"{rev}:{path}"))
    except subprocess.CalledProcessError:
        return None


def workspace_version(manifest):
    return manifest.get("workspace", {}).get("package", {}).get("version")


def base_version(rev):
    version = workspace_version(toml_at(rev, "Cargo.toml") or {})
    if version is None:
        # Before the workspace version, every crate carried the version; the client's is the one
        # the release used.
        client = toml_at(rev, "crates/client/Cargo.toml") or {}
        version = client.get("package", {}).get("version")
    if version is None:
        sys.exit(f"cannot read the version at {rev}")
    return version


def check_head():
    """Check that every crate inherits the workspace version, and return it."""
    with open("Cargo.toml", "rb") as f:
        root = tomllib.load(f)
    version = workspace_version(root)
    if version is None:
        return ["Cargo.toml has no [workspace.package] version."], None

    errors = []
    names = set()
    for member in root["workspace"]["members"]:
        with open(f"{member}/Cargo.toml", "rb") as f:
            package = tomllib.load(f)["package"]
        names.add(package["name"])
        if package.get("version") != {"workspace": True}:
            errors.append(
                f"{member}/Cargo.toml sets its own version; "
                "use `version.workspace = true`."
            )

    with open("Cargo.lock", "rb") as f:
        lock = tomllib.load(f)
    stale = sorted(
        package["name"]
        for package in lock["package"]
        if package["name"] in names
        and "source" not in package
        and package["version"] != version
    )
    if stale:
        errors.append(
            f"Cargo.lock does not have {', '.join(stale)} at {version}; "
            "run `cargo update --workspace`."
        )
    return errors, version


def parse(version):
    match = SEMVER.match(version)
    return tuple(int(part) for part in match.groups()) if match else None


def last_tag():
    versions = [parse(tag[1:]) for tag in git("tag", "--list", "v[0-9]*").split()]
    versions = [v for v in versions if v is not None]
    return max(versions) if versions else None


def has_section(version):
    try:
        with open("CHANGELOG.md", encoding="utf-8") as f:
            changelog = f.read()
    except FileNotFoundError:
        return False
    return any(m["version"] == version for m in SECTION.finditer(changelog))


def check_bump(base, version):
    errors = []
    if parse(version) is None:
        errors.append(f"{version} is not a X.Y.Z version.")
    else:
        latest = last_tag()
        if latest is not None and parse(version) <= latest:
            errors.append(
                f"{version} is not higher than the last release, "
                f"{'.'.join(map(str, latest))}."
            )
    if not has_section(version):
        errors.append(
            f"CHANGELOG.md has no ## [{version}] section; run `mise run release:bump`."
        )
    changed = set(git("diff", "--name-only", base, "HEAD").split())
    extra = sorted(changed - BUMP_FILES)
    if extra:
        errors.append(
            "A pull request that changes the version changes only "
            f"{', '.join(sorted(BUMP_FILES))}; this one also changes {', '.join(extra)}."
        )
    return errors


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = sys.argv[1]

    errors, version = check_head()
    if version is not None:
        before = base_version(base)
        if version != before:
            errors += check_bump(base, version)
            if not errors:
                print(f"Version bump: {before} -> {version}.")
        elif not errors:
            print(f"The version is unchanged at {version}.")

    for error in errors:
        print(f"::error::{error}")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
