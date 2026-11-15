#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.15"
# dependencies = []
# ///
"""End-to-end test of Sherpa-built Talos clusters through the sherpa CLI (Python 3.15+, stdlib only).

Runs each lab in this folder against the server the logged-in `sherpa` CLI points at:
`sherpa up`, then checks from the lab's util node with talosctl and kubectl, exercises the
lifecycle commands, and always destroys the lab. See README.md.
"""

import argparse
import ipaddress
import base64
import copy
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
from typing import NamedTuple
import unittest
from unittest import mock
import uuid


LABS = Path(__file__).resolve().parent
ROOT = LABS.parent.parent.parent
RUNS = ROOT / ".tmp" / "lab-tests"
TOOLS_SCRIPT = LABS / "install-tools.sh"
LAB_NAMES = ("single", "ha")
TALOS_VERSION = "1.14.4"
CLUSTER = "k8s"
UTIL_NODE = "util"
# Cluster credentials: where `sherpa up` writes them in the lab folder, and the plain names
# `scp` gives them in the util node's home directory.
REMOTE_KUBECONFIG = f"kubeconfig-{CLUSTER}"
REMOTE_TALOSCONFIG = f"talosconfig-{CLUSTER}"
KUBECONFIG = f".sherpa/k8s/{REMOTE_KUBECONFIG}"
TALOSCONFIG = f".sherpa/talos/{REMOTE_TALOSCONFIG}"
# Host offset of the first Talos cluster VIP in a lab management subnet (TALOS_VIP_HOST_OFFSETS).
FIRST_VIP_HOST_OFFSET = 11
# Host offsets of the first Cilium cluster's Gateway and Hubble UI (TALOS_LOAD_BALANCER_HOST_OFFSETS).
GATEWAY_HOST_OFFSET = 13
HUBBLE_UI_HOST_OFFSET = 14
NAMESPACE = "sherpa-e2e"
WORKLOAD = "echo"
WORKLOAD_IMAGE = "registry.k8s.io/e2e-test-images/agnhost:2.66.1"
WORKLOAD_PORT = 8080

COMMAND_TIMEOUT = 180
UP_TIMEOUT = 1800
HEALTH_TIMEOUT = "10m"
READY_TIMEOUT = 600
# Talos VIP failover after an unclean loss of the holder takes up to about a minute.
VIP_FAILOVER_TIMEOUT = 300
POLL_INTERVAL = 10
ECHO_REQUESTS = 60
# Keycloak login (`oidc` on the cluster): the realm Sherpa makes, its admin group, and the user
# the test adds outside that group.
KEYCLOAK_REALM = "sherpa"
KEYCLOAK_ADMIN = "sherpa"
CLUSTER_ADMINS_GROUP = "cluster-admins"
OIDC_CLAIM_PREFIX = "oidc:"
OUTSIDER = "sherpa-e2e-outsider"
# Vault secrets (`vault` on the cluster): the operator Sherpa installs, and the secret the test
# writes under the cluster's path and syncs into the test namespace.
VSO_NAMESPACE = "vault-secrets-operator-system"
VSO_INSTALL_JOB = "vault-secrets-operator-install"
VAULT_SECRET = "vault-demo"
VAULT_SECRET_KEY = "value"
VAULT_REFRESH = "10s"
VAULT_SYNC_TIMEOUT = 300


# OpenSSH options for lab nodes: no prompts, and no host key notices mixed into command output.
SSH_OPTIONS = ("-F", ".sherpa/ssh/config", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR")


class TestFailure(Exception):
    """A check failed; the message says which and why."""


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
        raise TestFailure(message)


def run_logged(log, label, arguments, cwd, timeout=COMMAND_TIMEOUT, stdin=None):
    """Run a command, record it and its output in the log, and return its Outcome."""
    print(f"[{cwd.name}] {label}", flush=True)
    try:
        result = subprocess.run(arguments, cwd=cwd, input=stdin, capture_output=True, text=True,
                                timeout=timeout)
        outcome = Outcome(result.returncode, result.stdout, result.stderr)
    except subprocess.TimeoutExpired as error:
        partial = [part.decode(errors="replace") if isinstance(part, bytes) else part or ""
                   for part in (error.stdout, error.stderr)]
        outcome = Outcome(None, partial[0], f"{partial[1]}\ntimed out after {timeout}s")
    except KeyboardInterrupt:
        log.write(f"=== {label}: {shlex.join(arguments)}\n--- interrupted\n")
        log.flush()
        raise
    log.write(f"=== {label}: {shlex.join(arguments)}\n--- exit {outcome.code}\n"
              f"--- stdout\n{outcome.stdout}\n--- stderr\n{outcome.stderr}\n")
    log.flush()
    return outcome


def stop_on_sigterm(_signal, _frame):
    raise KeyboardInterrupt("SIGTERM")


def value_toml(value):
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


def render_manifest(manifest, name, tools_script):
    """The lab manifest for one run: a unique lab name and an absolute tooling script path."""
    lines = []
    for key, value in manifest.items():
        if key in ("talos_clusters", "nodes"):
            continue
        lines.append(f"{key} = {value_toml(name if key == 'name' else value)}")
    for section in ("talos_clusters", "nodes"):
        lines.append("")
        lines.append(f"{section} = [")
        for entry in manifest[section]:
            entry = dict(entry)
            if "startup_scripts" in entry:
                entry["startup_scripts"] = [str(tools_script)]
            lines.append(f"  {value_toml(entry)},")
        lines.append("]")
    return "\n".join(lines) + "\n"


def lab_cluster(manifest):
    clusters = [cluster for cluster in manifest["talos_clusters"] if cluster["name"] == CLUSTER]
    require(len(clusters) == 1, f"Manifest must define exactly one talos cluster named {CLUSTER}")
    return clusters[0]


def cluster_members(manifest):
    cluster = lab_cluster(manifest)
    return cluster["controllers"], cluster.get("workers", [])


def node_conditions(nodes_json):
    """Map each Kubernetes node name to (Ready, InternalIP)."""
    nodes = {}
    for item in nodes_json["items"]:
        ready = any(condition["type"] == "Ready" and condition["status"] == "True"
                    for condition in item["status"].get("conditions", []))
        addresses = [address["address"] for address in item["status"].get("addresses", [])
                     if address["type"] == "InternalIP"]
        nodes[item["metadata"]["name"]] = (ready, addresses[0] if addresses else None)
    return nodes


def host_address(ipv4_network, offset):
    network = ipaddress.ip_network(ipv4_network)
    return str(network.network_address + offset)


def first_vip(ipv4_network):
    return host_address(ipv4_network, FIRST_VIP_HOST_OFFSET)


def gateway_route():
    """An HTTPRoute sending the cluster Gateway's HTTP traffic to the workload."""
    return {
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "HTTPRoute",
        "metadata": {"name": WORKLOAD, "namespace": NAMESPACE},
        "spec": {
            "parentRefs": [{"name": "sherpa", "namespace": "default"}],
            "rules": [{"backendRefs": [{"name": WORKLOAD, "port": 80}]}],
        },
    }


def keycloak_credentials_path(node):
    return f".sherpa/keycloak/keycloak-{node}.toml"


def vault_credentials_path(node):
    return f".sherpa/vault/vault-{node}.toml"


def realm_member(users):
    """The lab owner from a Keycloak credentials file's users: the one that is not Sherpa's admin."""
    owners = [user for user in users if user["username"] != KEYCLOAK_ADMIN]
    require(owners, f"Keycloak credentials list no user besides {KEYCLOAK_ADMIN}")
    return owners[0]


def password_grant_kubeconfig(kubeconfig, cluster, username, password):
    """A kubeconfig that logs in as one Keycloak user through `kubectl oidc-login`, without a prompt.

    It copies the `oidc@<cluster>` user Sherpa wrote and adds the plugin's password grant (the
    `kubernetes` client allows direct grants), with a token cache of the user's own.
    """
    oidc_user = f"oidc@{cluster}"
    users = [user for user in kubeconfig["users"] if user["name"] == oidc_user]
    require(len(users) == 1, f"kubeconfig has no {oidc_user} user")
    user = copy.deepcopy(users[0])
    user["name"] = username
    user["user"]["exec"]["args"] += [
        "--grant-type=password",
        f"--username={username}",
        f"--password={password}",
        f"--token-cache-dir=~/.kube/cache/oidc-login-{username}",
    ]
    user["user"]["exec"]["interactiveMode"] = "Never"
    return {
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": kubeconfig["clusters"],
        "users": [user],
        "contexts": [{"name": username, "context": {"cluster": cluster, "user": username}}],
        "current-context": username,
    }


def outsider_user(password):
    """A Keycloak realm user in no group, for the admin API's user create."""
    return {
        "username": OUTSIDER,
        "enabled": True,
        "email": f"{OUTSIDER}@{KEYCLOAK_REALM}.lab.local",
        "emailVerified": True,
        "firstName": OUTSIDER,
        "lastName": KEYCLOAK_REALM,
        "credentials": [{"type": "password", "value": password, "temporary": False}],
    }


def vault_static_secret():
    """A VaultStaticSecret syncing secret/<cluster>/<namespace> into the test namespace."""
    return {
        "apiVersion": "secrets.hashicorp.com/v1beta1",
        "kind": "VaultStaticSecret",
        "metadata": {"name": VAULT_SECRET, "namespace": NAMESPACE},
        "spec": {
            "type": "kv-v2",
            "mount": "secret",
            "path": f"{CLUSTER}/{NAMESPACE}",
            "destination": {"name": VAULT_SECRET, "create": True},
            "refreshAfter": VAULT_REFRESH,
        },
    }


def secret_value(secret_json, key):
    data = secret_json.get("data") or {}
    return base64.b64decode(data[key]).decode() if key in data else None


def owner_only(path):
    return stat.S_IMODE(path.stat().st_mode) == 0o600


def workload_objects(replicas):
    labels = {"app": WORKLOAD}
    return {
        "apiVersion": "v1",
        "kind": "List",
        "items": [
            {"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": NAMESPACE}},
            {
                "apiVersion": "apps/v1",
                "kind": "Deployment",
                "metadata": {"name": WORKLOAD, "namespace": NAMESPACE},
                "spec": {
                    "replicas": replicas,
                    "selector": {"matchLabels": labels},
                    "template": {
                        "metadata": {"labels": labels},
                        "spec": {
                            # One pod per worker, so requests cross nodes over the pod network.
                            "topologySpreadConstraints": [{
                                "maxSkew": 1,
                                "topologyKey": "kubernetes.io/hostname",
                                "whenUnsatisfiable": "DoNotSchedule",
                                "labelSelector": {"matchLabels": labels},
                            }],
                            "containers": [{
                                "name": WORKLOAD,
                                "image": WORKLOAD_IMAGE,
                                "args": ["netexec", f"--http-port={WORKLOAD_PORT}"],
                                "ports": [{"containerPort": WORKLOAD_PORT}],
                            }],
                        },
                    },
                },
            },
            {
                "apiVersion": "v1",
                "kind": "Service",
                "metadata": {"name": WORKLOAD, "namespace": NAMESPACE},
                "spec": {
                    "type": "NodePort",
                    "selector": labels,
                    "ports": [{"port": 80, "targetPort": WORKLOAD_PORT}],
                },
            },
        ],
    }


class LabRun:
    def __init__(self, lab, run_root, keep_failed):
        self.lab = lab
        self.manifest = tomllib.loads((LABS / lab / "manifest.toml").read_text())
        self.controllers, self.workers = cluster_members(self.manifest)
        self.cilium = lab_cluster(self.manifest).get("cni") == "cilium"
        self.oidc_node = lab_cluster(self.manifest).get("oidc")
        self.vault_node = lab_cluster(self.manifest).get("vault")
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / lab
        self.keep_failed = keep_failed
        self.started = False
        self.lab_id = None
        self.ipv4_network = None
        self.up_output = ""
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin)
        if check and outcome.code != 0:
            raise TestFailure(f"{label} failed (exit {outcome.code}); see {self.log.name}")
        return outcome

    def sherpa(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.command(label, ["sherpa", *arguments], timeout, check)

    def util(self, label, remote_command, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        return self.command(label, ["ssh", *SSH_OPTIONS, f"{UTIL_NODE}.{self.lab_id}", remote_command],
                            timeout, check, stdin)

    def kubectl(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        return self.util(label, shlex.join(["kubectl", "--kubeconfig", REMOTE_KUBECONFIG, *arguments]),
                         timeout, check, stdin)

    def talosctl(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True):
        return self.util(label, shlex.join(["talosctl", "--talosconfig", REMOTE_TALOSCONFIG, *arguments]),
                         timeout, check)

    # -- lab ---------------------------------------------------------------------------------

    def run(self):
        self.directory.mkdir(parents=True)
        with (self.directory / "commands.log").open("w") as self.log:
            (self.directory / "manifest.toml").write_text(
                render_manifest(self.manifest, self.name, TOOLS_SCRIPT))
            failure = None
            try:
                self.up()
                self.checks()
            except (TestFailure, OSError, KeyError, ValueError) as error:
                failure = error
            except BaseException:
                # Interrupted: never leave the lab behind, then stop.
                self.destroy()
                raise
            if failure and self.keep_failed:
                print(f"[{self.lab}] keeping failed lab {self.name} ({self.lab_id}) in {self.directory}")
            else:
                cleanup = self.destroy()
                failure = failure or cleanup
        if failure:
            raise TestFailure(f"{self.lab}: {failure}")

    def up(self):
        self.started = True
        outcome = self.sherpa("sherpa up", ["up"], UP_TIMEOUT, check=False)
        self.up_output = outcome.output
        # A partly created lab exits non-zero after writing lab-info.toml: record its ID first, so
        # destroy can check what it leaves behind.
        info_path = self.directory / ".sherpa" / "lab-info.toml"
        if info_path.exists():
            info = tomllib.loads(info_path.read_text())
            require(info["name"] == self.name, f".sherpa/lab-info.toml names {info['name']}, not {self.name}")
            self.lab_id = info["id"]
            self.ipv4_network = info["ipv4_network"]
        require(outcome.code == 0, f"sherpa up failed (exit {outcome.code}); see {self.log.name}")
        require(self.lab_id is not None, "sherpa up wrote no .sherpa/lab-info.toml")
        for name in self.credential_files():
            path = self.directory / name
            require(path.is_file(), f"sherpa up did not write {name}")
            require(owner_only(path), f"{name} is not readable only by its owner")
        self.wait_for_util()
        self.util("util tooling installed", "cloud-init status --wait && talosctl version --client "
                  "&& kubectl version --client && kubectl oidc-login --version", timeout=900)
        self.copy_credentials()

    def credential_files(self):
        files = [KUBECONFIG, TALOSCONFIG]
        if self.oidc_node:
            files.append(keycloak_credentials_path(self.oidc_node))
        if self.vault_node:
            files.append(vault_credentials_path(self.vault_node))
        return files

    def copy_credentials(self):
        self.command("copy credentials to util", ["scp", *SSH_OPTIONS, KUBECONFIG, TALOSCONFIG,
                                                  f"{UTIL_NODE}.{self.lab_id}:"])
        # Flush the copies so they survive a forced power-off if the guest ignores `sherpa down`.
        self.util("flush credentials to disk", "sync")

    def wait_for_util(self, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.util("util reachable", "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"util node not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def checks(self):
        self.wait_for_ready_nodes(self.controllers + self.workers)
        self.health()
        self.workload()
        if self.cilium:
            self.cilium_cluster()
        if self.oidc_node:
            self.oidc_login()
        if self.vault_node:
            self.vault_secret_sync()
        if len(self.controllers) == 3:
            self.vip_endpoint()
            self.survives_controller_loss()
        else:
            self.redeploys()
            self.down_and_resume()
            self.download_restores_credentials()

    def nodes(self):
        outcome = self.kubectl("kubectl get nodes", ["get", "nodes", "-o", "json"], check=False)
        try:
            return node_conditions(json.loads(outcome.stdout))
        except (json.JSONDecodeError, KeyError):
            return {}

    def wait_for_ready_nodes(self, expected, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while True:
            nodes = self.nodes()
            not_ready = [name for name in expected if not nodes.get(name, (False, None))[0]]
            if not not_ready:
                return nodes
            require(time.monotonic() < deadline, f"nodes not Ready after {timeout}s: {not_ready}")
            time.sleep(POLL_INTERVAL)

    def health(self):
        nodes = self.wait_for_ready_nodes(self.controllers + self.workers)
        controller_ips = [nodes[name][1] for name in self.controllers]
        worker_ips = [nodes[name][1] for name in self.workers]
        arguments = ["--nodes", controller_ips[0], "health", "--wait-timeout", HEALTH_TIMEOUT,
                     "--control-plane-nodes", ",".join(controller_ips)]
        if worker_ips:
            arguments += ["--worker-nodes", ",".join(worker_ips)]
        self.talosctl("talosctl health", arguments, timeout=900)

    def workload(self):
        replicas = len(self.workers) or len(self.controllers)
        self.kubectl("apply workload", ["apply", "-f", "-"],
                     stdin=json.dumps(workload_objects(replicas)))
        self.kubectl("workload rollout", ["-n", NAMESPACE, "rollout", "status",
                                          f"deployment/{WORKLOAD}", "--timeout=5m"], timeout=360)
        pods = self.kubectl("workload pods", ["-n", NAMESPACE, "get", "pods", "-l",
                                              f"app={WORKLOAD}", "-o", "json"])
        pods = json.loads(pods.stdout)["items"]
        pod_names = {pod["metadata"]["name"] for pod in pods}
        pod_nodes = {pod["spec"]["nodeName"] for pod in pods}
        require(len(pod_nodes) == replicas, f"workload pods share nodes: {sorted(pod_nodes)}")
        service = self.kubectl("workload service", ["-n", NAMESPACE, "get", "service",
                                                    WORKLOAD, "-o", "json"])
        node_port = json.loads(service.stdout)["spec"]["ports"][0]["nodePort"]
        entry_ip = self.nodes()[(self.workers or self.controllers)[0]][1]
        loop = (f"for i in $(seq {ECHO_REQUESTS}); do curl -fsS --max-time 5 "
                f"http://{entry_ip}:{node_port}/hostname; echo; done")
        replies = self.util("requests through one node's NodePort", loop, timeout=600)
        answered = {line.strip() for line in replies.stdout.splitlines() if line.strip()}
        missing = pod_names - answered
        require(not missing, f"pods never answered through {entry_ip}:{node_port}: {sorted(missing)}")

    def cilium_cluster(self):
        gateway = host_address(self.ipv4_network, GATEWAY_HOST_OFFSET)
        hubble_ui = host_address(self.ipv4_network, HUBBLE_UI_HOST_OFFSET)
        for line in (f"Gateway: http://{gateway}", f"Hubble UI: http://{hubble_ui}"):
            require(line in self.up_output, f"sherpa up did not print {line!r}")
        daemonsets = self.kubectl("kube-system daemonsets", ["-n", "kube-system", "get", "daemonsets",
                                                             "-o", "jsonpath={.items[*].metadata.name}"])
        names = daemonsets.stdout.split()
        require("cilium" in names, f"no cilium daemonset: {names}")
        require("kube-proxy" not in names and "kube-flannel" not in names,
                f"kube-proxy or Flannel deployed alongside Cilium: {names}")
        job = self.kubectl("cilium install job", ["-n", "kube-system", "get", "job", "cilium-install",
                                                  "-o", "jsonpath={.status.succeeded}"])
        require(job.stdout.strip() == "1", f"cilium-install job did not succeed: {job.stdout!r}")
        self.kubectl("apply gateway route", ["apply", "-f", "-"], stdin=json.dumps(gateway_route()))
        # The route is programmed into the Gateway's Envoy a few seconds after it is accepted.
        for label, url in (("workload through the gateway", f"http://{gateway}/hostname"),
                           ("hubble ui", f"http://{hubble_ui}/")):
            self.util(label, f"for i in $(seq 30); do curl -fsS -o /dev/null --max-time 5 {url} "
                      f"&& exit 0; sleep {POLL_INTERVAL}; done; exit 1", timeout=600)

    def oidc_login(self):
        """A realm user in cluster-admins gets the cluster's resources through `kubectl oidc-login`;
        a user outside the group is authenticated but refused."""
        credentials = tomllib.loads((self.directory / keycloak_credentials_path(self.oidc_node)).read_text())
        passwords = {user["username"]: user["password"] for user in credentials["users"]}
        member = realm_member(credentials["users"])
        outsider_password = secrets.token_urlsafe(18)
        self.add_outsider(credentials["address"], passwords[KEYCLOAK_ADMIN], outsider_password)
        kubeconfig = json.loads((self.directory / KUBECONFIG).read_text())
        for username, password in ((member["username"], member["password"]), (OUTSIDER, outsider_password)):
            document = password_grant_kubeconfig(kubeconfig, CLUSTER, username, password)
            # On stdin, so the password is in no command line or log.
            self.util(f"kubeconfig for {username}", f"umask 077 && cat > oidc-{username}.kubeconfig",
                      stdin=json.dumps(document))

        whoami = self.oidc_kubectl(member["username"], ["auth", "whoami", "-o", "json"])
        status = json.loads(whoami.stdout)["status"]["userInfo"]
        require(status["username"] == f"{OIDC_CLAIM_PREFIX}{member['username']}",
                f"Keycloak login is {status['username']}, not {OIDC_CLAIM_PREFIX}{member['username']}")
        require(f"{OIDC_CLAIM_PREFIX}{CLUSTER_ADMINS_GROUP}" in status.get("groups", []),
                f"Keycloak login is not in {OIDC_CLAIM_PREFIX}{CLUSTER_ADMINS_GROUP}: {status.get('groups')}")
        nodes = self.oidc_kubectl(member["username"], ["get", "nodes", "-o", "json"])
        names = {item["metadata"]["name"] for item in json.loads(nodes.stdout)["items"]}
        require(names == set(self.controllers + self.workers), f"Keycloak login saw nodes {sorted(names)}")
        self.oidc_kubectl(member["username"], ["get", "pods", "--all-namespaces"])

        outsider = self.oidc_kubectl(OUTSIDER, ["auth", "whoami", "-o", "json"])
        outsider_name = json.loads(outsider.stdout)["status"]["userInfo"]["username"]
        require(outsider_name == f"{OIDC_CLAIM_PREFIX}{OUTSIDER}", f"outsider logged in as {outsider_name}")
        refused = self.oidc_kubectl(OUTSIDER, ["get", "pods", "--all-namespaces"], check=False)
        require(refused.code != 0 and "forbidden" in refused.output.lower(),
                f"a user outside {CLUSTER_ADMINS_GROUP} was not refused (exit {refused.code})")

    def oidc_kubectl(self, username, arguments, check=True):
        return self.util(f"kubectl as {username}: {' '.join(arguments[:2])}",
                         shlex.join(["kubectl", "--kubeconfig", f"oidc-{username}.kubeconfig", *arguments]),
                         check=check)

    def add_outsider(self, keycloak, admin_password, password):
        """Add a realm user in no group through Keycloak's admin API, from the util node."""
        # Line one of stdin is the admin password, the rest the user; neither is in the log.
        script = (
            "read -r admin && user=$(cat) && "
            "token=$(curl -fsS {token_url} -d grant_type=password -d client_id=admin-cli "
            "-d username={admin} --data-urlencode password=\"$admin\" "
            "| python3 -c 'import json, sys; print(json.load(sys.stdin)[\"access_token\"])') && "
            "curl -sS -o /dev/null -w '%{{http_code}}' -X POST {users_url} "
            "-H \"Authorization: Bearer $token\" -H 'Content-Type: application/json' -d \"$user\""
        ).format(token_url=shlex.quote(f"{keycloak}/realms/master/protocol/openid-connect/token"),
                 admin=shlex.quote(KEYCLOAK_ADMIN),
                 users_url=shlex.quote(f"{keycloak}/admin/realms/{KEYCLOAK_REALM}/users"))
        created = self.util("keycloak: add a user outside cluster-admins", script,
                            stdin=f"{admin_password}\n{json.dumps(outsider_user(password))}")
        require(created.stdout.strip() == "201", f"Keycloak user create answered {created.stdout.strip()!r}")

    def vault_secret_sync(self):
        """A secret written to Vault is synced into a Kubernetes Secret by the Vault Secrets
        Operator, and follows when the Vault value changes."""
        credentials = tomllib.loads((self.directory / vault_credentials_path(self.vault_node)).read_text())
        # The operator is installed by a Job a couple of minutes after the cluster starts.
        self.kubectl("vault secrets operator installed", ["-n", VSO_NAMESPACE, "wait", "--for=condition=Complete",
                                                          f"job/{VSO_INSTALL_JOB}", "--timeout=10m"], timeout=660)
        self.kubectl("vault secrets operator available", ["-n", VSO_NAMESPACE, "wait", "--for=condition=Available",
                                                          "deployment", "--all", "--timeout=5m"], timeout=360)
        first, second = secrets.token_hex(8), secrets.token_hex(8)
        self.write_vault_secret(credentials, first)
        self.kubectl("apply vault static secret", ["apply", "-f", "-"], stdin=json.dumps(vault_static_secret()))
        self.wait_for_secret_value(first)
        self.write_vault_secret(credentials, second)
        self.wait_for_secret_value(second)

    def write_vault_secret(self, credentials, value):
        url = f"{credentials['address']}/v1/secret/data/{CLUSTER}/{NAMESPACE}"
        # Line one of stdin is the root token, the rest the secret; neither is in the log. The
        # util node trusts the lab CA that Vault's certificate chains to.
        script = (f"read -r token && curl -fsS -o /dev/null -X POST {shlex.quote(url)} "
                  "-H \"X-Vault-Token: $token\" -H 'Content-Type: application/json' --data @-")
        body = json.dumps({"data": {VAULT_SECRET_KEY: value}})
        self.util("vault: write the test secret", script, stdin=f"{credentials['root_token']}\n{body}")

    def wait_for_secret_value(self, expected, timeout=VAULT_SYNC_TIMEOUT):
        deadline = time.monotonic() + timeout
        while True:
            secret = self.kubectl("synced secret", ["-n", NAMESPACE, "get", "secret", VAULT_SECRET, "-o", "json"],
                                  check=False)
            value = secret_value(json.loads(secret.stdout), VAULT_SECRET_KEY) if secret.code == 0 else None
            if value == expected:
                return
            require(time.monotonic() < deadline,
                    f"Secret {NAMESPACE}/{VAULT_SECRET} did not get the Vault value within {timeout}s")
            time.sleep(POLL_INTERVAL)

    def vip_endpoint(self):
        kubeconfig = json.loads((self.directory / KUBECONFIG).read_text())
        server = kubeconfig["clusters"][0]["cluster"]["server"]
        expected = f"https://{first_vip(self.ipv4_network)}:6443"
        require(server == expected, f"kubeconfig server is {server}, expected the VIP {expected}")

    def survives_controller_loss(self):
        stopped = self.controllers[0]
        self.sherpa(f"sherpa down {stopped}", ["down", "--node", stopped], UP_TIMEOUT)
        # The VIP moves to another controller once the stopped holder's lease expires, which
        # Talos documents as taking up to about a minute. Wait for that, then for the cluster to
        # see the stopped controller as not Ready, then require the API to stay up.
        deadline = time.monotonic() + VIP_FAILOVER_TIMEOUT
        while True:
            nodes = self.nodes()
            if stopped in nodes and not nodes[stopped][0]:
                break
            require(time.monotonic() < deadline,
                    f"API through the VIP did not report {stopped} as not Ready within "
                    f"{VIP_FAILOVER_TIMEOUT}s")
            time.sleep(POLL_INTERVAL)
        for attempt in range(3):
            self.kubectl(f"API through the VIP without {stopped} ({attempt + 1}/3)",
                         ["get", "--raw", "/readyz"])
            time.sleep(POLL_INTERVAL)
        self.sherpa(f"sherpa resume {stopped}", ["resume", "--node", stopped], UP_TIMEOUT)
        self.health()

    def redeploys(self):
        controller, worker = self.controllers[0], self.workers[0]
        refused = self.sherpa(f"sherpa redeploy {controller} (refused)",
                              ["redeploy", "--node", controller], check=False)
        require(refused.code != 0 and "controller of talos cluster" in refused.output,
                f"controller redeploy was not refused (exit {refused.code})")
        self.sherpa(f"sherpa redeploy {worker}", ["redeploy", "--node", worker], UP_TIMEOUT)
        self.health()

    def down_and_resume(self):
        down = self.sherpa("sherpa down", ["down"], UP_TIMEOUT)
        require("forced" not in down.output, "sherpa down forced a node off instead of shutting it down")
        self.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
        self.wait_for_util()
        self.health()

    def download_restores_credentials(self):
        saved = {}
        for name in (KUBECONFIG, TALOSCONFIG):
            path = self.directory / name
            saved[name] = path.read_text()
            path.unlink()
        self.sherpa("sherpa download", ["download", self.lab_id, "--force"])
        for name, content in saved.items():
            path = self.directory / name
            require(path.is_file() and path.read_text() == content, f"sherpa download did not restore {name}")
            require(owner_only(path), f"downloaded {name} is not readable only by its owner")

    def destroy(self):
        """Destroy the lab and check nothing is left. Returns a failure, or None."""
        if not self.started:
            return None
        if not (self.directory / ".sherpa" / "lab-info.toml").exists():
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            return None
        try:
            self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
            inspect = self.sherpa("lab gone", ["inspect"], check=False)
            require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
            self.no_libvirt_leftovers()
        except TestFailure as error:
            return error
        return None

    def no_libvirt_leftovers(self):
        if not shutil.which("virsh"):
            self.log.write("=== libvirt leftover check skipped: virsh not found\n")
            return
        for kind, arguments in (("domains", ["list", "--all", "--name"]),
                                ("networks", ["net-list", "--all", "--name"])):
            listing = self.command(f"no leftover {kind}", ["virsh", "-c", "qemu:///system", *arguments],
                                   check=False)
            if listing.code != 0:
                self.log.write(f"=== libvirt {kind} check skipped: virsh exited {listing.code}\n")
                continue
            leftovers = [name for name in listing.stdout.split() if self.lab_id in name]
            require(not leftovers, f"libvirt {kind} left after destroy: {leftovers}")


def preflight(run_root):
    for tool in ("sherpa", "ssh", "scp"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    with (run_root / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], run_root)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        image = run_logged(log, "talos image present", ["sherpa", "image", "show", "talos_linux",
                                                        "--version", TALOS_VERSION], run_root)
        require(image.code == 0, f"the server has no talos_linux {TALOS_VERSION} image; pull it with "
                f"`sherpa server image pull --model talos_linux --version {TALOS_VERSION}`")
        invalid = run_root / "two-controllers"
        invalid.mkdir()
        (invalid / "manifest.toml").write_text(
            'name = "two-controllers"\n'
            'talos_clusters = [{ name = "k8s", controllers = ["cp1", "cp2"] }]\n'
            'nodes = [\n'
            '  { name = "cp1", model = "talos_linux" },\n'
            '  { name = "cp2", model = "talos_linux" },\n'
            ']\n')
        validate = run_logged(log, "validate refuses two controllers", ["sherpa", "validate"], invalid)
        require(validate.code != 0 and "needs 1 or 3" in validate.output, "sherpa validate accepted a two-controller cluster")


def run_lab(lab, run_root, keep_failed):
    """Run one lab in this process: the child side of a parallel run."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    try:
        LabRun(lab, run_root, keep_failed).run()
    except TestFailure as error:
        print(f"FAIL {error}", file=sys.stderr, flush=True)
        return 1
    print(f"PASS {lab}", flush=True)
    return 0


def run_labs(labs, run_root, keep_failed):
    """Run every lab at once, each in its own process so an interrupt reaches each lab's cleanup."""
    children = {}
    for lab in labs:
        command = [sys.executable, str(Path(__file__).resolve()), "--child", lab, "--run-root", str(run_root)]
        if keep_failed:
            command.append("--keep-failed")
        children[lab] = subprocess.Popen(command)
    try:
        codes = {lab: child.wait() for lab, child in children.items()}
    except KeyboardInterrupt:
        # Ctrl-C reaches the children directly; SIGTERM reaches only this process, so pass it on.
        for child in children.values():
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
        for child in children.values():
            child.wait()
        raise
    return [lab for lab, code in codes.items() if code != 0]


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("labs", nargs="*", metavar="lab",
                        help=f"Labs to run at the same time: {', '.join(LAB_NAMES)} (default: all)")
    parser.add_argument("--keep-failed", action="store_true", help="Leave a failed lab running for debugging")
    parser.add_argument("--self-test", action="store_true", help="Run the runner's own unit tests")
    parser.add_argument("--child", help=argparse.SUPPRESS)
    parser.add_argument("--run-root", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.self_test:
        sys.argv = sys.argv[:1]
        unittest.main()
        return 0
    if args.child:
        return run_lab(args.child, args.run_root, args.keep_failed)
    labs = args.labs or list(LAB_NAMES)
    unknown = sorted(set(labs) - set(LAB_NAMES))
    if unknown:
        parser.error(f"unknown labs: {', '.join(unknown)}")
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = RUNS / f"talos-linux-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    print(f"Logs: {run_root}", flush=True)
    try:
        preflight(run_root)
    except TestFailure as error:
        print(f"FAIL preflight: {error}", file=sys.stderr)
        return 1
    failed = run_labs(labs, run_root, args.keep_failed)
    print(f"{'FAIL' if failed else 'PASS'}: {len(labs) - len(failed)}/{len(labs)} labs passed"
          + (f" (failed: {', '.join(failed)})" if failed else "") + f". Logs: {run_root}")
    return 1 if failed else 0


class SelfTest(unittest.TestCase):
    def manifest(self, lab):
        return tomllib.loads((LABS / lab / "manifest.toml").read_text())

    def test_a_partly_created_lab_keeps_its_id_for_cleanup(self):
        # `sherpa up` exits 1 on a partly created lab after writing lab-info.toml; the run must
        # still know the lab's ID, so destroy checks for leftovers instead of crashing.
        with tempfile.TemporaryDirectory() as temp:
            run = LabRun(LAB_NAMES[0], Path(temp) / "talos-linux-000000abcdef", keep_failed=False)
            (run.directory / ".sherpa").mkdir(parents=True)
            (run.directory / ".sherpa" / "lab-info.toml").write_text(
                f'id = "4ed6360e"\nname = "{run.name}"\nipv4_network = "172.31.0.0/24"\n')
            run.log = (run.directory / "commands.log").open("w")
            self.addCleanup(run.log.close)

            def failing_up(log, label, arguments, *rest):
                return Outcome(1 if arguments[:2] == ["sherpa", "up"] else 0, "", "")

            with mock.patch(f"{__name__}.run_logged", failing_up), self.assertRaises(TestFailure):
                run.up()
            self.assertEqual(run.lab_id, "4ed6360e")

            def leftover(log, label, arguments, *rest):
                return Outcome(1 if "inspect" in arguments else 0, "node-4ed6360e-x\n", "")

            with (mock.patch(f"{__name__}.run_logged", leftover),
                  mock.patch("shutil.which", return_value="/usr/bin/true")):
                self.assertIsInstance(run.destroy(), TestFailure)

    def test_rendered_manifests_round_trip_with_a_new_name_and_absolute_tools_path(self):
        for lab in LAB_NAMES:
            manifest = self.manifest(lab)
            rendered = tomllib.loads(render_manifest(manifest, "renamed", Path("/abs/install-tools.sh")))
            self.assertEqual(rendered["name"], "renamed")
            self.assertEqual(rendered["talos_clusters"], manifest["talos_clusters"])
            util = [node for node in rendered["nodes"] if node["name"] == UTIL_NODE]
            self.assertEqual(util[0]["startup_scripts"], ["/abs/install-tools.sh"])
            self.assertEqual(len(rendered["nodes"]), len(manifest["nodes"]))

    def test_lab_shapes(self):
        self.assertEqual(tuple(map(len, cluster_members(self.manifest("single")))), (1, 3))
        self.assertEqual(tuple(map(len, cluster_members(self.manifest("ha")))), (3, 1))

    def test_single_runs_flannel_and_ha_runs_cilium(self):
        self.assertNotIn("cni", lab_cluster(self.manifest("single")))
        self.assertEqual(lab_cluster(self.manifest("ha"))["cni"], "cilium")

    def test_ha_logs_in_through_keycloak_and_reads_secrets_from_vault(self):
        cluster = lab_cluster(self.manifest("ha"))
        models = {node["name"]: node["model"] for node in self.manifest("ha")["nodes"]}
        self.assertEqual(models[cluster["oidc"]], "keycloak_server")
        self.assertEqual(models[cluster["vault"]], "hashicorp_vault")
        self.assertNotIn("oidc", lab_cluster(self.manifest("single")))
        self.assertNotIn("vault", lab_cluster(self.manifest("single")))

    def test_credential_files_follow_the_cluster(self):
        with tempfile.TemporaryDirectory() as temp:
            ha = LabRun("ha", Path(temp) / "talos-linux-000000abcdef", keep_failed=False)
            single = LabRun("single", Path(temp) / "talos-linux-000000abcdef", keep_failed=False)
        self.assertEqual(ha.credential_files(), [KUBECONFIG, TALOSCONFIG, ".sherpa/keycloak/keycloak-kc01.toml",
                                                 ".sherpa/vault/vault-vault01.toml"])
        self.assertEqual(single.credential_files(), [KUBECONFIG, TALOSCONFIG])

    def test_realm_member_is_the_lab_owner(self):
        users = [{"username": "sherpa", "password": "a"}, {"username": "alice", "password": "b"}]
        self.assertEqual(realm_member(users)["username"], "alice")
        with self.assertRaises(TestFailure):
            realm_member(users[:1])

    def test_password_grant_kubeconfig_keeps_sherpas_oidc_settings(self):
        kubeconfig = {
            "clusters": [{"name": "k8s", "cluster": {"server": "https://172.31.1.11:6443"}}],
            "users": [
                {"name": "admin@k8s", "user": {"client-key-data": "secret"}},
                {"name": "oidc@k8s", "user": {"exec": {
                    "apiVersion": "client.authentication.k8s.io/v1", "command": "kubectl",
                    "args": ["oidc-login", "get-token", "--oidc-issuer-url=https://kc/realms/sherpa",
                             "--oidc-client-id=kubernetes", "--certificate-authority-data=Q0E="],
                    "interactiveMode": "IfAvailable"}}},
            ],
        }
        document = password_grant_kubeconfig(kubeconfig, "k8s", "alice", "pw")
        self.assertEqual([user["name"] for user in document["users"]], ["alice"])
        exec_config = document["users"][0]["user"]["exec"]
        self.assertEqual(exec_config["args"][:5], kubeconfig["users"][1]["user"]["exec"]["args"])
        self.assertIn("--grant-type=password", exec_config["args"])
        self.assertIn("--username=alice", exec_config["args"])
        self.assertIn("--password=pw", exec_config["args"])
        self.assertEqual(exec_config["interactiveMode"], "Never")
        self.assertEqual(document["contexts"], [{"name": "alice", "context": {"cluster": "k8s", "user": "alice"}}])
        self.assertEqual(document["current-context"], "alice")
        # Sherpa's own user is left as it was.
        self.assertEqual(len(kubeconfig["users"][1]["user"]["exec"]["args"]), 5)
        with self.assertRaises(TestFailure):
            password_grant_kubeconfig({"users": []}, "k8s", "alice", "pw")

    def test_outsider_is_in_no_group(self):
        user = outsider_user("pw")
        self.assertNotIn("groups", user)
        self.assertEqual(user["credentials"], [{"type": "password", "value": "pw", "temporary": False}])

    def test_vault_static_secret_reads_the_clusters_path(self):
        spec = vault_static_secret()["spec"]
        self.assertEqual((spec["mount"], spec["path"]), ("secret", f"{CLUSTER}/{NAMESPACE}"))
        self.assertEqual(spec["destination"], {"name": VAULT_SECRET, "create": True})

    def test_secret_value(self):
        encoded = base64.b64encode(b"s3cret").decode()
        self.assertEqual(secret_value({"data": {"value": encoded}}, "value"), "s3cret")
        self.assertIsNone(secret_value({"data": {}}, "value"))
        self.assertIsNone(secret_value({}, "value"))

    def test_manifest_tools_path_resolves_from_the_lab_folder(self):
        for lab in LAB_NAMES:
            util = [node for node in self.manifest(lab)["nodes"] if node["name"] == UTIL_NODE][0]
            self.assertEqual((LABS / lab / util["startup_scripts"][0]).resolve(), TOOLS_SCRIPT)

    def test_node_conditions(self):
        nodes = node_conditions({"items": [
            {"metadata": {"name": "cp1"}, "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "addresses": [{"type": "Hostname", "address": "cp1"},
                              {"type": "InternalIP", "address": "172.31.1.21"}]}},
            {"metadata": {"name": "w1"}, "status": {
                "conditions": [{"type": "Ready", "status": "Unknown"}], "addresses": []}},
        ]})
        self.assertEqual(nodes, {"cp1": (True, "172.31.1.21"), "w1": (False, None)})

    def test_first_vip_is_offset_eleven(self):
        self.assertEqual(first_vip("172.31.4.0/24"), "172.31.4.11")

    def test_cilium_addresses_follow_the_vips(self):
        self.assertEqual(host_address("172.31.4.0/24", GATEWAY_HOST_OFFSET), "172.31.4.13")
        self.assertEqual(host_address("172.31.4.0/24", HUBBLE_UI_HOST_OFFSET), "172.31.4.14")

    def test_gateway_route_targets_the_workload_service(self):
        route = gateway_route()
        self.assertEqual(route["spec"]["parentRefs"][0], {"name": "sherpa", "namespace": "default"})
        self.assertEqual(route["spec"]["rules"][0]["backendRefs"][0], {"name": WORKLOAD, "port": 80})

    def test_value_toml(self):
        self.assertEqual(value_toml({"a": "x", "b": [1, True]}), '{ a = "x", b = [1, true] }')
        with self.assertRaises(TypeError):
            value_toml(1.5)

    def test_workload_spreads_one_pod_per_node(self):
        deployment = workload_objects(3)["items"][1]
        self.assertEqual(deployment["spec"]["replicas"], 3)
        spread = deployment["spec"]["template"]["spec"]["topologySpreadConstraints"][0]
        self.assertEqual((spread["maxSkew"], spread["whenUnsatisfiable"]), (1, "DoNotSchedule"))

    def test_outcome_keeps_streams_apart_for_parsing(self):
        outcome = Outcome(0, '{"items": []}', "Warning: Permanently added host\n")
        self.assertEqual(json.loads(outcome.stdout), {"items": []})
        self.assertIn("Warning", outcome.output)

    def test_owner_only(self):
        path = RUNS / f"owner-only-{uuid.uuid4().hex}"
        RUNS.mkdir(parents=True, exist_ok=True)
        try:
            path.write_text("x")
            os.chmod(path, 0o600)
            self.assertTrue(owner_only(path))
            os.chmod(path, 0o644)
            self.assertFalse(owner_only(path))
        finally:
            path.unlink()


if __name__ == "__main__":
    sys.exit(main())
