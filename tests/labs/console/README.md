# Console lab test

Tests `sherpa console` end to end through the `sherpa` CLI, in a pseudo-terminal as a user's
terminal runs it, and `sherpa vnc` with a scripted VNC viewer. Both consoles are relayed by
sherpad over its API, and every command runs with the directories holding a system `telnet` or
`ssh` left off `PATH`: neither is needed.

| Node | Model | Role |
|---|---|---|
| `alp01` | `alpine_linux` (the default version) | VM whose serial and VNC consoles are opened |
| `ctr01` | `generic`, kind `container` (`alpine` 3.22) | Container, which has neither console |

The runner writes the manifest for each run itself; there is no manifest file here.

## Requirements

- The `sherpa` CLI logged in as the lab's owner, with a device password set
  (`sherpa server user device-password`; the run stops early without one). It needs no Docker
  access.
- The `alpine_linux` image and the `generic` container `alpine` 3.22 image. On a server without
  the container image, `--no-container` leaves `ctr01` out and skips its two cases.
- `sherpa` not in the same directory as `telnet` or `ssh`.
- An admin who is not the owner, such as the built-in `sherpa-admin`. It is logged in
  non-interactively to a `HOME` of its own (the owner's client config and trusted server
  certificate are copied there), so the owner's login is never touched:

| Input | Meaning |
|---|---|
| `--admin-user NAME` or `SHERPA_LAB_TEST_ADMIN_USER` | An admin, not the owner |
| `SHERPA_LAB_TEST_ADMIN_PASSWORD` | Its Sherpa password, only read from the environment |

- An SSH key on the owner's account, as `sherpa up` needs.

The other user the cases need, a normal user who does not own the lab, is made by the run, so no
account is made by hand. The admin creates it with `sherpa server user create`, under a unique
name (`console` and 8 hex digits) and with a password generated with Python's `secrets`; the
runner types the password at both prompts in a pseudo-terminal, as a user does. The user is then
logged in to a `HOME` of its own with `sherpa login`, the password passed in
`SHERPA_USER_PASSWORD`, and adds the owner's public key with `sherpa server user ssh-key add`.
The password is only held in memory: never written to a file or put on a command line, and
masked in the logs, as are the admin's password and both tokens. At the end, even when a case
fails, the admin deletes the user with `sherpa server user delete --force` and checks
`sherpa server user list` no longer shows it.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
export SHERPA_LAB_TEST_ADMIN_PASSWORD=...
uv run tests/labs/console/console_lab_test.py --admin-user admin
uv run tests/labs/console/console_lab_test.py --admin-user admin --keep-failed
uv run tests/labs/console/console_lab_test.py --admin-user admin --no-container
uv run tests/labs/console/console_lab_test.py --admin-user admin -k vnc_console_
uv run tests/labs/console/console_lab_test.py --self-test     # the runner's own unit tests
```

pytest runs it directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/console/console_lab_test.py --admin-user admin`. The options are
defined in `conftest.py` next to the runner.

The module-scoped `prepared` fixture checks the requirements, logs the admin in, creates the
other user and writes the run's manifest under a unique lab name, and deletes the other user at
the end; the `lab` fixture brings the lab up once and
destroys it at the end, even when a case fails, unless `--keep-failed` is given and something
failed. Each case starts from a console of its own, given by a function fixture that ends it
after the case:

- `at_login`: the owner's `sherpa console alp01` at the login prompt. Enter is pressed until
  `login: ` shows, and typed text is echoed by the guest; the line is cleared, never submitted,
  so no login name or password is ever sent. After the case it is left with `Ctrl+]` if still
  open.
- `vnc`: the owner's `sherpa vnc alp01`, listening. After the case it is stopped with Ctrl+C if
  still running.
- `held_vnc`: `vnc` with a viewer connected that got the banner, so the VNC console is held.

The cases are the functions named `test_<case_id>_*`, in the order below; every case runs even
when one fails. The runner's own unit tests, under `--self-test`, are not collected by pytest.

Logs, including each console's terminal transcript, are under
`.tmp/lab-tests/console-<run>/lab/commands.log`; the accounts' commands, the other user's
creation and deletion included, are in `.tmp/lab-tests/console-<run>/accounts.log`. A run takes about 1–5 minutes.

## What it checks

It holds the e2e cases of [`tests/specs/consoles.md`](../../specs/consoles.md) that need a lab:

| Case | Check |
|---|---|
| `serial_console_001` | The owner's console, with no `telnet` or `ssh` on `PATH`, prints the connect line naming `alp01`, the lab and both escapes, shows the login prompt and echoes typed text |
| `serial_console_002` | `Ctrl+]` ends the console with exit code 0 and one close line, starting on a new line |
| `serial_console_003` | Enter then `~.` ends it the same way |
| `vnc_console_001` | `sherpa vnc alp01` prints its local address on `127.0.0.1`, and a viewer (a plain TCP connection) gets the server's `RFB 003.008` banner |
| `vnc_console_002` | Ctrl+C (`SIGINT`) ends `sherpa vnc` with code 0 and its stop line |
| `vnc_console_005` | With a viewer holding the console, a second `sherpa vnc alp01` prints its address, says the VNC console is in use by the owner and offers `--force`, and keeps running |
| `vnc_console_006` | With a viewer holding the console, a second viewer of the same command gets nothing, and the command says the console is in use by the owner and keeps running |
| `vnc_console_007` | Once a viewer leaves, the command says it disconnected, and a new viewer gets the banner |
| `console_access_001` | The other user, in a folder holding a copy of the owner's `.sherpa/lab-info.toml`, is refused `sherpa console alp01` by the server with `Lab not found: <id>`, the answer for a lab that does not exist |
| `console_access_002` | The other user and the admin each run `sherpa vnc alp01` in their folder: it ends with exit code 1 and no local address, the user told `Lab not found: <id>` and the admin `Access denied: only the owner of lab '<id>' can open its consoles` |
| `console_session_001` | With the console open, a second `sherpa console alp01` fails, naming the owner as the holder and offering `--force` |
| `console_session_002` | `sherpa console alp01 --force` takes the open console over, at the login prompt; the console taken over ends with code 0, saying it was taken over by the owner |
| `console_refusal_001` | `sherpa console ctr01` fails, saying containers have no serial console and to use `sherpa ssh` |
| `console_refusal_002` | `sherpa vnc ctr01` ends with exit code 1, saying containers have no VNC console, and prints no local address: it asks the server before it opens a port, so no viewer connects |

An admin refused `sherpa console` is `access_control_007`, shown by
[`owner-login`](../owner-login/README.md), so this lab checks only the other user's console
refusal.

The fixture's teardown checks that `sherpa destroy` succeeds and `sherpa inspect` no longer finds
the lab.

Who can reach the serial and VNC sockets on the server host itself, and that nothing listens on
the old console addresses or the VNC ports 5900 to 6999, is checked by
`crates/libvirt/tests/console_socket.rs`, which needs root and runs in the VM release suite.
