# Account lockout test

Tests the account lockout through the `sherpa` CLI: failed logins in a row lock a throwaway
account, and `sherpa server user unlock` unlocks it. It brings no lab up, so it needs no node
image and runs in seconds.

## Requirements

- The `sherpa` CLI on `PATH`, logged in as an admin. The runner creates a throwaway user,
  `lockout<id>`, with a password generated for the run, typed at the CLI's prompts and never
  printed, and deletes it at the end, even when the case fails. The user logs in from a HOME of
  its own under the run's folder, so the admin's token is never touched.

## Running

The runner is a pytest module; `uv run` installs the pytest version pinned in its script header
and runs pytest on it, passing on any other arguments:

```sh
uv run tests/labs/account-lockout/account_lockout_lab_test.py
uv run tests/labs/account-lockout/account_lockout_lab_test.py --self-test  # runner unit tests, seconds
```

The module-scoped `account` fixture makes the run's folder,
`.tmp/lab-tests/account-lockout-<id>/`, creates the user and deletes it at the end; the folder
keeps `commands.log`, which records every command and its output with the password masked.

## What it checks

It holds the CLI case of `user_login` in
[`tests/specs/accounts-and-auth.md`](../../specs/accounts-and-auth.md):

| Case | What it checks |
|---|---|
| `user_login_024` | Three wrong passwords lock the user, the right one is then refused with "username and/or password is incorrect", `sherpa server user unlock` unlocks it, and the user logs in again |

The other `user_login` cases are integration tests in `crates/server` and `crates/db`.
