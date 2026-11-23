#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = ["pytest==9.2.0"]
# ///
"""End-to-end test of the account lockout through the sherpa CLI (Python 3.15+, pytest).

Checks the e2e cases of `user_login` in tests/specs/accounts-and-auth.md: failed logins in a row
lock a throwaway account, which `sherpa server user unlock` unlocks (024), and root on the server
host unlocks a locked account, `sherpa-admin` included, with `sherpad user unlock` while the
server is stopped (033). Brings no lab up. Runs wherever the `sherpa` CLI is logged in as an admin; 033
also needs the server host, passwordless sudo and `--stop-server`, as it stops sherpad; see
README.md.

A pytest module: the `account` fixture creates a throwaway user for each case as the admin, in a
HOME of its own, and deletes it at the end, even when the case fails. Run as a script, it runs
pytest on itself; `--self-test` runs the runner's own unit tests instead.
"""

import argparse
import os
from pathlib import Path
import re
import shutil
import signal
import ssl
import sys
import tempfile
import time
import tomllib
import unittest
from unittest import mock
import urllib.request
import uuid

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    INFO_ADMIN,
    Terminal,
    TestFailure,
    add_secret,
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

# Case 033 stops sherpad, so it runs only when this is set, by `--stop-server`.
STOP_SERVER_ENV = "SHERPA_TEST_STOP_SERVER"
BUILT_IN_ADMIN = "sherpa-admin"
SHERPAD_AS_SHERPA = ("sudo", "-n", "-u", "sherpa", "sherpad")
SHERPAD_AS_ROOT = ("sudo", "-n", "sherpad")
SYSTEMCTL = ("sudo", "-n", "systemctl")
SERVICE = "sherpad"
READY_TIMEOUT = 60
READY_INTERVAL = 1
REFUSED_RUNNING = "sherpad is running"
REFUSED_ACCOUNT = "must run as the 'sherpa' service account, not 'root'"
NOT_FOUND = "User not found"
DEFAULT_WS_PORT = 3030
ADMIN_USERS_PAGE = "/admin/users"
AUTH_COOKIE = "sherpa_auth"


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

    def login(self, password, label, username=None):
        """`sherpa login` as the user, or as `username`, with `password`, in the user's HOME."""
        username = username or self.username
        return run_logged(self.log, label, ["sherpa", "login"], self.root,
                          env={**self.env, LOGIN_USER_ENV: username, LOGIN_PASSWORD_ENV: password})

    def lock(self, username=None):
        """Lock the user, or `username`, with LIMIT wrong passwords in a row."""
        username = username or self.username
        for attempt in range(1, LIMIT + 1):
            failed = self.login(f"wrong-{uuid.uuid4().hex}", f"sherpa login {username}, wrong password {attempt}",
                                username)
            require(failed.code != 0, f"a wrong password logged in as {username}; see {self.log.name}")

    def require_locked(self, label):
        """The user's right password is refused, as for a locked account."""
        locked = self.login(self.password, label)
        require(locked.code != 0 and REFUSED in locked.output,
                f"a locked account was not refused with {REFUSED!r}: {locked.output.strip()}")


def admin(log, root, label, arguments):
    """A `sherpa` command as the CLI's own login, the admin."""
    return run_logged(log, label, ["sherpa", *arguments], root)


# -- the fixtures --------------------------------------------------------------------------------


@pytest.fixture
def account():
    """The case's throwaway user, created by the admin the CLI is logged in as, and deleted at the
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
    account.lock()
    account.require_locked("sherpa login, locked")

    unlock = admin(account.log, account.root, "sherpa server user unlock",
                   ["server", "user", "unlock", account.username])
    require(unlock.code == 0 and re.search(rf"'{account.username}' unlocked", unlock.output),
            f"sherpa server user unlock failed (exit {unlock.code}): {unlock.output.strip()}")

    unlocked = account.login(account.password, "sherpa login, unlocked")
    require(unlocked.code == 0, f"the unlocked account could not log in: {unlocked.output.strip()}")


def server_address(config_path):
    """The base URL of the server's web UI, on its TLS port, and the CA certificate path, from
    the CLI's config."""
    config = tomllib.loads(config_path.read_text())
    port = config.get("ws_port", DEFAULT_WS_PORT)
    return f"https://{config['server_ipv4']}:{port}", config["server_connection"]["ca_cert_path"]


def locked_on_admin_page(log, username):
    """Whether the admin users page, read with the CLI's admin token, offers to unlock `username`,
    which it does for a locked account only."""
    config = Path.home() / ".sherpa" / "config" / "sherpa.toml"
    base, ca_cert = server_address(config)
    token = (Path.home() / ".sherpa" / "token").read_text().strip()
    add_secret(token)
    request = urllib.request.Request(f"{base}{ADMIN_USERS_PAGE}", headers={"Cookie": f"{AUTH_COOKIE}={token}"})
    with urllib.request.urlopen(request, context=ssl.create_default_context(cafile=ca_cert), timeout=30) as reply:
        page = reply.read().decode()
    locked = f'hx-post="{ADMIN_USERS_PAGE}/{username}/unlock"' in page
    log.write(f"=== GET {ADMIN_USERS_PAGE}: {username} locked: {locked}\n")
    log.flush()
    return locked


def unlock_on_host(account, label, username, command=SHERPAD_AS_SHERPA):
    """`sherpad user unlock username` on the server host, as `sherpa` unless `command` says."""
    return run_logged(account.log, label, [*command, "user", "unlock", username], Path("/"))


def systemctl(account, action):
    outcome = run_logged(account.log, f"systemctl {action} {SERVICE}", [*SYSTEMCTL, action, SERVICE], Path("/"))
    require(outcome.code == 0, f"systemctl {action} {SERVICE} failed: {outcome.output.strip()}")


def start_and_wait(account):
    """Start sherpad and wait until the CLI's admin is answered."""
    systemctl(account, "start")
    deadline = time.monotonic() + READY_TIMEOUT
    while time.monotonic() < deadline:
        info = admin(account.log, account.root, "sherpa server user info, ready?", ["server", "user", "info"])
        if info.code == 0:
            return
        time.sleep(READY_INTERVAL)
    raise TestFailure(f"sherpad was not ready {READY_TIMEOUT}s after it started; see {account.log.name}")


@pytest.mark.skipif(os.environ.get(STOP_SERVER_ENV) != "1",
                    reason=f"stops sherpad on the server host; run with --stop-server or {STOP_SERVER_ENV}=1")
def test_user_login_033_root_unlocks_a_locked_account_sherpa_admin_included_with_sherpad_user_unlock_while_the_server_is_stopped(
        account):
    require(shutil.which("sherpad"), "sherpad is not on PATH: this case runs on the server host")
    unknown = f"{USER_PREFIX}unknown{uuid.uuid4().hex[:8]}"
    account.lock()
    account.require_locked("sherpa login, locked")
    account.lock(BUILT_IN_ADMIN)
    require(locked_on_admin_page(account.log, BUILT_IN_ADMIN), "sherpa-admin is not locked after 3 wrong passwords")
    stopped = False
    try:
        running = unlock_on_host(account, "sherpad user unlock, server running", account.username)
        require(running.code != 0 and REFUSED_RUNNING in running.output,
                f"sherpad user unlock was not refused while the server runs: {running.output.strip()}")
        account.require_locked("sherpa login, locked after the refused unlock")

        systemctl(account, "stop")
        stopped = True
        as_root = unlock_on_host(account, "sherpad user unlock as root", account.username, SHERPAD_AS_ROOT)
        require(as_root.code != 0 and REFUSED_ACCOUNT in as_root.output,
                f"sherpad user unlock as root was not refused: {as_root.output.strip()}")
        missing = unlock_on_host(account, "sherpad user unlock, unknown username", unknown)
        require(missing.code != 0 and NOT_FOUND in missing.output,
                f"sherpad user unlock of an unknown username did not fail: {missing.output.strip()}")
        start_and_wait(account)
        stopped = False
        account.require_locked("sherpa login, locked after the refused unlocks")
        require(locked_on_admin_page(account.log, BUILT_IN_ADMIN), "a refused unlock unlocked sherpa-admin")

        systemctl(account, "stop")
        stopped = True
        for username in (account.username, BUILT_IN_ADMIN):
            unlocked = unlock_on_host(account, f"sherpad user unlock {username}", username)
            require(unlocked.code == 0 and f"Unlocked {username}" in unlocked.stdout,
                    f"sherpad user unlock {username} failed (exit {unlocked.code}): {unlocked.output.strip()}")
        start_and_wait(account)
        stopped = False
        logged_in = account.login(account.password, "sherpa login, unlocked")
        require(logged_in.code == 0, f"the unlocked account could not log in: {logged_in.output.strip()}")
        require(not locked_on_admin_page(account.log, BUILT_IN_ADMIN), "sherpa-admin is still locked")
    finally:
        if stopped:
            start_and_wait(account)
        if locked_on_admin_page(account.log, BUILT_IN_ADMIN):
            admin(account.log, account.root, "sherpa server user unlock sherpa-admin, cleanup",
                  ["server", "user", "unlock", BUILT_IN_ADMIN])


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter,
                                     epilog="Other arguments, such as -k, are passed to pytest.")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    parser.add_argument("--stop-server", action="store_true",
                        help="Also run user_login_033, which stops and starts sherpad on this host")
    args, pytest_args = parser.parse_known_args()
    if args.stop_server:
        os.environ[STOP_SERVER_ENV] = "1"
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

    def test_the_server_address_is_read_from_the_cli_config_with_the_default_port(self):
        with tempfile.TemporaryDirectory() as root:
            config = Path(root) / "sherpa.toml"
            config.write_text('server_ipv4 = "192.0.2.1"\n[server_connection]\nca_cert_path = "/ca.crt"\n')
            self.assertEqual(server_address(config), ("https://192.0.2.1:3030", "/ca.crt"))
            config.write_text('server_ipv4 = "192.0.2.1"\nws_port = 8443\n'
                              '[server_connection]\nca_cert_path = "/ca.crt"\n')
            self.assertEqual(server_address(config)[0], "https://192.0.2.1:8443")


if __name__ == "__main__":
    sys.exit(main())
