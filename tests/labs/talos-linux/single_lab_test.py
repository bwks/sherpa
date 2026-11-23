"""End-to-end test of the `single` Talos lab through the sherpa CLI (Python 3.15+, pytest).

One controller and three workers on Flannel, and a util node that runs `kubectl` and `talosctl`
inside the lab. Checks the `single` cases of tests/specs/talos.md, with `sherpa validate`
refusing a two-controller cluster from tests/specs/manifests.md and the cluster's credential
files from tests/specs/client-lab-files.md. See README.md.

A pytest module: the `prepared` fixture checks the server and writes the lab's manifest, and the
`lab` fixture brings the lab up once for the module and destroys it at the end, checking nothing
of it is left. Each lifecycle step (a worker redeploy, a down, a resume) is a fixture that does
the step and records what it sees, without asserting anything; the cases assert on those
records. The cases are the `test_<case_id>_*` functions, in case order, the lifecycle cases
last. `talos_lab_test.py` runs this module and `ha_lab_test.py` at the same time.
"""

from pathlib import Path
import signal
import sys

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from labkit import require, run_logged, stop_on_sigterm
from talos import (
    CLUSTER,
    KUBECONFIG,
    TALOSCONFIG,
    UP_TIMEOUT,
    Lab,
    keep_failed_lab,
    new_run_root,
    owner_only,
    preflight,
    require_none,
)


LAB = "single"
TWO_CONTROLLERS = """name = "two-controllers"
talos_clusters = [{ name = "k8s", controllers = ["cp1", "cp2"] }]
nodes = [
  { name = "cp1", model = "talos_linux" },
  { name = "cp2", model = "talos_linux" },
]
"""


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
def worker_redeployed(lab):
    """The first worker redeployed, and the cluster's health problems afterwards recorded."""
    worker = lab.workers[0]
    lab.sherpa(f"sherpa redeploy {worker}", ["redeploy", "--node", worker], UP_TIMEOUT)
    return lab.health_problems()


@pytest.fixture(scope="module")
def downed(lab):
    """The outcome of `sherpa down` of the whole lab."""
    return lab.sherpa("sherpa down", ["down"], UP_TIMEOUT, check=False)


@pytest.fixture(scope="module")
def resumed(lab, downed):
    """The lab resumed after the down, and the cluster's health problems afterwards recorded."""
    lab.sherpa("sherpa resume", ["resume"], UP_TIMEOUT)
    lab.wait_for_util()
    return lab.health_problems()


# -- cases ---------------------------------------------------------------------------------------


def test_manifest_clusters_001_validate_refuses_a_cluster_with_two_controllers(prepared):
    folder = prepared.directory.parent / "two-controllers"
    folder.mkdir()
    (folder / "manifest.toml").write_text(TWO_CONTROLLERS)
    validate = run_logged(prepared.log, "validate refuses two controllers", ["sherpa", "validate"], folder)
    require(validate.code == 1 and "needs 1 or 3" in validate.output,
            f"sherpa validate of a two-controller cluster exited {validate.code}, "
            f"expected 1 and 'needs 1 or 3'; see {prepared.log.name}")


def test_cluster_bootstrap_001_a_cluster_with_one_controller_comes_up_healthy(lab):
    require_none(lab.health_problems(), "after up")


def test_cluster_bootstrap_003_on_flannel_pods_on_different_workers_all_answer_through_one_nodes_port(lab):
    require_none(lab.node_port_problems(lab.deploy_workload(), lab.workers[:1]), "after up")


def test_lab_credentials_001_the_kubeconfig_and_talosconfig_are_saved_readable_by_the_user_alone(lab):
    problems = [f"{name} is not readable only by its owner" for name in (KUBECONFIG, TALOSCONFIG)
                if not owner_only(lab.directory / name)]
    require_none(problems, "after up")


def test_lab_credentials_002_download_puts_back_deleted_credential_files_byte_for_byte(lab):
    saved = {}
    for name in (KUBECONFIG, TALOSCONFIG):
        path = lab.directory / name
        saved[name] = path.read_text()
        path.unlink()
    lab.sherpa("sherpa download", ["download", lab.lab_id, "--force"])
    problems = []
    for name, content in saved.items():
        path = lab.directory / name
        if not path.is_file() or path.read_text() != content:
            problems.append(f"{name} not restored as it was")
        elif not owner_only(path):
            problems.append(f"{name} is not readable only by its owner")
    require_none(problems, "after sherpa download")


def test_cluster_redeploy_001_redeploying_a_controller_is_refused_naming_its_cluster(lab):
    controller = lab.controllers[0]
    refused = lab.sherpa(f"sherpa redeploy {controller} (refused)", ["redeploy", "--node", controller],
                         UP_TIMEOUT, check=False)
    require(refused.code != 0 and f"controller of talos cluster '{CLUSTER}'" in refused.output,
            f"redeploy of controller {controller} was not refused naming cluster {CLUSTER} "
            f"(exit {refused.code}); see {lab.log.name}")


def test_cluster_redeploy_002_a_redeployed_worker_joins_again_and_the_cluster_is_healthy(worker_redeployed):
    require_none(worker_redeployed, "after redeploying a worker")


def test_cluster_shutdown_001_down_shuts_every_member_down_and_none_is_forced_off(lab, downed):
    require(downed.code == 0, f"sherpa down failed (exit {downed.code}); see {lab.log.name}")
    require("forced" not in downed.output,
            f"sherpa down forced a node off instead of shutting it down; see {lab.log.name}")


def test_cluster_shutdown_002_after_resume_every_member_is_ready_and_the_cluster_is_healthy(resumed):
    require_none(resumed, "after down and resume")
