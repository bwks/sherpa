# SSH access lab test

Tests how `sherpa ssh` and SSH provisioners reach a lab's nodes, end to end through the `sherpa`
CLI: directly by default, and through the Sherpa server with `--ssh-jump`. It holds the e2e cases
of the `ssh_access` behaviour in [`tests/specs/client-lab-files.md`](../../specs/client-lab-files.md).

| Node | Model | Role |
|---|---|---|
| `alp01` | `alpine_linux` (the default version) | VM with an SSH provisioner whose script prints the connection the node sees (`$SSH_CONNECTION`) and holds it for 5 s |

The runner writes the manifest and the script for each run itself, under a unique lab name;
there is no manifest file here.

## What it checks

Each command's connections to SSH ports on this machine are read with `ss` while it runs. The
connection the node reports is matched by its client address and port:

| Case | Check |
|---|---|
| `ssh_access_001` | `.sherpa/ssh/config` has no `ProxyJump` or `ProxyCommand`, and the node's block names its own address |
| `ssh_access_002` | `sherpa ssh alp01`, in a pseudo-terminal, holds the connection the node reports itself, and exits with the shell's code |
| `ssh_access_003` | `sherpa ssh alp01 --ssh-jump` holds only a connection to the server's SSH port; the connection to the node belongs to another process (the server's `sshd` for the user) |
| `ssh_access_004` | `sherpa provision` holds the connection the node reports itself |
| `ssh_access_005` | `sherpa provision --ssh-jump` connects to the node only through the server, as for `ssh_access_003` |

The other cases of the behaviour are Rust tests in the client crate.

## Running it

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/ssh-access/ssh_access_lab_test.py
uv run tests/labs/ssh-access/ssh_access_lab_test.py --keep-failed
uv run tests/labs/ssh-access/ssh_access_lab_test.py -k ssh_access_003
uv run tests/labs/ssh-access/ssh_access_lab_test.py --self-test   # runner unit tests, seconds
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/ssh-access/ssh_access_lab_test.py`. `--keep-failed` is defined in `conftest.py` next to
the runner.

It needs:

- a machine with a route to the lab's management network, such as the Sherpa server itself, and
  `ss` (iproute2);
- the `sherpa` CLI logged in, with a device password (`sherpa server user device-password`);
- a server with the `alpine_linux` image;
- for the `--ssh-jump` cases, an account on the server's host named as the Sherpa user, that takes
  the user's SSH key, and the server's host key in `~/.ssh/known_hosts` (or a terminal to accept it
  on): the jump is `<user>@<server>`, as `ssh -J` takes it.

The module-scoped `lab` fixture checks those, brings the lab up once and destroys it at the end,
even when a case fails, unless `--keep-failed` is given and something failed. The cases are the
functions named `test_ssh_access_<NNN>_*` that take it, in case order; every case runs even when
one fails. The runner's own unit tests, under `--self-test`, are not collected by pytest.

Each run logs every command and its output under `.tmp/lab-tests/ssh-access-<id>/`. A run takes
about 2 minutes.
