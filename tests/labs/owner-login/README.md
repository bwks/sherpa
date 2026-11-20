# Owner login lab test

Tests who can reach a lab's nodes and what an admin may do to a lab they do not own, end to end
through the `sherpa` CLI, and takes the lab down, resumes it and redeploys a node with the owner's
keys. One lab, started by a throwaway owner, has a Linux VM and a network OS container.

| Node | Model | Role |
|---|---|---|
| `ubu01` | `ubuntu_linux` 24.04 VM | The owner's account has a home, bash and passwordless `sudo` |
| `srl01` | `nokia_srlinux` 26.7.2 container | The owner's account has the `srl-admin` role (a superuser) |

SR Linux needs no internet. The runner adds a unique suffix to the lab name for each run.

## Accounts

Two Sherpa accounts are involved, neither of them the CLI's own login, which the run never uses or
changes:

- **The admin**, named by the inputs below. It is logged in non-interactively to a `HOME` of its
  own, holding a copy of the runner's client config and trusted server certificate, and creates
  and deletes the owner.
- **The owner**, a throwaway normal user the run creates with a unique name (`owner<8 hex>`), a
  login password and a device password generated for the run, and an SSH key pair generated for
  the run; a case rotates the key to a second one. The owner's CLI works with a `HOME` of its own
  too, with its key as `~/.ssh/id_ed25519` and no SSH agent. The owner needs no account on the
  server's host: the lab's SSH config reaches the nodes directly, with no jump.

The passwords live only in the runner's memory and in the environment of the child that reads
one (`sherpa login`), or are typed at the prompts of `sherpa server user create` and
`sherpa server user device-password` in a pseudo-terminal. They are never written to a file and
are masked in every log and failure message. The `HOME`s and keys are in a temporary folder that is
removed at the end.

The `session` fixture creates the owner and deletes it at the end; the `lab` fixture adds the
owner's first key, brings the lab up and destroys it at the end. Both happen even when a case
fails, unless `--keep-failed` is given and something failed.

## Requirements

- A Sherpa server with the `ubuntu_linux` 24.04 and `nokia_srlinux` 26.7.2 images. The run fails
  early if either is missing.
- A route from where it runs to the lab's management network: run it on the Sherpa server, or on a
  machine on the lab's tailnet that accepts its routes.
- `ssh`, `ssh-keygen` and `ssh-keyscan` on `PATH`, and the runner's `~/.sherpa/config` pointing at
  the server.
- An admin account that is not the built-in `admin`, given by the inputs below. Its password is
  only read from the environment, never from an option.

| Input | Meaning |
|---|---|
| `--admin-user NAME` or `SHERPA_LAB_TEST_ADMIN_USER` | The admin account's name |
| `SHERPA_LAB_TEST_ADMIN_PASSWORD` | The admin account's Sherpa password, for `sherpa login` |

## Running

```sh
export SHERPA_LAB_TEST_ADMIN_PASSWORD=...
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user dana
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user dana --keep-failed
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user dana -k owner_login_
uv run tests/labs/owner-login/owner_login_lab_test.py --collect-only -q
uv run tests/labs/owner-login/owner_login_lab_test.py --self-test   # runner unit tests, seconds
```

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header and
runs pytest on it, passing on any other arguments. pytest runs it directly too, for example
`uvx --python 3.15 pytest==9.2.0 tests/labs/owner-login/owner_login_lab_test.py --admin-user dana`.
`--admin-user` and `--keep-failed` are defined in `conftest.py` next to the runner.

Each run logs every command and its output, passwords and tokens masked, to
`.tmp/lab-tests/owner-login-<id>/commands.log`, next to the owner's lab folder (`lab/`) and the
admin's (`admin/`, holding only a copy of the owner's `.sherpa/lab-info.toml`, which is how an
admin names a lab they do not own). A run takes about 5 minutes. Run on the Sherpa server, it
also checks no libvirt domain, container, Docker network or host interface of the lab is left;
elsewhere it skips those checks.

## Cases

The cases are the test functions, in this order; every case runs even when one fails. A case that
changes the lab or the account leaves them as the next case needs them, and the admin's destroy
comes last.

| Case | What it checks |
|---|---|
| `owner_login_001` | Before the lab exists, `sherpa up` by the owner with no SSH key is refused with `has no SSH public key registered`, and writes no `.sherpa/lab-info.toml` |
| `owner_login_002` | `sherpa ssh ubu01`, in a pseudo-terminal, logs in by key; the session shows the owner's name, `/home/<owner>` owned by them, `bash` as the login shell and `sudo -n true` succeeding |
| `owner_login_003` | `sherpa ssh srl01` logs in by key; `info from state /system aaa authentication user <owner>` shows the login just made and the `srl-admin` role |
| `owner_login_004` | `ssh` with no key offered logs in to each node with the owner's device password, typed at the node's own prompt (a prompt from any other host fails), with the same account and rights |
| `owner_login_005` | `ssh -v` straight to each node's management address with only a key not on the owner's account (the admin's own throwaway key), as the admin's name and as the owner's, connects to the node itself and is refused there (`Permission denied (publickey`) |
| `access_control_004` | The admin's `sherpa inspect` lists both nodes |
| `access_control_005` | The admin's `sherpa download <lab id>` in an empty folder is refused with `Only the owner of lab '<id>' can perform this operation`, and writes nothing |
| `access_control_006` | The admin's `sherpa ssh` on each node is refused with `Only the owner of lab '<id>' can connect to its nodes`, before anything connects |
| `access_control_007` | The admin's `sherpa console ubu01` is refused by the server with `only the owner of lab '<id>' can open its consoles` |
| `access_control_008` | `POST /api/v1/admin/users/<owner>/ssh-keys` with the admin's token gets HTTP 403 with `only user '<owner>' can do this for their own account`, and the owner's keys are unchanged |
| `lab_down_resume_002` | `sherpa down`, then `sherpa resume` streams `Node ubu01 - Ready` before its result `ubu01: Started and ready`, and the owner's first login right after it returns succeeds |
| `lab_down_resume_003` | Down and resume again: `ubu01`'s host keys (`ssh-keyscan`) are unchanged and the owner logs in with the lab's SSH config, which checks them strictly |
| `owner_login_006` | A new key is added to the owner's account and the old one removed; after down and resume both nodes still take the key they were built with and refuse the new one |
| `owner_login_007` | With every key removed from the owner's account, `sherpa redeploy --node ubu01` is refused with `has no SSH public key registered`; the keys are put back |
| `lab_redeploy_001` | `sherpa redeploy --node ubu01` rebuilds the VM (its boot id changes) and the owner logs in with the key now on their account |
| `lab_redeploy_002` | A redeploy refused for want of a key leaves `ubu01` running untouched: its boot id is unchanged; the keys are put back |
| `access_control_009` | The admin's `sherpa destroy --yes` destroys the lab. The owner's cleanup then removes the lab's stale Include from their `~/.ssh/sherpa_lab_hosts` with `sherpa ssh-config clean` |

The specs are in [`tests/specs/accounts-and-auth.md`](../../specs/accounts-and-auth.md) and
[`tests/specs/lab-lifecycle.md`](../../specs/lab-lifecycle.md).
