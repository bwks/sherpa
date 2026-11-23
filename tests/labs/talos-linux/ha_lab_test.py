"""End-to-end test of the `ha` Talos lab through the sherpa CLI (Python 3.15+, pytest).

Three controllers sharing a VIP and one worker, with Cilium, a Keycloak node users log in
through and a Vault node workloads read secrets from, and a util node that runs `kubectl`,
`talosctl` and `curl` inside the lab. Checks the `ha` cases of tests/specs/talos.md, with the
Keycloak and Vault credential files from tests/specs/client-lab-files.md. See README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, and the
`lab` fixture brings the lab up once for the module and destroys it at the end, checking nothing
of it is left. The `workload` fixture deploys the workload the Gateway and the Vault secret use,
and `oidc_logins` gives the util node a kubeconfig for each Keycloak user. Losing a controller
is a fixture that stops one, records what the API answers, resumes it and records the cluster's
health, without asserting anything; its case asserts on that record. The cases are the
`test_<case_id>_*` functions, in case order, the controller loss last. `talos_lab_test.py` runs
this module and `single_lab_test.py` at the same time.
"""

import json
from pathlib import Path
import secrets
import signal
import sys
import time
import tomllib
from typing import NamedTuple

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import require, stop_on_sigterm
from talos import (
    CLUSTER_ADMINS_GROUP,
    FIRST_VIP_HOST_OFFSET,
    GATEWAY_HOST_OFFSET,
    HUBBLE_UI_HOST_OFFSET,
    OIDC_CLAIM_PREFIX,
    OUTSIDER,
    POLL_INTERVAL,
    READYZ_ATTEMPTS,
    UP_TIMEOUT,
    VAULT_SECRET,
    VAULT_SYNC_TIMEOUT,
    VIP_FAILOVER_TIMEOUT,
    VSO_INSTALL_JOB,
    VSO_NAMESPACE,
    Lab,
    gateway_route,
    host_address,
    keep_failed_lab,
    keycloak_credentials_path,
    new_run_root,
    owner_only,
    preflight,
    realm_member,
    require_none,
    vault_credentials_path,
    vault_static_secret,
)


LAB = "ha"


class ControllerLoss(NamedTuple):
    """What the cluster did with one controller stopped and then resumed."""

    stopped: str
    # Whether the API, through the VIP, reported the stopped controller as not Ready in time.
    seen_not_ready: bool
    readyz_failures: list
    health: list


# -- the fixtures --------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def prepared(request):
    """The lab before it is up: the run's manifest written and the server checked."""
    signal.signal(signal.SIGTERM, stop_on_sigterm)
    run_root = request.config.getoption("run_root") or new_run_root()
    print(f"Logs: {run_root / LAB}", flush=True)
    lab = Lab(LAB, run_root)
    lab.prepare()
    try:
        preflight(lab.directory)
        yield lab
    finally:
        lab.log.close()


@pytest.fixture(scope="module")
def lab(request, prepared):
    """The lab every case uses: brought up once and destroyed at the end, even when a case fails,
    unless `--keep-failed` is given and something failed. Destroying it checks that `sherpa
    inspect` no longer finds it and that libvirt has no domain or network of it."""
    keep = False
    try:
        try:
            prepared.up()
        except Exception:
            keep = request.config.getoption("keep_failed")
            raise
        yield prepared
        keep = keep_failed_lab(request)
    finally:
        if keep:
            print(f"keeping failed lab {prepared.name} ({prepared.lab_id}) in {prepared.directory}", flush=True)
        else:
            prepared.destroy()


@pytest.fixture(scope="module")
def workload(lab):
    """The workload and its namespace, which the Gateway's route and the Vault secret use."""
    return lab.deploy_workload()


@pytest.fixture(scope="module")
def oidc_logins(lab):
    """The util node given a kubeconfig that logs in through Keycloak for the lab's owner, a realm
    user in cluster-admins, and for a user the fixture adds in no group. Returns the owner's name."""
    credentials = tomllib.loads((lab.directory / keycloak_credentials_path(lab.oidc_node)).read_text())
    require("admin" in credentials, "Keycloak credentials have no [admin], the master realm admin")
    member = realm_member(credentials["users"])
    outsider_password = lab.add_outsider(credentials["address"], credentials["admin"])
    lab.write_oidc_kubeconfig(member["username"], member["password"])
    lab.write_oidc_kubeconfig(OUTSIDER, outsider_password)
    return member["username"]


@pytest.fixture(scope="module")
def controller_lost(lab):
    """The first controller stopped with `sherpa down --node`, the API's answers through the VIP
    recorded, then the controller resumed and the cluster's health recorded."""
    stopped = lab.controllers[0]
    lab.sherpa(f"sherpa down {stopped}", ["down", "--node", stopped], UP_TIMEOUT)
    # The VIP moves to another controller once the stopped holder's lease expires, which Talos
    # documents as taking up to about a minute. Wait for the cluster to see the stopped
    # controller as not Ready through the VIP, then ask the API a few times.
    deadline = time.monotonic() + VIP_FAILOVER_TIMEOUT
    seen_not_ready = False
    while time.monotonic() < deadline:
        nodes = lab.nodes()
        if stopped in nodes and not nodes[stopped][0]:
            seen_not_ready = True
            break
        time.sleep(POLL_INTERVAL)
    readyz_failures = []
    if seen_not_ready:
        for attempt in range(1, READYZ_ATTEMPTS + 1):
            readyz = lab.kubectl(f"API through the VIP without {stopped} ({attempt}/{READYZ_ATTEMPTS})",
                                 ["get", "--raw", "/readyz"], check=False)
            if readyz.code != 0:
                readyz_failures.append(f"/readyz attempt {attempt} exited {readyz.code}")
            time.sleep(POLL_INTERVAL)
    lab.sherpa(f"sherpa resume {stopped}", ["resume", "--node", stopped], UP_TIMEOUT)
    return ControllerLoss(stopped, seen_not_ready, readyz_failures, lab.health_problems())


# -- cases ---------------------------------------------------------------------------------------


def test_cluster_bootstrap_002_three_controllers_and_a_worker_with_cilium_keycloak_and_vault_come_up_healthy(lab):
    require_none(lab.health_problems(), "after up")


def test_cluster_credentials_001_the_kubeconfig_of_three_controllers_names_the_shared_address(lab):
    server = lab.kubeconfig()["clusters"][0]["cluster"]["server"]
    expected = f"https://{host_address(lab.ipv4_network, FIRST_VIP_HOST_OFFSET)}:6443"
    require(server == expected, f"kubeconfig server is {server}, expected the VIP {expected}")


def test_cluster_cilium_001_cilium_runs_with_no_kube_proxy_or_flannel_and_its_install_job_succeeded(lab):
    daemonsets = lab.kubectl("kube-system daemonsets", ["-n", "kube-system", "get", "daemonsets",
                                                        "-o", "jsonpath={.items[*].metadata.name}"])
    names = daemonsets.stdout.split()
    job = lab.kubectl("cilium install job", ["-n", "kube-system", "get", "job", "cilium-install",
                                             "-o", "jsonpath={.status.succeeded}"], check=False)
    problems = []
    if "cilium" not in names:
        problems.append(f"no cilium daemonset: {names}")
    if "kube-proxy" in names or "kube-flannel" in names:
        problems.append(f"kube-proxy or Flannel deployed alongside Cilium: {names}")
    if job.stdout.strip() != "1":
        problems.append(f"cilium-install job did not succeed: {job.stdout.strip()!r}")
    require_none(problems, "after up")


def test_cluster_cilium_003_up_prints_the_gateway_and_a_workload_with_an_httproute_answers_there(lab, workload):
    gateway = host_address(lab.ipv4_network, GATEWAY_HOST_OFFSET)
    line = f"Gateway: http://{gateway}"
    require(line in lab.up_output, f"sherpa up did not print {line!r}")
    lab.kubectl("apply gateway route", ["apply", "-f", "-"], stdin=json.dumps(gateway_route()))
    # The route is programmed into the Gateway's Envoy a few seconds after it is accepted.
    require(lab.answers("workload through the gateway", f"http://{gateway}/hostname"),
            f"the workload did not answer through the Gateway at {gateway}")


def test_cluster_cilium_004_up_prints_hubble_ui_and_it_answers_there(lab):
    hubble_ui = host_address(lab.ipv4_network, HUBBLE_UI_HOST_OFFSET)
    line = f"Hubble UI: http://{hubble_ui}"
    require(line in lab.up_output, f"sherpa up did not print {line!r}")
    require(lab.answers("hubble ui", f"http://{hubble_ui}/"), f"Hubble UI did not answer at {hubble_ui}")


def test_cluster_oidc_001_the_owner_logs_in_through_keycloak_as_a_cluster_admin(lab, oidc_logins):
    member = oidc_logins
    whoami = lab.oidc_kubectl(member, ["auth", "whoami", "-o", "json"])
    status = json.loads(whoami.stdout)["status"]["userInfo"]
    problems = []
    if status["username"] != f"{OIDC_CLAIM_PREFIX}{member}":
        problems.append(f"logged in as {status['username']}, not {OIDC_CLAIM_PREFIX}{member}")
    if f"{OIDC_CLAIM_PREFIX}{CLUSTER_ADMINS_GROUP}" not in status.get("groups", []):
        problems.append(f"not in {OIDC_CLAIM_PREFIX}{CLUSTER_ADMINS_GROUP}: {status.get('groups')}")
    nodes = lab.oidc_kubectl(member, ["get", "nodes", "-o", "json"])
    names = {item["metadata"]["name"] for item in json.loads(nodes.stdout)["items"]}
    if names != set(lab.controllers + lab.workers):
        problems.append(f"saw nodes {sorted(names)}")
    if lab.oidc_kubectl(member, ["get", "pods", "--all-namespaces"], check=False).code != 0:
        problems.append("could not list every namespace's pods")
    require_none(problems, f"Keycloak login of {member}")


def test_cluster_oidc_002_a_realm_user_in_no_group_logs_in_and_is_refused_as_forbidden(lab, oidc_logins):
    whoami = lab.oidc_kubectl(OUTSIDER, ["auth", "whoami", "-o", "json"])
    username = json.loads(whoami.stdout)["status"]["userInfo"]["username"]
    require(username == f"{OIDC_CLAIM_PREFIX}{OUTSIDER}", f"{OUTSIDER} logged in as {username}")
    refused = lab.oidc_kubectl(OUTSIDER, ["get", "pods", "--all-namespaces"], check=False)
    require(refused.code != 0 and "forbidden" in refused.output.lower(),
            f"a user outside {CLUSTER_ADMINS_GROUP} was not refused as forbidden (exit {refused.code})")


def test_cluster_vault_001_a_secret_in_vault_is_synced_into_a_kubernetes_secret_and_follows_changes(lab, workload):
    credentials = tomllib.loads((lab.directory / vault_credentials_path(lab.vault_node)).read_text())
    # The operator is installed by a Job a couple of minutes after the cluster starts.
    lab.kubectl("vault secrets operator installed", ["-n", VSO_NAMESPACE, "wait", "--for=condition=Complete",
                                                     f"job/{VSO_INSTALL_JOB}", "--timeout=10m"], timeout=660)
    lab.kubectl("vault secrets operator available", ["-n", VSO_NAMESPACE, "wait", "--for=condition=Available",
                                                     "deployment", "--all", "--timeout=5m"], timeout=360)
    first, second = secrets.token_hex(8), secrets.token_hex(8)
    lab.write_vault_secret(credentials, first)
    lab.kubectl("apply vault static secret", ["apply", "-f", "-"], stdin=json.dumps(vault_static_secret()))
    require(lab.wait_for_secret_value(first),
            f"Secret {VAULT_SECRET} did not get the Vault value within {VAULT_SYNC_TIMEOUT}s")
    lab.write_vault_secret(credentials, second)
    require(lab.wait_for_secret_value(second),
            f"Secret {VAULT_SECRET} did not follow the changed Vault value within {VAULT_SYNC_TIMEOUT}s")


def test_lab_credentials_003_the_keycloak_and_vault_credential_files_are_saved_readable_by_the_user_alone(lab):
    problems = []
    for name in (keycloak_credentials_path(lab.oidc_node), vault_credentials_path(lab.vault_node)):
        path = lab.directory / name
        if not path.is_file():
            problems.append(f"sherpa up did not write {name}")
        elif not owner_only(path):
            problems.append(f"{name} is not readable only by its owner")
    require_none(problems, "after up")


def test_cluster_addresses_001_with_a_controller_stopped_the_api_answers_through_the_shared_address(controller_lost):
    require(controller_lost.seen_not_ready,
            f"the API through the VIP did not report {controller_lost.stopped} as not Ready within "
            f"{VIP_FAILOVER_TIMEOUT}s")
    require_none(controller_lost.readyz_failures, f"through the VIP without {controller_lost.stopped}")
    require_none(controller_lost.health, f"after resuming {controller_lost.stopped}")
