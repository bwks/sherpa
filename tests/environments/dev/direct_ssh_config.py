#!/usr/bin/env python3
"""Write an SSH config that reaches the dev lab's nodes directly, without the ProxyJump.

`sherpa up` writes `sherpa_ssh_config` here, which routes every node through a ProxyJump to the
Sherpa server. On the server itself that jump is a needless extra hop. This script resolves each
node from that file with `ssh -G`, so it always follows the current lab, and writes
`sherpa_ssh_direct_config` with a direct connection and a shared, persistent master connection.

Run it after every `sherpa up`, then: ssh -F tests/environments/dev/sherpa_ssh_direct_config dev
"""

import os
from pathlib import Path
import subprocess
import sys
import tomllib

HERE = Path(__file__).resolve().parent
SHERPA_CONFIG = HERE / "sherpa_ssh_config"
DIRECT_CONFIG = HERE / "sherpa_ssh_direct_config"
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


def resolve(alias):
    """The settings `ssh` uses for this alias in Sherpa's config, as (key, value) pairs."""
    output = subprocess.run(["ssh", "-F", str(SHERPA_CONFIG), "-G", alias],
                            check=True, capture_output=True, text=True).stdout
    return [tuple(line.split(" ", 1)) for line in output.splitlines() if " " in line]


def node_block(name, lab_id):
    settings = resolve(f"{name}.{lab_id}")
    lines = [f"Host {name}"]
    lines += [f"    {key} {value}" for key, value in settings if key in COPIED]
    lines += [f"    {key} {value}" for key, value in DIRECT.items()]
    return "\n".join(lines)


def main():
    if not SHERPA_CONFIG.is_file():
        sys.exit(f"{SHERPA_CONFIG.name} not found; run `sherpa up` in {HERE} first")
    lab_id = tomllib.loads((HERE / "lab-info.toml").read_text())["id"]
    nodes = tomllib.loads((HERE / "manifest.toml").read_text())["nodes"]
    blocks = [node_block(node["name"], lab_id) for node in nodes]
    DIRECT_CONFIG.write_text("# Written by direct_ssh_config.py; do not edit.\n\n"
                             + "\n\n".join(blocks) + "\n")
    DIRECT_CONFIG.chmod(0o600)
    print(f"Wrote {DIRECT_CONFIG}")


if __name__ == "__main__":
    main()
