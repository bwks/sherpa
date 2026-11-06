#!/usr/bin/env python3
"""The VM code suite's build cache (Python 3.11+, standard library only).

A Redis server runs on the Sherpa host in a container, and sccache in the test guests keeps
compiled crates in it. The password, the generated Redis configuration and the data live under
STATE, outside the checkout. A run uses the cache when STATE holds a password; without one the
guests build uncached.
"""

import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch


HARNESS = Path(__file__).resolve().parent
ROOT = HARNESS.parent.parent
SETTINGS = ROOT / "tests" / "environments" / "build-cache.toml"
STATE = Path.home() / ".local" / "state" / "sherpa-tests" / "build-cache"
CLI_CONFIG = Path.home() / ".sherpa" / "config" / "sherpa.toml"
# Where the Redis image reads the configuration it is started with.
REDIS_CONF = "/usr/local/etc/redis/redis.conf"


def load_settings(path=SETTINGS):
    return tomllib.loads(path.read_text())


def server_address(path=CLI_CONFIG):
    """The Sherpa server's address in the CLI configuration, where the test guests reach the host."""
    return tomllib.loads(path.read_text())["server_ipv4"]


def endpoint(settings, address):
    return f"redis://{address}:{settings['redis']['port']}"


def password(state=STATE):
    path = state / "credentials.toml"
    if not path.is_file():
        return ""
    return tomllib.loads(path.read_text())["password"]


def redis_config(settings, secret):
    return ("bind 0.0.0.0\n"
            "port 6379\n"
            f"requirepass {secret}\n"
            f"maxmemory {settings['redis']['max_memory']}\n"
            "maxmemory-policy allkeys-lru\n"
            # Snapshot to the data directory so the cache survives a host restart.
            "save 3600 1 300 100\n"
            "dir /data\n")


def docker(arguments, check=True, env=None):
    process = subprocess.run(["docker", *arguments], stdin=subprocess.DEVNULL, capture_output=True,
                             text=True, env=env)
    if check and process.returncode:
        raise RuntimeError(f"docker {arguments[0]} failed: {process.stderr.strip()}")
    return process


def container_exists(settings):
    return docker(["container", "inspect", settings["redis"]["container"]], check=False).returncode == 0


def redis_cli(settings, arguments, state=STATE):
    # redis-cli reads the password from REDISCLI_AUTH; passing the variable by name keeps it out
    # of the docker command line.
    return docker(["exec", "--env", "REDISCLI_AUTH", settings["redis"]["container"], "redis-cli", *arguments],
                  env={**os.environ, "REDISCLI_AUTH": password(state)}).stdout.strip()


def start(settings, address, state=STATE):
    redis = settings["redis"]
    if container_exists(settings):
        raise RuntimeError(f"{redis['container']} already exists; stop it first to recreate it")
    state.mkdir(parents=True, mode=0o700, exist_ok=True)
    (state / "data").mkdir(mode=0o700, exist_ok=True)
    credentials = state / "credentials.toml"
    if not credentials.is_file():
        credentials.touch(mode=0o600)
        credentials.write_text(f'password = "{secrets.token_hex(32)}"\n')
    config = state / "redis.conf"
    config.touch(mode=0o600)
    config.write_text(redis_config(settings, password(state)))
    # Run as the invoking user, who owns the configuration and the data directory.
    docker(["run", "--detach", "--name", redis["container"], "--restart", "unless-stopped",
            "--user", f"{os.getuid()}:{os.getgid()}",
            "--publish", f"{address}:{redis['port']}:6379",
            "--volume", f"{config}:{REDIS_CONF}:ro", "--volume", f"{state / 'data'}:/data",
            redis["image"], "redis-server", REDIS_CONF])
    print(f"Build cache running at {endpoint(settings, address)}")


def stop(settings):
    """Remove the container. The password and the cached data stay in STATE for the next start."""
    if container_exists(settings):
        docker(["rm", "--force", settings["redis"]["container"]])
    print("Build cache stopped")


def status(settings, address, state=STATE):
    if not container_exists(settings):
        print("Build cache is not running")
        return 1
    state_line = docker(["container", "inspect", "--format", "{{.State.Status}}",
                         settings["redis"]["container"]]).stdout.strip()
    print(f"Build cache {state_line} at {endpoint(settings, address)}")
    print(f"Entries: {redis_cli(settings, ['DBSIZE'], state)}")
    return 0


def clear(settings, state=STATE):
    redis_cli(settings, ["FLUSHALL"], state)
    print("Build cache cleared")


class BuildCacheTests(unittest.TestCase):
    settings = {"redis": {"image": "redis:test", "container": "cache", "port": 6379, "max_memory": "4gb"}}

    def test_password_is_empty_without_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(password(Path(directory)), "")

    def test_redis_config_requires_the_password_and_bounds_memory(self):
        config = redis_config(self.settings, "secret")
        self.assertIn("requirepass secret\n", config)
        self.assertIn("maxmemory 4gb\n", config)
        self.assertIn("maxmemory-policy allkeys-lru\n", config)

    def test_endpoint_uses_the_server_address(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sherpa.toml"
            path.write_text('server_ipv4 = "192.0.2.10"\nserver_port = 3030\n')
            self.assertEqual(endpoint(self.settings, server_address(path)), "redis://192.0.2.10:6379")

    def test_start_keeps_secrets_private_and_off_the_command_line(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch(f"{__name__}.container_exists", return_value=False), \
             patch(f"{__name__}.docker") as run:
            state = Path(directory) / "state"
            start(self.settings, "192.0.2.10", state)
            secret = password(state)
            self.assertEqual(len(secret), 64)
            for name in ("credentials.toml", "redis.conf"):
                self.assertEqual((state / name).stat().st_mode & 0o777, 0o600)
            self.assertEqual(state.stat().st_mode & 0o777, 0o700)
            arguments = run.call_args.args[0]
            self.assertNotIn(secret, " ".join(arguments))
            self.assertEqual(arguments[arguments.index("--publish") + 1], "192.0.2.10:6379:6379")
            # A restart keeps the password, so running guests and the stored cache stay valid.
            with patch(f"{__name__}.container_exists", return_value=False):
                start(self.settings, "192.0.2.10", state)
            self.assertEqual(password(state), secret)

    def test_start_refuses_an_existing_container(self):
        with patch(f"{__name__}.container_exists", return_value=True), \
             patch(f"{__name__}.docker") as run, \
             self.assertRaisesRegex(RuntimeError, "already exists"):
            start(self.settings, "192.0.2.10", Path("/unused"))
        run.assert_not_called()


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
