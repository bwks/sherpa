#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Check a pull request's Sherpa version (Python 3.15+, standard library only).

Pull requests do not change the version. The one exception is the release pull request that the
Prepare release workflow opens: it is authored by the release app, from a `release/vX.Y.Z` branch
of this repository, sets the workspace version to X.Y.Z, and changes only the files a release
changes. See docs/architecture/RELEASE.md.

Run from the root of a checkout of the pull request's merge commit:

    check_version.py BASE_REV

BASE_REV is the commit the pull request merges into. The pull request is described by the
environment: PR_AUTHOR (the author's login), PR_HEAD_REF (the head branch), PR_HEAD_REPO (the
head repository, owner/name) and REPOSITORY (this repository, owner/name).
"""

import os
import re
import subprocess
import sys
import tomllib

RELEASE_AUTHOR = "bwks-sherpa-release[bot]"
RELEASE_BRANCH_PREFIX = "release/v"
RELEASE_FILES = {"Cargo.toml", "Cargo.lock", "CHANGELOG.md"}
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")


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


def check_release(base, version):
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
    changed = set(git("diff", "--name-only", base, "HEAD").split())
    extra = sorted(changed - RELEASE_FILES)
    if extra:
        errors.append(
            "A release pull request changes only "
            f"{', '.join(sorted(RELEASE_FILES))}; this one also changes {', '.join(extra)}."
        )
    return errors


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = sys.argv[1]

    errors, version = check_head()
    if version is not None:
        before = base_version(base)
        head_ref = os.environ.get("PR_HEAD_REF", "")
        is_release = (
            os.environ.get("PR_AUTHOR") == RELEASE_AUTHOR
            and head_ref.startswith(RELEASE_BRANCH_PREFIX)
            and os.environ.get("PR_HEAD_REPO") == os.environ.get("REPOSITORY")
        )
        if is_release:
            branch_version = head_ref.removeprefix(RELEASE_BRANCH_PREFIX)
            if version != branch_version:
                errors.append(
                    f"The release pull request for {branch_version} sets the version to {version}."
                )
            errors += check_release(base, version)
            if not errors:
                print(f"Release pull request: {before} -> {version}.")
        elif version != before:
            errors.append(
                f"This pull request changes the version from {before} to {version}. Pull requests "
                "do not change the version; the Prepare release workflow does, in the release "
                "pull request (docs/architecture/RELEASE.md)."
            )
        elif not errors:
            print(f"The version is unchanged at {version}.")

    for error in errors:
        print(f"::error::{error}")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()
