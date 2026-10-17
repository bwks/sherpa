# Talos Linux and Kubernetes clusters

Design: `docs/architecture/TALOS.md`.

## Node model

- `talos_linux` defaults: 2 vCPU, 4 GiB, NoCloud ZTP, 20 GB default boot disk, Talos API readiness port, no SSH `[unit]` **P0**
- A manifest `boot_disk_size` overrides the 20 GB default `[unit]` **P0**
- VM readiness uses the manifest `ready_port`, else the model port (50000 for Talos, 22 otherwise), in `up`, `resume` and `redeploy` `[unit]` **P0**
- Nodes without SSH are left out of the lab SSH config `[unit]` **P0**
- Talos is not a guest-account (`provision_current_user`) target `[unit]` **P1**
- `image pull` builds the NoCloud URL from the schematic ID returned by the Image Factory, with or without a leading `v` `[unit]`; the live lookup `[integration]` (needs internet) **P1**

## NoCloud delivery

- `user-data` is the node's config byte for byte; `meta-data` names the node; `network-config` is a static netplan v2 management interface `[unit]` **P0**
- Without a config, stale `user-data` is removed so the node waits in maintenance mode `[unit]` **P0**

## Clusters

- `talos_clusters` manifest shape: `controllers` required, `workers` optional, unknown fields and the old node-level `cluster`/`cluster_role` fields and `clusters` section rejected; role lookup and member order (controllers first, in bootstrap order) `[unit]` **P0**
- Manifest cluster rules: unique DNS-label names, one or three controllers (0, 2 and 4 refused), members are defined `talos_linux` nodes without `ztp_config`, each node listed once across all clusters, VIP capacity `[unit]` **P0**
- Plan: single control plane uses its address; HA clusters get VIPs `.11`–`.12` in manifest order `[unit]` **P0**
- Secrets: PKCS#8 keys of the right types, token formats, owner-only storage and round trip `[unit]` **P0**
- Admin client certificates carry `os:admin` / `system:masters` and chain to their CAs `[unit]` **P0**
- Only the pinned Talos minor version is accepted for cluster nodes `[unit]` **P0**
- Machine configs match the `talosctl gen config` document set; workers carry no CA private keys; VIP and taint rules `[unit]` **P0**
- talosconfig and kubeconfig rendering `[unit]` **P1**
- Bootstrap gRPC status handling (`0`, `6` ALREADY_EXISTS, errors) and Ready node counting `[unit]` **P0**
- Controllers cannot be redeployed; workers can `[unit]` **P0**
- End to end: 1 controller + 1 worker come up Ready, the returned kubeconfig works, controller redeploy is refused, a worker redeploy rejoins, `download` returns credentials `[integration]` **P0**

Tests: `shared::data::node`, `services::{node_ops, up, resume, ssh_config, import, guest_account}`,
`services::talos::*`, `tls::generator`, `validate::cluster`, `crates/template/tests/talos`,
client `cluster_credentials`, and `crates/server/tests/talos_cluster_tests.rs`.

The integration test needs root, KVM, internet access and the Talos image at
`/opt/sherpa/images/talos_linux/<version>/virtioa.qcow2` (default version 1.14.4, override with
`SHERPA_TEST_TALOS_VERSION`). It is not provisioned by the VM release suite yet. Run it on a
disposable host:

```sh
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad --test talos_cluster_tests -- --ignored --nocapture
```
