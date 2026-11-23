# Talos Linux lab test

Tests Sherpa-built Talos Kubernetes clusters end to end through the `sherpa` CLI.

| Lab | Nodes |
|---|---|
| [`single`](single/manifest.toml) | 1 controller, 3 workers, util |
| [`ha`](ha/manifest.toml) | 3 controllers sharing a VIP, 2 workers, Keycloak `kc01`, Vault `vault01`, util; Cilium, Keycloak login (`oidc`) and Vault secrets (`vault`) |

The `util` node is Ubuntu with `talosctl`, `kubectl` and the kubelogin plugin
(`kubectl oidc-login`) installed by [`install-tools.sh`](install-tools.sh) (pinned versions,
SHA-256 verified). All cluster checks run from it, inside the lab, with the credentials
`sherpa up` writes. The Keycloak and Vault checks call their APIs from it with `curl`; passwords
and tokens go to it on stdin, so they are in no command line or log.

## Requirements

- A Sherpa server with the `talos_linux` `1.14.4`, `ubuntu_linux` `26.04`, `keycloak_server`
  `26.4` and `hashicorp_vault` `1.21` images, KVM, and internet access from labs (tool
  downloads, Kubernetes and workload images). The run fails early if the Talos image is missing.
- The `sherpa` CLI logged in, with a device password set (`sherpa server user device-password`), plus `ssh` and `scp`.
- About 44 GB of RAM: both labs run at the same time, `single` with four 4 GB Talos nodes and
  `ha` with five, each with the util node, and `ha` also with Keycloak and Vault, about 3 GB
  more. On a smaller server run
  the labs one at a time.

## Running

The runner, `talos_lab_test.py`, holds no cases. It runs the two lab modules, `single_lab_test.py`
and `ha_lab_test.py`, each in its own pytest process and both at the same time, in one run
directory. `uv run` installs the pytest version pinned in its script header; arguments after
the lab names are passed to pytest:

```sh
uv run tests/labs/talos-linux/talos_lab_test.py                 # both labs, in parallel
uv run tests/labs/talos-linux/talos_lab_test.py single          # one lab
uv run tests/labs/talos-linux/talos_lab_test.py --keep-failed
uv run tests/labs/talos-linux/talos_lab_test.py ha -k cluster_oidc_
uv run tests/labs/talos-linux/talos_lab_test.py --collect-only  # list the cases, in this process
uv run tests/labs/talos-linux/talos_lab_test.py --self-test     # runner unit tests, seconds
```

pytest runs a module directly too, for example `uvx --python 3.15 pytest==9.2.0
tests/labs/talos-linux/single_lab_test.py`. `--keep-failed` and `--run-root`, which the runner
uses to give both labs one run directory, are defined in `conftest.py`. What the two modules
share (the lab, its commands through the util node, the workload and the Keycloak and Vault
helpers) is in [`talos.py`](talos.py); what every lab runner shares is in
[`../labkit.py`](../labkit.py).

In each module the `prepared` fixture writes the run's manifest under a unique lab name and
checks the server; the `lab` fixture brings the lab up once, waits for the util node's tools
and copies the cluster's credentials to it, and destroys the lab at the end, even when a case
fails, unless `--keep-failed` is given and something failed. Destroying it checks that `sherpa
inspect` no longer finds the lab and that libvirt has no domain or network of it; that check
proves no case, and a leftover shows as an error in the lab fixture's teardown. Each lifecycle
step (a worker redeploy, a down, a resume, a stopped controller) is a fixture that does the
step and records what it sees; its case asserts on the record. The cases are the functions
named `test_<case_id>_*`, in the order below, with the cases that change the lab last; every
case runs even when one fails. The runner's own unit tests, under `--self-test`, are not
collected by pytest.

Ctrl-C reaches both pytest processes, and the runner passes SIGTERM on to them, so each lab is
still destroyed. Each run logs every command and its output under
`.tmp/lab-tests/talos-linux-<id>/<lab>/commands.log`. The run directory also holds the cluster
credentials, so keep it private. A run of both labs takes about 30–40 minutes.

## What it checks

`single` ([`single_lab_test.py`](single_lab_test.py)):

| Case | Check |
|---|---|
| `manifest_clusters_001` | `sherpa validate` of a cluster with two controllers exits 1 and says a cluster needs 1 or 3 |
| `cluster_bootstrap_001` | Every node is Ready, and `talosctl health` passes |
| `cluster_bootstrap_003` | A workload spread one pod per worker answers from every pod through a single node's NodePort, so requests cross nodes over Flannel |
| `lab_credentials_001` | `.sherpa/k8s/kubeconfig-k8s` and `.sherpa/talos/talosconfig-k8s` are readable only by the owner |
| `lab_credentials_002` | `sherpa download --force` restores both, deleted, byte for byte and readable only by the owner |
| `cluster_redeploy_001` | `sherpa redeploy --node cp1` is refused, naming cluster `k8s` |
| `cluster_redeploy_002` | After `sherpa redeploy --node w1` every node is Ready and `talosctl health` passes |
| `cluster_shutdown_001` | `sherpa down` succeeds and forces no node off |
| `cluster_shutdown_002` | After `sherpa resume` every node is Ready and `talosctl health` passes |

`ha` ([`ha_lab_test.py`](ha_lab_test.py)):

| Case | Check |
|---|---|
| `cluster_bootstrap_002` | Every node is Ready, and `talosctl health` passes |
| `cluster_credentials_001` | The kubeconfig's server is the cluster VIP, management offset `.11` |
| `cluster_cilium_001` | Cilium runs with no kube-proxy or Flannel daemonset, and its install Job succeeded |
| `cluster_cilium_002` | The workload, one pod on each of `w1` and `w2`, answers every request through the NodePort of each node, controllers included, and from both pods through each, so requests cross nodes over Cilium |
| `cluster_cilium_003` | `sherpa up` printed the Gateway (`.13`), and the workload answers there through the `sherpa` Gateway with an HTTPRoute, from the util node |
| `cluster_cilium_004` | `sherpa up` printed Hubble UI (`.14`), and it answers there from the util node |
| `cluster_oidc_001` | The lab owner, a `sherpa` realm user in `cluster-admins`, logs in with `kubectl oidc-login` (its password grant, non-interactively, with Sherpa's `oidc@k8s` settings), is `oidc:<owner>` in `oidc:cluster-admins`, and lists the nodes and every namespace's pods |
| `cluster_oidc_002` | A user the `oidc_logins` fixture adds through Keycloak's admin API, in no group, logs in as `oidc:<user>` and is refused as forbidden |
| `cluster_vault_001` | Once the Vault Secrets Operator is installed, a secret written under `secret/k8s/` is synced by a `VaultStaticSecret` into a Kubernetes Secret, which follows when the Vault value changes |
| `lab_credentials_003` | `.sherpa/keycloak/keycloak-kc01.toml` and `.sherpa/vault/vault-vault01.toml` are there, readable only by the owner |
| `cluster_addresses_001` | With cp1 stopped by `sherpa down --node`, the API through the VIP reports it not Ready and answers `/readyz`; after `sherpa resume --node` every node is Ready and `talosctl health` passes |

The workload is deployed on `ha` too, by the `workload` fixture, one pod per worker, as the
controllers are tainted against workloads: `cluster_cilium_002` sends requests through its
NodePort, `cluster_cilium_003` routes to it, and `cluster_vault_001` syncs the secret into its
namespace.

The lab uses `sherpa down --node` and `sherpa resume --node` but does not check that they leave
the other nodes alone (`node_down_006`, `node_resume_005`). It does not show `talos_node_003`,
`cluster_secrets_005`, `keycloak_realm_001` or `vault_init_012` to `014`.

## Running a lab by hand

`sherpa up` works directly in `single/` or `ha/`, because `startup_scripts` paths resolve from
the manifest's folder. Destroy the lab afterwards and delete the files `sherpa up` wrote there;
the runner never writes in this folder.
