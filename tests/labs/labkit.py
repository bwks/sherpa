"""What the lab runners share (Python 3.15+, stdlib only): the failure and outcome types, running
and logging a command, the run's secrets and their masking in the logs, the TOML of a run's
manifest, the device password preflight, generated passwords, and a command in a pseudo-terminal
of its own whose prompts the runner answers.

A lab runner imports it from the directory above its own. Run as a script, it runs its own unit
tests.
"""

import fcntl
import json
import os
from pathlib import Path
import re
import secrets
import select
import shlex
import string
import struct
import subprocess
import sys
import tempfile
import termios
import time
from typing import NamedTuple
import unittest


COMMAND_TIMEOUT = 180
PROMPT_TIMEOUT = 60
# How long a finished command's output is read at most, so output held open by something else
# cannot hang the runner.
DRAIN_SECONDS = 5
# A generated password: a letter or digit, then letters, digits and punctuation both the login
# password and the device password rules take.
PASSWORD_LENGTH = 16
PASSWORD_PUNCTUATION = "_-"
MASK = "********"
DEVICE_PASSWORD_SET = re.compile(r"^\s*Device password:\s*set\s*$", re.MULTILINE)
INFO_USERNAME = re.compile(r"^\s*Username:\s*(\S+)\s*$", re.MULTILINE)
INFO_ADMIN = re.compile(r"^\s*Admin:\s*(Yes|No)\s*$", re.MULTILINE)
# The "device status report" a program sends to ask for the cursor position, and a terminal's
# answer: row 1, column 1.
CURSOR_POSITION_QUERY = b"\x1b[6n"
CURSOR_POSITION_REPORT = b"\x1b[1;1R"
# Every secret of the run, such as a generated password or a token, masked wherever it is logged.
SECRETS = []


class TestFailure(Exception):
    """A check failed; the message says which and why."""

    __test__ = False


class Outcome(NamedTuple):
    """A finished command. Parse `stdout`; search `output` for messages, which may be on either stream."""

    code: int | None
    stdout: str
    stderr: str

    @property
    def output(self):
        return self.stdout + self.stderr


def require(condition, message):
    if not condition:
        raise TestFailure(mask(message))


def add_secret(secret):
    if secret and secret not in SECRETS:
        SECRETS.append(secret)


def mask(text):
    """`text` with every secret of the run replaced, for the logs."""
    for secret in SECRETS:
        text = text.replace(secret, MASK)
    return text


def log_name(log):
    return getattr(log, "name", "the log")


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT, env=None, stdin=None):
    """Run a command, record it and its output in the log, and return its Outcome. `stdin` is the
    text the command reads; with none, it reads nothing."""
    print(f"[{cwd.name}] {label}", flush=True)
    feed = {"input": stdin} if stdin is not None else {"stdin": subprocess.DEVNULL}
    try:
        result = subprocess.run(arguments, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                                env=env, **feed)
        outcome = Outcome(result.returncode, result.stdout, result.stderr)
    except subprocess.TimeoutExpired as error:
        partial = [part.decode(errors="replace") if isinstance(part, bytes) else part or ""
                   for part in (error.stdout, error.stderr)]
        outcome = Outcome(None, partial[0], f"{partial[1]}\ntimed out after {timeout}s")
    except KeyboardInterrupt:
        log.write(mask(f"=== {label}: {shlex.join(arguments)}\n--- interrupted\n"))
        log.flush()
        raise
    log.write(mask(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
                   f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n"))
    log.flush()
    return outcome


def stop_on_sigterm(_signal, _frame):
    raise KeyboardInterrupt("SIGTERM")


def value_toml(value):
    """A value as inline TOML."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(value_toml(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {value_toml(item)}" for key, item in value.items()) + " }"
    raise TypeError(f"Unsupported TOML value: {value!r}")


def require_device_password(log, run_root):
    """Stop unless the lab owner has a device password, which `sherpa up` needs; return the owner."""
    info = run_logged(log, "sherpa server user info", ["sherpa", "server", "user", "info"], run_root)
    require(info.code == 0, "could not read the user's details (`sherpa server user info`)")
    require(DEVICE_PASSWORD_SET.search(info.stdout),
            "the user has no device password, which `sherpa up` needs: "
            "run `sherpa server user device-password`")
    owner = INFO_USERNAME.search(info.stdout)
    require(owner, "`sherpa server user info` did not name the user")
    return owner.group(1)


def password_fits(password, username):
    """Whether a generated password meets both the login and the device password rules."""
    return (PASSWORD_LENGTH == len(password) and password[0].isalnum()
            and all(c.isascii() and (c.isalnum() or c in PASSWORD_PUNCTUATION) for c in password)
            and any(c.islower() for c in password) and any(c.isupper() for c in password)
            and any(c.isdigit() for c in password) and any(c in PASSWORD_PUNCTUATION for c in password)
            and username.lower() not in password.lower() and "cisco" not in password.lower()
            and not any(a == b == c for a, b, c in zip(password, password[1:], password[2:])))


def generate_password(username):
    """A random password for the run that meets both the login and the device password rules. It is
    only held in memory, and masked in the logs."""
    first = string.ascii_letters + string.digits
    rest = first + PASSWORD_PUNCTUATION
    while True:
        password = secrets.choice(first) + "".join(secrets.choice(rest) for _ in range(PASSWORD_LENGTH - 1))
        if password_fits(password, username):
            add_secret(password)
            return password


def parse_key_list(stdout):
    """The keys in `sherpa server --output json user ssh-key list`'s output."""
    try:
        return json.loads(stdout[stdout.index("{"):])["ssh_keys"]
    except (ValueError, KeyError) as error:
        raise TestFailure(f"the key list printed no keys: {stdout.strip()}") from error


def drain(read, limit=DRAIN_SECONDS):
    """Keep what a finished command printed: call `read` (which returns False at the end of the
    output) until the end, for at most `limit` seconds, so output held open by something else
    cannot hang the runner."""
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline and read(0.1):
        pass


class Terminal:
    """A command in a pseudo-terminal of its own, as a user's terminal runs it: it is the command's
    controlling terminal, so `ssh`, `sherpa ssh` and the password prompts of `sherpa` read from it."""

    def __init__(self, log, label, arguments, cwd, env):
        self.log = log
        self.label = label
        self.transcript = bytearray()
        master, slave = os.openpty()
        # A terminal of a real size: a full-screen CLI lays its prompt out by it.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 120, 0, 0))
        self.master = master
        self.process = subprocess.Popen(arguments, cwd=cwd, env=env, stdin=slave, stdout=slave,
                                        stderr=slave, preexec_fn=lambda: os.login_tty(slave))
        os.close(slave)
        print(f"[{Path(cwd).name}] {label}", flush=True)

    def read(self, timeout):
        """Read what the command printed within `timeout` seconds; False once it has closed."""
        ready, _, _ = select.select([self.master], [], [], timeout)
        if not ready:
            return True
        try:
            data = os.read(self.master, 4096)
        except OSError:
            return False
        self.transcript += data
        # A full-screen CLI such as SR Linux's asks the terminal where its cursor is and waits for
        # the answer; a real terminal replies, so this one does too.
        for _ in range(data.count(CURSOR_POSITION_QUERY)):
            os.write(self.master, CURSOR_POSITION_REPORT)
        return bool(data)

    def text(self):
        return self.transcript.decode(errors="replace").replace("\r\n", "\n")

    def send(self, data):
        os.write(self.master, data)

    def running(self):
        return self.process.poll() is None

    def wait_for(self, condition, timeout, what):
        """Wait until `condition(text)` holds; return the text."""
        deadline = time.monotonic() + timeout
        while True:
            text = self.text()
            if condition(text):
                return text
            remaining = deadline - time.monotonic()
            require(remaining > 0, f"{self.label}: {what} not seen within {timeout}s; see {log_name(self.log)}")
            open_ = self.read(min(remaining, 1))
            require(open_ or self.running(), f"{self.label} ended before {what}; see {log_name(self.log)}")

    def wait_for_end(self, timeout):
        """Read until the command ends; whether it did within `timeout` seconds."""
        deadline = time.monotonic() + timeout
        while self.running() and time.monotonic() < deadline:
            self.read(0.5)
        return not self.running()

    def kill(self):
        if self.running():
            self.process.kill()

    def finish(self, timeout=PROMPT_TIMEOUT):
        """Wait for the command to end and return its exit code, keeping what it printed."""
        ended = self.wait_for_end(timeout)
        drain(self.read)
        code = self.process.poll()
        if code is None:
            self.process.kill()
            self.process.wait()
        os.close(self.master)
        self.log.write(mask(f"=== {self.label}\n--- exit {code}\n--- terminal\n{self.text()}\n"))
        self.log.flush()
        require(ended, f"{self.label} did not end within {timeout}s; see {log_name(self.log)}")
        return code


def answer_prompts(terminal, answers, timeout=PROMPT_TIMEOUT):
    """Type each `(prompt, answer)` of `answers`, in order, once its prompt shows, then wait for the
    command to end and return its exit code and what it printed."""
    try:
        seen = 0
        for prompt, answer in answers:
            text = terminal.wait_for(lambda text, prompt=prompt: prompt in text[seen:], timeout,
                                     f"the prompt {prompt!r}")
            seen = text.index(prompt, seen) + len(prompt)
            terminal.send(answer.encode() + b"\r")
    except BaseException:
        terminal.kill()
        terminal.finish()
        raise
    code = terminal.finish(timeout)
    return code, terminal.text()


class SelfTest(unittest.TestCase):
    # The helpers' own checks, run as `uv run tests/labs/labkit.py`; pytest must not collect them
    # as lab cases.
    __test__ = False

    def setUp(self):
        saved = list(SECRETS)
        self.addCleanup(lambda: SECRETS.__setitem__(slice(None), saved))

    def test_a_secret_is_masked_in_the_log_and_in_failures(self):
        add_secret("hunter2")
        add_secret("")
        self.assertEqual(mask("pw hunter2, again hunter2"), f"pw {MASK}, again {MASK}")
        with self.assertRaises(TestFailure) as raised:
            require(False, "login with hunter2 failed")
        self.assertEqual(str(raised.exception), f"login with {MASK} failed")
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)
            with (path / "log").open("w+") as log:
                run_logged(log, "echo", ["echo", "hunter2"], path)
                log.seek(0)
                text = log.read()
        self.assertNotIn("hunter2", text)
        self.assertIn(MASK, text)

    def test_a_generated_password_meets_both_rules_and_is_masked(self):
        passwords = {generate_password("owner1a2b3c4d") for _ in range(50)}
        self.assertEqual(len(passwords), 50)
        for password in passwords:
            self.assertTrue(password_fits(password, "owner1a2b3c4d"))
            self.assertEqual(mask(f"pw={password}"), f"pw={MASK}")
        self.assertFalse(password_fits("aaab-Cdefghij1kl", "x"))
        self.assertFalse(password_fits("-Abcdefghij1klmn", "x"))
        self.assertFalse(password_fits("Abcdefghij1klmno", "x"))
        self.assertFalse(password_fits("Abc-ciscoj1klmno", "x"))
        self.assertFalse(password_fits("Abc-owner1A2B3C4", "owner1a2b3c4d"[:10]))

    def test_keys_are_read_from_the_json_after_any_note(self):
        self.assertEqual(parse_key_list('note\n{"username": "a", "ssh_keys": ["ssh-ed25519 AAAA x"]}'),
                         ["ssh-ed25519 AAAA x"])
        with self.assertRaises(TestFailure):
            parse_key_list("no keys")

    def test_the_command_reads_the_given_text_and_otherwise_nothing(self):
        with tempfile.TemporaryDirectory() as temp, open(Path(temp) / "log", "w") as log:
            echo = [sys.executable, "-I", "-c", "import sys; print(sys.stdin.read())"]
            self.assertEqual(run_logged(log, "fed", echo, Path(temp), stdin="typed").stdout, "typed\n")
            self.assertEqual(run_logged(log, "unfed", echo, Path(temp)).stdout, "\n")

    def test_prompts_are_answered_in_order_and_not_echoed_to_the_log(self):
        script = ("import getpass, sys\n"
                  "first = getpass.getpass('Password for bob: ')\n"
                  "second = getpass.getpass('Confirm password: ')\n"
                  "sys.exit(0 if first == second == 's3cret' else 3)\n")
        add_secret("s3cret")
        with tempfile.TemporaryDirectory() as temp, open(Path(temp) / "log", "w") as log:
            terminal = Terminal(log, "prompts", [sys.executable, "-I", "-c", script], Path(temp), os.environ.copy())
            code, text = answer_prompts(terminal, [("Password for bob: ", "s3cret"), ("Confirm password: ", "s3cret")])
            log.flush()
            self.assertEqual(code, 0)
            self.assertNotIn("s3cret", text)
            self.assertNotIn("s3cret", (Path(temp) / "log").read_text())

    def test_values_are_written_as_inline_toml(self):
        self.assertEqual(value_toml({"name": "a", "count": 2, "skip": True, "env": ["X=1"]}),
                         '{ name = "a", count = 2, skip = true, env = ["X=1"] }')


if __name__ == "__main__":
    unittest.main()
