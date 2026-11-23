"""What the Talos lab modules share: the lab, its commands and the pure helpers its cases use.

`single_lab_test.py` and `ha_lab_test.py` each bring one lab of this folder up with a `Lab` and
assert on what its methods return; `talos_lab_test.py` runs them and holds their unit tests.
"""

import base64
import copy
import ipaddress
import json
from pathlib import Path
import secrets
import shlex
import shutil
import stat
import sys
import time
import tomllib
from typing import NamedTuple
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import (
    COMMAND_TIMEOUT,
    TestFailure,
    require,
    require_device_password,
    run_logged,
    value_toml,
)


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

UP_TIMEOUT = 1800
HEALTH_TIMEOUT = "10m"
READY_TIMEOUT = 600
# Talos VIP failover after an unclean loss of the holder takes up to about a minute.
VIP_FAILOVER_TIMEOUT = 300
POLL_INTERVAL = 10
ECHO_REQUESTS = 60
READYZ_ATTEMPTS = 3
# Keycloak login (`oidc` on the cluster): the realm Sherpa makes, its admin group, and the user
# the test adds outside that group. The `master` realm admin is the credentials file's `[admin]`
# table, apart from the realm users.
KEYCLOAK_REALM = "sherpa"
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


class Workload(NamedTuple):
    """The workload spread one pod per node, once its rollout is done."""

    replicas: int
    pods: set
    nodes: set
    node_port: int


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
    """The lab owner from a Keycloak credentials file's users: a new node's only realm user."""
    require(len(users) == 1, f"Keycloak credentials list {len(users)} realm users, not the owner alone")
    return users[0]


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


def require_none(problems, stage):
    """Fail with every problem found at a stage, if there are any."""
    require(not problems, f"{stage}: {'; '.join(problems)}")


def new_run_root():
    """A fresh directory for one run of the Talos labs."""
    run_root = RUNS / f"talos-linux-{uuid.uuid4().hex[:12]}"
    run_root.mkdir(parents=True)
    return run_root


def keep_failed_lab(request):
    """Whether the lab is to be left running: `--keep-failed` is given and a case has failed."""
    return request.config.getoption("keep_failed") and request.session.testsfailed > 0


def preflight(directory):
    """Stop unless the tools are there, the CLI is logged in with a device password, and the server
    has the Talos image."""
    for tool in ("sherpa", "ssh", "scp"):
        require(shutil.which(tool), f"{tool} is not on PATH")
    with (directory / "preflight.log").open("w") as log:
        whoami = run_logged(log, "sherpa whoami", ["sherpa", "whoami"], directory)
        require(whoami.code == 0, "the sherpa CLI is not logged in (run `sherpa login`)")
        require_device_password(log, directory)
        image = run_logged(log, "talos image present", ["sherpa", "image", "show", "talos_linux",
                                                        "--version", TALOS_VERSION], directory)
        require(image.code == 0, f"the server has no talos_linux {TALOS_VERSION} image; pull it with "
                f"`sherpa server image pull --model talos_linux --version {TALOS_VERSION}`")


# -- the lab -------------------------------------------------------------------------------------


class Lab:
    """One lab of this folder: its manifest written by `prepare`, brought up once by `up` and
    destroyed by `destroy`, from its module's `lab` fixture."""

    def __init__(self, lab, run_root):
        self.lab = lab
        self.manifest = tomllib.loads((LABS / lab / "manifest.toml").read_text())
        cluster = lab_cluster(self.manifest)
        self.controllers = cluster["controllers"]
        self.workers = cluster.get("workers", [])
        self.oidc_node = cluster.get("oidc")
        self.vault_node = cluster.get("vault")
        self.name = f"{self.manifest['name']}-{run_root.name[-6:]}"
        self.directory = run_root / lab
        self.started = False
        self.destroyed = False
        self.lab_id = None
        self.ipv4_network = None
        self.up_output = ""
        self.log = None

    # -- commands ----------------------------------------------------------------------------

    def command(self, label, arguments, timeout=COMMAND_TIMEOUT, check=True, stdin=None):
        outcome = run_logged(self.log, label, arguments, self.directory, timeout, stdin=stdin)
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

    # -- lifecycle ---------------------------------------------------------------------------

    def prepare(self):
        self.directory.mkdir(parents=True)
        self.log = (self.directory / "commands.log").open("w")
        (self.directory / "manifest.toml").write_text(render_manifest(self.manifest, self.name, TOOLS_SCRIPT))

    def up(self):
        """Bring the lab up, wait for the util node's tools and give it the cluster's credentials."""
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
        for name in (KUBECONFIG, TALOSCONFIG):
            require((self.directory / name).is_file(), f"sherpa up did not write {name}")
        self.wait_for_util()
        self.util("util tooling installed", "cloud-init status --wait && talosctl version --client "
                  "&& kubectl version --client && kubectl oidc-login --version", timeout=900)
        self.command("copy credentials to util", ["scp", *SSH_OPTIONS, KUBECONFIG, TALOSCONFIG,
                                                  f"{UTIL_NODE}.{self.lab_id}:"])
        # Flush the copies so they survive a forced power-off if the guest ignores `sherpa down`,
        # and a down and resume of the util node.
        self.util("flush credentials to disk", "sync")

    def wait_for_util(self, timeout=READY_TIMEOUT):
        deadline = time.monotonic() + timeout
        while self.util("util reachable", "true", check=False).code != 0:
            require(time.monotonic() < deadline, f"util node not reachable over SSH after {timeout}s")
            time.sleep(POLL_INTERVAL)

    def destroy(self):
        """Destroy the lab and check `sherpa inspect` no longer finds it and libvirt has no domain or
        network of it. Does nothing when the lab was never started or is already destroyed."""
        if not self.started or self.destroyed:
            return
        if self.lab_id is None:
            # `sherpa up` failed early; destroy whatever it created, found by the manifest's lab name.
            self.sherpa("sherpa destroy (after failed up)", ["destroy", "--yes"], UP_TIMEOUT, check=False)
            self.destroyed = True
            return
        self.sherpa("sherpa destroy", ["destroy", "--yes"], UP_TIMEOUT)
        self.destroyed = True
        inspect = self.sherpa("lab gone", ["inspect"], check=False)
        require(inspect.code != 0, f"lab {self.lab_id} still exists after destroy")
        self.no_libvirt_leftovers()

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

    # -- the cluster -------------------------------------------------------------------------

    def nodes(self):
        outcome = self.kubectl("kubectl get nodes", ["get", "nodes", "-o", "json"], check=False)
        try:
            return node_conditions(json.loads(outcome.stdout))
        except (json.JSONDecodeError, KeyError):
            return {}

    def wait_for_ready_nodes(self, timeout=READY_TIMEOUT):
        """Every member's Kubernetes node, with the members not Ready when the wait ended."""
        expected = self.controllers + self.workers
        deadline = time.monotonic() + timeout
        while True:
            nodes = self.nodes()
            not_ready = [name for name in expected if not nodes.get(name, (False, None))[0]]
            if not not_ready or time.monotonic() >= deadline:
                return nodes, not_ready
            time.sleep(POLL_INTERVAL)

    def health_problems(self):
        """Why the cluster is not healthy: members not Ready, or `talosctl health` failing; empty
        when every member is Ready and `talosctl health` passes."""
        nodes, not_ready = self.wait_for_ready_nodes()
        if not_ready:
            return [f"nodes not Ready after {READY_TIMEOUT}s: {not_ready}"]
        controller_ips = [nodes[name][1] for name in self.controllers]
        worker_ips = [nodes[name][1] for name in self.workers]
        arguments = ["--nodes", controller_ips[0], "health", "--wait-timeout", HEALTH_TIMEOUT,
                     "--control-plane-nodes", ",".join(controller_ips)]
        if worker_ips:
            arguments += ["--worker-nodes", ",".join(worker_ips)]
        health = self.talosctl("talosctl health", arguments, timeout=900, check=False)
        if health.code != 0:
            return [f"talosctl health failed (exit {health.code}); see {self.log.name}"]
        return []

    def deploy_workload(self):
        """Apply the workload, one pod per worker (per controller when there are none), and wait
        for its rollout."""
        replicas = len(self.workers) or len(self.controllers)
        self.kubectl("apply workload", ["apply", "-f", "-"], stdin=json.dumps(workload_objects(replicas)))
        self.kubectl("workload rollout", ["-n", NAMESPACE, "rollout", "status",
                                          f"deployment/{WORKLOAD}", "--timeout=5m"], timeout=360)
        pods = self.kubectl("workload pods", ["-n", NAMESPACE, "get", "pods", "-l",
                                              f"app={WORKLOAD}", "-o", "json"])
        pods = json.loads(pods.stdout)["items"]
        service = self.kubectl("workload service", ["-n", NAMESPACE, "get", "service", WORKLOAD, "-o", "json"])
        return Workload(replicas=replicas,
                        pods={pod["metadata"]["name"] for pod in pods},
                        nodes={pod["spec"]["nodeName"] for pod in pods},
                        node_port=json.loads(service.stdout)["spec"]["ports"][0]["nodePort"])

    def node_port_problems(self, workload, entry_nodes):
        """Why the workload's pods do not all answer through the NodePort of each of the entry
        nodes: pods sharing a node, so no request need cross nodes, requests through a node not
        answered, or pods that never answered through a node."""
        if len(workload.nodes) != workload.replicas:
            return [f"workload pods share nodes: {sorted(workload.nodes)}"]
        nodes = self.nodes()
        problems = []
        for entry in entry_nodes:
            if entry not in nodes:
                problems.append(f"{entry} is not a Kubernetes node")
                continue
            address = f"{nodes[entry][1]}:{workload.node_port}"
            loop = (f"for i in $(seq {ECHO_REQUESTS}); do curl -fsS --max-time 5 "
                    f"http://{address}/hostname; echo; done")
            replies = self.util(f"requests through {entry}'s NodePort", loop, timeout=600, check=False)
            answers = [line.strip() for line in replies.stdout.splitlines() if line.strip()]
            if len(answers) != ECHO_REQUESTS:
                problems.append(f"{len(answers)} of {ECHO_REQUESTS} requests through {entry} ({address}) "
                                "answered")
            missing = workload.pods - set(answers)
            if missing:
                problems.append(f"pods never answered through {entry} ({address}): {sorted(missing)}")
        return problems

    def answers(self, label, url):
        """Whether the URL answers from the util node within a few minutes."""
        return self.util(label, f"for i in $(seq 30); do curl -fsS -o /dev/null --max-time 5 {url} "
                         f"&& exit 0; sleep {POLL_INTERVAL}; done; exit 1", timeout=600, check=False).code == 0

    def kubeconfig(self):
        return json.loads((self.directory / KUBECONFIG).read_text())

    # -- Keycloak login ----------------------------------------------------------------------

    def oidc_kubectl(self, username, arguments, check=True):
        return self.util(f"kubectl as {username}: {' '.join(arguments[:2])}",
                         shlex.join(["kubectl", "--kubeconfig", f"oidc-{username}.kubeconfig", *arguments]),
                         check=check)

    def write_oidc_kubeconfig(self, username, password):
        """Give the util node a kubeconfig that logs in as the user through `kubectl oidc-login`."""
        document = password_grant_kubeconfig(self.kubeconfig(), CLUSTER, username, password)
        # On stdin, so the password is in no command line or log.
        self.util(f"kubeconfig for {username}", f"umask 077 && cat > oidc-{username}.kubeconfig",
                  stdin=json.dumps(document))

    def add_outsider(self, keycloak, admin):
        """Add a realm user in no group through Keycloak's admin API, from the util node, and return
        its password."""
        password = secrets.token_urlsafe(18)
        # Line one of stdin is the admin password, the rest the user; neither is in the log.
        script = (
            "read -r admin && user=$(cat) && "
            "token=$(curl -fsS {token_url} -d grant_type=password -d client_id=admin-cli "
            "-d username={admin} --data-urlencode password=\"$admin\" "
            "| python3 -c 'import json, sys; print(json.load(sys.stdin)[\"access_token\"])') && "
            "curl -sS -o /dev/null -w '%{{http_code}}' -X POST {users_url} "
            "-H \"Authorization: Bearer $token\" -H 'Content-Type: application/json' -d \"$user\""
        ).format(token_url=shlex.quote(f"{keycloak}/realms/master/protocol/openid-connect/token"),
                 admin=shlex.quote(admin["username"]),
                 users_url=shlex.quote(f"{keycloak}/admin/realms/{KEYCLOAK_REALM}/users"))
        created = self.util("keycloak: add a user outside cluster-admins", script,
                            stdin=f"{admin['password']}\n{json.dumps(outsider_user(password))}")
        require(created.stdout.strip() == "201", f"Keycloak user create answered {created.stdout.strip()!r}")
        return password

    # -- Vault secrets -----------------------------------------------------------------------

    def write_vault_secret(self, credentials, value):
        url = f"{credentials['address']}/v1/secret/data/{CLUSTER}/{NAMESPACE}"
        # Line one of stdin is the root token, the rest the secret; neither is in the log. The
        # util node trusts the lab CA that Vault's certificate chains to.
        script = (f"read -r token && curl -fsS -o /dev/null -X POST {shlex.quote(url)} "
                  "-H \"X-Vault-Token: $token\" -H 'Content-Type: application/json' --data @-")
        body = json.dumps({"data": {VAULT_SECRET_KEY: value}})
        self.util("vault: write the test secret", script, stdin=f"{credentials['root_token']}\n{body}")

    def wait_for_secret_value(self, expected, timeout=VAULT_SYNC_TIMEOUT):
        """Whether the synced Kubernetes Secret gets the value within the timeout."""
        deadline = time.monotonic() + timeout
        while True:
            secret = self.kubectl("synced secret", ["-n", NAMESPACE, "get", "secret", VAULT_SECRET, "-o", "json"],
                                  check=False)
            value = secret_value(json.loads(secret.stdout), VAULT_SECRET_KEY) if secret.code == 0 else None
            if value == expected:
                return True
            if time.monotonic() >= deadline:
                return False
            time.sleep(POLL_INTERVAL)
