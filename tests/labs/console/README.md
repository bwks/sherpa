# Console lab test

Tests `sherpa console` end to end through the `sherpa` CLI, in a pseudo-terminal as a user's
terminal runs it. The console is relayed by sherpad over its API, and every command runs with the
directories holding a system `telnet` or `ssh` left off `PATH`: neither is needed.

| Node | Model | Role |
|---|---|---|
| `alp01` | `alpine_linux` (the default version) | VM whose serial console is opened |
| `ctr01` | `generic`, kind `container` (`alpine` 3.22) | Container, which has no serial console |

The runner writes the manifest for each run itself; there is no manifest file here.

## What it checks

1. `sherpa up`.
2. `sherpa console ctr01` fails with the message that containers have no serial console.
3. Another normal user and an admin, neither owning the lab, each in a folder holding a copy of
   the owner's `.sherpa/lab-info.toml`, are refused `sherpa console alp01` by the server with
   `Access denied: only the owner of lab '<id>' can open its consoles`.
4. `sherpa console alp01` prints its connect line; Enter is pressed until `login: ` shows, and
   typed text is echoed by the guest (the line is cleared, never submitted, so no login name or
   password is ever sent).
5. With that console open, a second `sherpa console alp01` fails, naming the owner as the
   holder and offering `--force`.
6. `sherpa console alp01 --force` takes the console over, at the login prompt; the first
   console ends with code 0, saying it was taken over by the owner.
7. `Ctrl+]` ends the console that took over with exit code 0 and one close line, starting on a
   new line.
8. A new console, at the login prompt again, ends with Enter and `~.` the same way.
9. `sherpa destroy`, and the lab is gone.

## Running it

It runs wherever the CLI is logged in as the lab's owner, with a device password set
(`sherpa server user device-password`; the run stops early without one), to a server with the
`alpine_linux` image and the `generic` container `alpine` 3.22 image; it needs no Docker access.
`sherpa` must not be in the same directory as `telnet` or `ssh`.

Two more accounts are needed, each logged in non-interactively to a `HOME` of its own (the
owner's client config and trusted server certificate are copied there), so the owner's login is
never touched:

| Input | Meaning |
|---|---|
| `--other-user NAME` or `SHERPA_LAB_TEST_OTHER_USER` | A normal user, not the owner |
| `SHERPA_LAB_TEST_OTHER_PASSWORD` | Its Sherpa password |
| `--admin-user NAME` or `SHERPA_LAB_TEST_ADMIN_USER` | An admin, not the owner |
| `SHERPA_LAB_TEST_ADMIN_PASSWORD` | Its Sherpa password |

Passwords are only read from the environment.

```
export SHERPA_LAB_TEST_OTHER_PASSWORD=... SHERPA_LAB_TEST_ADMIN_PASSWORD=...
uv run tests/labs/console/console_lab_test.py --other-user bob --admin-user dana
uv run tests/labs/console/console_lab_test.py --other-user bob --admin-user dana --keep-failed
uv run tests/labs/console/console_lab_test.py --self-test     # the runner's own unit tests
```

Logs, including each console's terminal transcript, are under
`.tmp/lab-tests/console-<run>/`. A run takes about 3–5 minutes.

Who can reach the console socket on the server host itself is checked by
`crates/libvirt/tests/console_socket.rs`, which needs root and runs in the VM release suite.
