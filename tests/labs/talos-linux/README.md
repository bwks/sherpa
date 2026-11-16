# Talos Linux lab test

Tests Sherpa-built Talos Kubernetes clusters end to end through the `sherpa` CLI.

| Lab | Nodes |
|---|---|
| [`single`](single/manifest.toml) | 1 controller, 3 workers, util |
| [`ha`](ha/manifest.toml) | 3 controllers sharing a VIP, 1 worker, Keycloak `kc01`, Vault `vault01`, util; Cilium, Keycloak login (`oidc`) and Vault secrets (`vault`) |

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
- About 40 GB of RAM: both labs run at the same time, each with four 4 GB Talos nodes and the
  util node, and `ha` also with Keycloak and Vault, about 3 GB more. On a smaller server run
  the labs one at a time.

## Running

```sh
uv run tests/labs/talos-linux/talos_lab_test.py              # both labs, in parallel
uv run tests/labs/talos-linux/talos_lab_test.py single       # one lab
uv run tests/labs/talos-linux/talos_lab_test.py --keep-failed
uv run tests/labs/talos-linux/talos_lab_test.py --self-test  # runner unit tests, seconds
```

Each lab runs in its own process, and Ctrl-C or SIGTERM still destroys every lab. Each run logs
every command and its output under `.tmp/lab-tests/talos-linux-<id>/<lab>/commands.log`. The run
directory also holds the cluster credentials, so keep it private.

## What it checks

Before the labs:

- the server has the `talos_linux` 1.14.4 image;
- `sherpa validate` refuses a cluster with two controllers.

Both labs:

- `sherpa up` writes `.sherpa/k8s/kubeconfig-k8s` and `.sherpa/talos/talosconfig-k8s`, readable
  only by the owner;
- every node is Ready, and `talosctl health` passes;
- a workload spread one pod per worker answers through a single node's NodePort from every pod,
  which proves pod networking across nodes;
- `sherpa destroy` leaves no lab, libvirt domain or network behind.

`single` only:

- `sherpa redeploy` is refused for the controller; a worker redeploy rejoins and the cluster is
  healthy;
- `sherpa down` shuts every node down cleanly (none forced off), and `sherpa resume` brings
  the cluster back healthy;
- `sherpa download` restores deleted credential files unchanged.

`ha` only:

- the kubeconfig server is the cluster VIP (management offset `.11`);
- with one controller stopped, the API still answers through the VIP; after `sherpa resume`
  the cluster is healthy again;
- Cilium: `sherpa up` prints the Gateway (`.13`) and Hubble UI (`.14`) addresses; Cilium runs
  without kube-proxy or Flannel, and its install Job succeeded; the workload answers through
  the `sherpa` Gateway with an HTTPRoute, and Hubble UI answers, from the util node.
- `sherpa up` writes `.sherpa/keycloak/keycloak-kc01.toml` and `.sherpa/vault/vault-vault01.toml`,
  readable only by the owner.
- Keycloak login: the lab owner, a `sherpa` realm user in `cluster-admins`, logs in with
  `kubectl` through `kubectl oidc-login`, non-interactively with the plugin's password grant
  and Sherpa's `oidc@k8s` settings. `kubectl auth whoami` shows `oidc:<owner>` in
  `oidc:cluster-admins`, and the user lists the nodes and every namespace's pods. A user the
  test adds through Keycloak's admin API, in no group, logs in as `oidc:<user>` and is refused
  (`Forbidden`).
- Vault secrets: once the Vault Secrets Operator is installed, a secret written under
  `secret/k8s/` is synced by a `VaultStaticSecret` into a Kubernetes Secret, and the Secret
  changes when the Vault value changes.

## Running a lab by hand

`sherpa up` works directly in `single/` or `ha/`, because `startup_scripts` paths resolve from
the manifest's folder. Destroy the lab afterwards and delete the files `sherpa up` wrote there;
the runner never writes in this folder.
