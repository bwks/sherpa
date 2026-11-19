#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""Write an SSH config that reaches the dev lab's nodes directly, without the ProxyJump.

`sherpa up` writes `.sherpa/ssh/config` in the lab directory, which routes every node through a
ProxyJump to the Sherpa server. On the server itself that jump is a needless extra hop. This script resolves each
node from that file with `ssh -G`, so it always follows the current lab, and writes
`.sherpa/ssh/direct_config` with a direct connection and a shared, persistent master connection.
The paths in it are absolute, as in Sherpa's config, and `sherpa destroy` deletes it with
`.sherpa/`.

Run it after every `sherpa up` or `sherpa redeploy`, with the lab directory (default: this
script's directory). It writes one block per node in the lab's manifest:

    uv run direct_ssh_config.py ~/labs/sherpa-dev
    ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev01
    ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev02
"""

import os
from pathlib import Path
import subprocess
import sys
import tomllib

HERE = Path(__file__).resolve().parent
# Unix socket paths are limited to about 108 bytes, so the master socket cannot live here.
SOCKET_DIR = Path(os.environ.get("XDG_RUNTIME_DIR", Path.home() / ".ssh"))
# Settings copied from Sherpa's resolved config; the rest are set below. IdentitiesOnly is copied,
# not forced: a node that logs in as the lab owner has no IdentityFile in Sherpa's config and
# uses the owner's SSH agent or default keys. A node that logs in as `sherpa` has the lab key
# with IdentitiesOnly yes.
COPIED = ("hostname", "port", "user", "identitiesonly", "userknownhostsfile",
          "stricthostkeychecking", "pubkeyacceptedalgorithms", "hostkeyalgorithms",
          "kexalgorithms")
# Copied only when Sherpa's config names a key for the node: otherwise `ssh -G` lists ssh's
# default key paths, and ssh warns about each one that does not exist.
IDENTITY = "identityfile"
DIRECT = {
    "ProxyJump": "none",
    "ControlMaster": "auto",
    "ControlPath": str(SOCKET_DIR / "sherpa-ssh-%C"),
    "ControlPersist": "30m",
    "ServerAliveInterval": "15",
    "ServerAliveCountMax": "3",
}


def resolve(sherpa_config, alias):
    """The settings `ssh` uses for this alias in Sherpa's config, as (key, value) pairs."""
    output = subprocess.run(["ssh", "-F", str(sherpa_config), "-G", alias],
                            check=True, capture_output=True, text=True).stdout
    return [tuple(line.split(" ", 1)) for line in output.splitlines() if " " in line]


def names_identity(sherpa_config, alias):
    """Whether the Host block for this alias in Sherpa's config sets an IdentityFile."""
    in_block = False
    for line in sherpa_config.read_text().splitlines():
        words = line.split()
        if not words:
            continue
        keyword = words[0].lower()
        if keyword in ("host", "match"):
            in_block = keyword == "host" and alias in words[1:]
        elif in_block and keyword == IDENTITY:
            return True
    return False


def node_block(sherpa_config, name, lab_id):
    alias = f"{name}.{lab_id}"
    settings = resolve(sherpa_config, alias)
    copied = COPIED + ((IDENTITY,) if names_identity(sherpa_config, alias) else ())
    lines = [f"Host {name}"]
    lines += [f"    {key} {value}" for key, value in settings if key in copied]
    lines += [f"    {key} {value}" for key, value in DIRECT.items()]
    return "\n".join(lines)


def main():
    if len(sys.argv) > 2:
        sys.exit("Usage: direct_ssh_config.py [lab dir]")
    lab = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else HERE
    sherpa_config = lab / ".sherpa" / "ssh" / "config"
    direct_config = lab / ".sherpa" / "ssh" / "direct_config"
    if not sherpa_config.is_file():
        sys.exit(f"{sherpa_config} not found; run `sherpa up` in {lab} first")
    lab_id = tomllib.loads((lab / ".sherpa" / "lab-info.toml").read_text())["id"]
    nodes = tomllib.loads((lab / "manifest.toml").read_text())["nodes"]
    blocks = [node_block(sherpa_config, node["name"], lab_id) for node in nodes]
    direct_config.write_text("# Written by direct_ssh_config.py; do not edit.\n\n"
                             + "\n\n".join(blocks) + "\n")
    direct_config.chmod(0o600)
    print(f"Wrote {direct_config}")


if __name__ == "__main__":
    main()
