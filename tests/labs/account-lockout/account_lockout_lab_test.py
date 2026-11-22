#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of the account lockout through the sherpa CLI (Python 3.15+, pytest).

Checks the CLI case of `user_login` in tests/specs/accounts-and-auth.md: failed logins in a row
lock a throwaway account, which `sherpa server user unlock` unlocks. Brings no lab up. Runs
wherever the `sherpa` CLI is logged in as an admin; see README.md.

A pytest module: the `account` fixture creates the run's throwaway user as the admin, in a HOME
of its own, and deletes it at the end, even when a case fails. Run as a script, it runs pytest on
itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import signal
import sys
import tempfile
import unittest
from unittest import mock
import uuid

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    INFO_ADMIN,
    Terminal,
    TestFailure,
    answer_prompts,
    generate_password,
    require,
    run_logged,
    stop_on_sigterm,
)


LAB = Path(__file__).resolve().parent
ROOT = LAB.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"

USER_PREFIX = "lockout"
CREATE_PROMPTS = ("Password for {username}: ", "Confirm password: ")
LOGIN_USER_ENV = "SHERPA_USER_NAME"
LOGIN_PASSWORD_ENV = "SHERPA_USER_PASSWORD"
# Failed logins in a row that lock an account, and the server's answer to every refused login.
LIMIT = 3
REFUSED = "username and/or password is incorrect"


class Account:
    """The run's throwaway user: its name, its password and a HOME of its own with no token."""

    def __init__(self, root, log):
        self.root = root
        self.log = log
        self.username = f"{USER_PREFIX}{uuid.uuid4().hex[:8]}"
        self.password = generate_password(self.username)
        home = root / "user-home"
        (home / ".sherpa").mkdir(parents=True, mode=0o700)
        source = Path.home() / ".sherpa"
        for part in ("config", "trusted_certs"):
            if (source / part).is_dir():
                shutil.copytree(source / part, home / ".sherpa" / part)
        self.env = {key: value for key, value in os.environ.items()
                    if key not in (LOGIN_USER_ENV, LOGIN_PASSWORD_ENV)}
        self.env["HOME"] = str(home)

    def login(self, password, label):
        """`sherpa login` as the user with `password`, in the user's HOME."""
        return run_logged(self.log, label, ["sherpa", "login"], self.root,
                          env={**self.env, LOGIN_USER_ENV: self.username, LOGIN_PASSWORD_ENV: password})


def admin(log, root, label, arguments):
    """A `sherpa` command as the CLI's own login, the admin."""
    return run_logged(log, label, ["sherpa", *arguments], root)


# -- the fixtures --------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def account():
    """The run's throwaway user, created by the admin the CLI is logged in as, and deleted at the
    end. The run's folder under `.tmp/lab-tests/` keeps the command log."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    require(shutil.which("sherpa"), "sherpa is not on PATH")
    root = RUNS / f"account-lockout-{uuid.uuid4().hex[:12]}"
    root.mkdir(parents=True)
    print(f"Logs: {root}", flush=True)
    log = (root / "commands.log").open("w")
    created = False
    try:
        info = admin(log, root, "sherpa server user info", ["server", "user", "info"])
        found = INFO_ADMIN.search(info.stdout)
        require(info.code == 0 and found and found.group(1) == "Yes",
                "the sherpa CLI must be logged in as an admin (run `sherpa login`)")
        user = Account(root, log)
        label = f"sherpa server user create {user.username}"
        terminal = Terminal(log, label, ["sherpa", "server", "user", "create", user.username], root, None)
        created = True
        prompts = [prompt.format(username=user.username) for prompt in CREATE_PROMPTS]
        code, text = answer_prompts(terminal, [(prompt, user.password) for prompt in prompts])
        require(code == 0, f"{label} failed (exit {code}): {text.strip()[-600:]}")
        yield user
    finally:
        try:
            if created:
                delete = admin(log, root, "sherpa server user delete",
                               ["server", "user", "delete", user.username, "--force"])
                if delete.code != 0:
                    raise TestFailure(f"could not delete {user.username}; see {log.name}")
        finally:
            log.close()
            shutil.rmtree(root / "user-home", ignore_errors=True)


# -- cases ---------------------------------------------------------------------------------------


def test_user_login_024_an_admin_unlocks_an_account_with_sherpa_server_user_unlock(account):
    for attempt in range(1, LIMIT + 1):
        failed = account.login(f"{account.password}x", f"sherpa login, wrong password {attempt}")
        require(failed.code != 0, f"a wrong password logged in; see {account.log.name}")
    locked = account.login(account.password, "sherpa login, locked")
    require(locked.code != 0 and REFUSED in locked.output,
            f"a locked account was not refused with {REFUSED!r}: {locked.output.strip()}")

    unlock = admin(account.log, account.root, "sherpa server user unlock",
                   ["server", "user", "unlock", account.username])
    require(unlock.code == 0 and re.search(rf"'{account.username}' unlocked", unlock.output),
            f"sherpa server user unlock failed (exit {unlock.code}): {unlock.output.strip()}")

    unlocked = account.login(account.password, "sherpa login, unlocked")
    require(unlocked.code == 0, f"the unlocked account could not log in: {unlocked.output.strip()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    args, pytest_args = parser.parse_known_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    return pytest.main(["-v", "-p", "no:cacheprovider", __file__, *pytest_args])


class SelfTest(unittest.TestCase):
    # The runner's own checks, run with --self-test; pytest must not collect them as cases.
    __test__ = False

    def test_the_user_gets_a_home_of_its_own_with_no_token_and_no_login_variables(self):
        with tempfile.TemporaryDirectory() as root, \
                mock.patch.dict(os.environ, {LOGIN_USER_ENV: "dev", LOGIN_PASSWORD_ENV: "secret"}):
            user = Account(Path(root), None)
            home = Path(user.env["HOME"])
            self.assertEqual(home, Path(root) / "user-home")
            self.assertFalse((home / ".sherpa" / "token").exists())
            self.assertNotIn(LOGIN_USER_ENV, user.env)
            self.assertNotIn(LOGIN_PASSWORD_ENV, user.env)


if __name__ == "__main__":
    sys.exit(main())
