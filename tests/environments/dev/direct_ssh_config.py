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

Run it after every `sherpa up`, with the lab directory (default: this script's directory):

    uv run direct_ssh_config.py ~/labs/sherpa-dev
    ssh -F ~/labs/sherpa-dev/.sherpa/ssh/direct_config dev
"""

import os
from pathlib import Path
import subprocess
import sys
import tomllib

HERE = Path(__file__).resolve().parent
# Unix socket paths are limited to about 108 bytes, so the master socket cannot live here.
SOCKET_DIR = Path(os.environ.get("XDG_RUNTIME_DIR", Path.home() / ".ssh"))
# Settings copied from Sherpa's resolved config; the rest are set below.
COPIED = ("hostname", "port", "user", "identityfile", "userknownhostsfile",
          "stricthostkeychecking", "pubkeyacceptedalgorithms", "hostkeyalgorithms",
          "kexalgorithms")
DIRECT = {
    "IdentitiesOnly": "yes",
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


def node_block(sherpa_config, name, lab_id):
    settings = resolve(sherpa_config, f"{name}.{lab_id}")
    lines = [f"Host {name}"]
    lines += [f"    {key} {value}" for key, value in settings if key in COPIED]
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
