# Owner login lab test

Tests who can reach a lab's nodes, end to end through the `sherpa` CLI. One lab, started by the
owner, has a Linux VM and a network OS container. Two Sherpa accounts are involved: the owner, a
normal user, and an admin who does not own the lab.

| Node | Model | Role |
|---|---|---|
| `ubu01` | `ubuntu_linux` 24.04 VM | The owner's account has `sudo` |
| `srl01` | `nokia_srlinux` 26.7.2 container | The owner's account has the `srl-admin` role (a superuser) |

SR Linux needs no internet. The runner adds a unique suffix to the lab name for each run.

## Requirements

- A Sherpa server with the `ubuntu_linux` 24.04 and `nokia_srlinux` 26.7.2 images. The run fails
  early if either is missing.
- The `sherpa` CLI logged in as the owner: a normal user (not an admin, not the built-in
  `admin`) with an SSH key registered on their account and available to `ssh`, and a device
  password (`sherpa server user device-password`). `ssh` and `ssh-keygen` on `PATH`.
- An account on the Sherpa server's host for the owner, of the same name, taking the owner's key:
  the lab's SSH config jumps through the server as that user. The admin's direct `ssh` is refused
  at that hop when they have no host account, or at the node when they do; either counts.
- A second account that is an admin, is not the built-in `admin` and has a device password. The
  run logs it in non-interactively, to a `HOME` of its own: the owner's client config and trusted
  server certificate are copied there, and the admin's token is kept there, so the owner's login
  is never touched.
- The three inputs below. Passwords are only read from the environment, never from options, and
  are masked in every log.

| Input | Meaning |
|---|---|
| `--admin-user NAME` or `SHERPA_LAB_TEST_ADMIN_USER` | The admin account's name |
| `SHERPA_LAB_TEST_ADMIN_PASSWORD` | The admin account's Sherpa password, for `sherpa login` |
| `SHERPA_LAB_TEST_DEVICE_PASSWORD` | The owner's device password, typed at the nodes' password prompts. It is the one set with `sherpa server user device-password`; the server stores it sealed, so the run cannot check it first |

The preflight stops early, with a message, when the CLI is not logged in, when either account has
no device password, when the owner is an admin or is the built-in `admin`, when the admin is not
an admin, is the built-in `admin` or is the owner, when the admin cannot log in, when an image is
missing or when `ssh` or `ssh-keygen` is missing.

## Running

```sh
export SHERPA_LAB_TEST_ADMIN_PASSWORD=...
export SHERPA_LAB_TEST_DEVICE_PASSWORD=...
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user dana
uv run tests/labs/owner-login/owner_login_lab_test.py --admin-user dana --keep-failed
uv run tests/labs/owner-login/owner_login_lab_test.py --self-test  # runner unit tests, seconds
```

Each run logs every command and its output under `.tmp/lab-tests/owner-login-<id>/`:
`lab/commands.log` for the lab and `preflight.log`. It also keeps the owner's lab folder (`lab/`),
the admin's (`admin/`, holding only a copy of the owner's `.sherpa/lab-info.toml`), the admin's
`HOME` (`admin-home/`) and the admin's throwaway SSH key (`admin-key/`). A run takes about 5-10
minutes. Run on the Sherpa server, it also checks no libvirt domain, container, Docker network or
host interface of the lab is left; elsewhere it skips those checks.

## What it checks

As the owner, on each node:

- `sherpa ssh <node>` logs in by key, in a pseudo-terminal, and the session shows the owner's own
  name;
- `ssh` logs in with the owner's device password and no key offered (the lab's SSH config, with
  public key authentication turned off for the node only: the jump through the Sherpa server still
  uses the key). The runner types the password at the node's prompt itself, so there is no
  `sshpass`; a prompt of the Sherpa server fails the check;
- the owner has administrator rights, by key and by password: on the VM `sudo -n true` succeeds;
  on SR Linux the owner's account state (`info from state /system aaa authentication user
  <owner>`) shows the login just made (`last-successful-login`) and the `srl-admin` role, the
  superuser role the generated config gives the owner (SR Linux 26.7 lists sessions only by id);
- the old shared `sherpa` user is refused, with the owner's key and with its old published
  password;
- the lab folder has no `.sherpa/ssh/key` and no private key anywhere under `.sherpa/`;
- the owner's `sherpa inspect` shows a VNC port for the VM, so the admin's not showing one means
  something.

As the admin, working in a folder that holds a copy of the owner's `.sherpa/lab-info.toml`, which
is how an admin names a lab they do not own:

- `sherpa inspect` lists the nodes and shows no VNC port for any (the CLI's table does not show
  the serial console address, so the console address is not checked here);
- `sherpa download <lab id>`, in an empty folder, is refused with `Only the owner of lab '<id>' can
  perform this operation`, and writes nothing;
- `sherpa ssh` on each node and `sherpa console` on the VM are refused with `Only the owner of lab
  '<id>' can connect to its nodes`, before anything connects;
- `ssh` straight to each node's management address through the Sherpa server (the server hop and
  addresses come from the owner's lab SSH config), with only the admin's own throwaway key, is
  refused (`Permission denied`) as the admin's username and as the owner's username;
- adding a key to the owner's account with `POST /api/v1/admin/users/<owner>/ssh-keys` and the
  admin's token (Python's `urllib`, checking the server's certificate as the client config says)
  gets HTTP 403 with `only user '<owner>' can do this for their own account`, and the owner's
  `sherpa server user ssh-key list` is unchanged;
- `sherpa destroy --yes` succeeds, and the lab is gone. The owner's cleanup then removes the
  lab's stale Include from `~/.ssh/sherpa_lab_hosts` with `sherpa ssh-config clean`.

If a check fails, the owner's cleanup still destroys the lab, unless `--keep-failed` is given.
