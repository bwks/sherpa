# Talos Linux lab test

Tests Sherpa-built Talos Kubernetes clusters end to end through the `sherpa` CLI.

| Lab | Nodes |
|---|---|
| [`single`](single/manifest.toml) | 1 controller, 3 workers, util |
| [`ha`](ha/manifest.toml) | 3 controllers sharing a VIP, 1 worker, util |

The `util` node is Ubuntu with `talosctl` and `kubectl` installed by
[`install-tools.sh`](install-tools.sh) (pinned versions, SHA-256 verified). All cluster checks
run from it, inside the lab, with the credentials `sherpa up` writes.

## Requirements

- A Sherpa server with the `talos_linux` `1.14.4` and `ubuntu_linux` `26.04` images, KVM, and
  internet access from labs (tool downloads, Kubernetes and workload images). The run fails
  early if the Talos image is missing.
- The `sherpa` CLI logged in, plus `ssh` and `scp`.
- About 40 GB of RAM: both labs run at the same time, each with four 4 GB Talos nodes and the
  util node.

## Running

```sh
python3 tests/labs/talos-linux/talos_lab_test.py              # both labs, in parallel
python3 tests/labs/talos-linux/talos_lab_test.py single       # one lab
python3 tests/labs/talos-linux/talos_lab_test.py --keep-failed
python3 tests/labs/talos-linux/talos_lab_test.py --self-test  # runner unit tests, seconds
```

Each lab runs in its own process, and Ctrl-C or SIGTERM still destroys every lab. Each run logs
every command and its output under `.tmp/lab-tests/talos-linux-<id>/<lab>/commands.log`. The run
directory also holds the cluster credentials, so keep it private.

## What it checks

Before the labs:

- the server has the `talos_linux` 1.14.4 image;
- `sherpa validate` refuses a cluster with two controllers.

Both labs:

- `sherpa up` writes `kubeconfig-k8s` and `talosconfig-k8s`, readable only by the owner;
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
  the cluster is healthy again.

## Running a lab by hand

`sherpa up` works directly in `single/` or `ha/`, because `startup_scripts` paths resolve from
the manifest's folder. Destroy the lab afterwards and delete the files `sherpa up` wrote there;
the runner never writes in this folder.
