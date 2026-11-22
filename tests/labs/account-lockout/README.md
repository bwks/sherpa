# Account lockout test

Tests the account lockout through the `sherpa` CLI: failed logins in a row lock a throwaway
account, and `sherpa server user unlock` unlocks it; on the server host, root unlocks a locked
account and `admin` with `sherpad user unlock` while the server is stopped. It brings no lab up,
so it needs no node image and runs in seconds.

## Requirements

- The `sherpa` CLI on `PATH`, logged in as an admin. The runner creates a throwaway user,
  `lockout<id>`, with a password generated for the run, typed at the CLI's prompts and never
  printed, and deletes it at the end, even when the case fails. The user logs in from a HOME of
  its own under the run's folder, so the admin's token is never touched. Each case has a user of
  its own.
- For `user_login_033` only, given `--stop-server`: the server host, with `sherpad` on `PATH` and
  passwordless `sudo`. The case stops and starts sherpad twice and locks the built-in `admin` with
  wrong passwords, so run it only where stopping the server is fine. It reads `admin`'s state from
  the admin users page, with the CLI's admin token, so it needs no password of `admin`, and it
  leaves `admin` unlocked, even when it fails. Without the flag the case is skipped.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/account-lockout/account_lockout_lab_test.py
uv run tests/labs/account-lockout/account_lockout_lab_test.py --stop-server  # 033 too, on the server host
uv run tests/labs/account-lockout/account_lockout_lab_test.py --self-test  # runner unit tests, seconds
```

The `account` fixture makes the case's folder, `.tmp/lab-tests/account-lockout-<id>/`, creates
the user and deletes it at the end; the folder keeps `commands.log`, which records every command
and its output with the password and token masked.

## What it checks

It holds the e2e cases of `user_login` in
[`tests/specs/accounts-and-auth.md`](../../specs/accounts-and-auth.md):

| Case | What it checks |
|---|---|
| `user_login_024` | Three wrong passwords lock the user, the right one is then refused with "username and/or password is incorrect", `sherpa server user unlock` unlocks it, and the user logs in again |
| `user_login_033` | The user and `admin` are locked with three wrong passwords each. `sudo -u sherpa sherpad user unlock` is refused while sherpad runs; with it stopped, the same as root is refused and an unknown username fails, and after a restart both are still locked. With sherpad stopped again, it unlocks the user and `admin`, printing `Unlocked <name>`; after the restart the user logs in and the admin users page no longer offers to unlock `admin` |

The other `user_login` cases are integration tests in `crates/server` and `crates/db`.
