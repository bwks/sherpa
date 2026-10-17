# Talos Linux and Kubernetes clusters

How Sherpa runs Talos Linux nodes and builds Kubernetes clusters from them. User-facing
behaviour is documented in `../sherpa-docs` (Talos Linux node page, manifest `clusters`).
The research behind these choices is in `docs/investigations/talos-linux.md`.

## Node model

`NodeModel::TalosLinux` (`crates/shared/src/data/node.rs`) is a Q35 virtio VM: 2 vCPU, 4 GiB,
SATA CD-ROM, `host-model` CPU (Talos needs x86-64-v2).

- **Disk.** `NodeModel::default_boot_disk_size()` returns 20 GB for Talos; the NoCloud image is
  about 4 GB virtual and Talos needs at least 10. `process_manifest_nodes` (`up.rs`) applies it
  when the manifest sets no `boot_disk_size`.
- **Readiness.** `NodeModel::ready_port()` returns the Talos API port (50000); every other VM
  model returns SSH (22). `node_ops::vm_ready_port` prefers a manifest `ready_port`. `up`,
  `resume` and `redeploy` all probe this port.
- **No SSH.** `NodeModel::has_ssh()` is false. `ZtpRecord::ssh_port` is `None` for such nodes,
  so `build_ssh_config` leaves them out, and `NodeInfo::ssh_port` is `None`.
- **Image.** `resolve_download_url` posts the vanilla schematic (`customization: {}`) to the
  Image Factory's `/schematics` endpoint, reads the returned ID, and downloads
  `/image/<id>/v<version>/nocloud-amd64.qcow2`. The ID is derived from the schematic's
  content, so Sherpa asks for it rather than storing it. `--url` skips the lookup.

## NoCloud delivery

`ZtpMethod::NoCloud` (`node_ops::generate_nocloud_ztp`) builds a `cidata` ISO from:

- `meta-data`: instance ID and hostname;
- `network-config`: Sherpa's netplan v2 document (`CloudInitNetwork::ztp_interface`), a static
  management address, gateway and DNS server — Talos applies it with no DHCP;
- `user-data`: the node's machine config, written byte for byte, when there is one. Without
  it any earlier `user-data` is removed and Talos waits in maintenance mode.

The machine config comes from the node's `ztp_config`: the user's file, or the config Sherpa
generates for cluster nodes (validation forbids users setting `ztp_config` on those).

## Clusters

Manifest `talos_clusters` (`topology::TalosCluster`): each cluster names its member nodes in
`controllers` (the Talos control plane, in bootstrap order) and `workers`. Nodes carry no cluster
fields; `topology::find_talos_cluster` looks up a node's cluster and role (`ClusterRole`, which
keeps the Talos term `ControlPlane` internally). `validate::validate_talos_clusters` enforces the
rules (names, one or three controllers, members are defined `talos_linux` nodes listed once
without a user `ztp_config`, VIP capacity). Two controllers are refused: etcd needs a majority,
so two members tolerate no failure and are less available than one.

```text
up, Phase 8                              up, after node readiness
-----------                              ------------------------
plan_clusters (endpoint / VIP)           bring_up_clusters
generate_cluster_secrets  --saved-->      write talosconfig + kubeconfig
node_machine_config per node              bootstrap (first controller, once)
  -> node.ztp_config -> NoCloud ISO       wait_for_kubernetes (all members Ready)
```

Code: `crates/server/src/services/talos/` (`plan`, `secrets`, `bootstrap`, and the lab
integration in `lab`); document rendering in `crates/template/src/talos.rs`.

### Plan

`plan::plan_clusters` lists each cluster's control planes and workers with their management
addresses (`.20 + node index`, as Phase 8 assigns them; see "Lab management addressing" in
`SERVER.md`). A cluster with one control plane uses its address as the Kubernetes endpoint. A
cluster with three gets a shared address from `TALOS_VIP_HOST_OFFSETS` (`.11`–`.12`, in the lab
control-plane block, outside the dnsmasq DHCP range), in manifest order, and a `Layer2VIPConfig`
on its control planes.

### Secrets

`secrets::generate_cluster_secrets`, using the generic helpers in `tls/generator.rs`:

| Secret | Kind |
|---|---|
| Talos OS CA | Ed25519, `O=talos` |
| etcd, Kubernetes, aggregator CAs | ECDSA P-256 (`O=etcd`, `O=kubernetes`, empty) |
| Service account key | ECDSA P-256 |
| Bootstrap and trustd tokens | `[a-z0-9]{6}.[a-z0-9]{16}` |
| Secretbox key | 32 random bytes, base64 |

All private keys are PKCS#8. Client certificates are issued from a CA rebuilt from its saved key
(`issue_client_certificate`): a child certificate only needs the issuer's subject and key.
The talosconfig carries an `O=os:admin` certificate (OS CA); the kubeconfig an
`O=system:masters` certificate (Kubernetes CA).

Saved per cluster under `/opt/sherpa/labs/<lab>/clusters/<cluster>/`, mode 0600:
`secrets.toml`, `cluster.toml` (the plan), `talosconfig`, `kubeconfig`. `redeploy` and
`download` read them; `destroy` removes them with the lab directory.

### Machine configs

`template::talos_machine_config` writes the Talos 1.14 multi-document config as a
`---`-separated stream of JSON documents (JSON is valid YAML). Talos decodes strictly — an
unknown key or a legacy `v1alpha1` field that a typed document now owns is fatal — so the
documents follow `talosctl gen config` for the pinned version exactly, except: no
`UnattendedInstallConfig` (the NoCloud image is installed), no discovery documents, a static
`HostnameConfig`, `KubeNodeConfig.nodeIP.validSubnets` set to the management subnet, and no
control-plane taint when the cluster has no workers.

`TALOS_SUPPORTED_MINOR_VERSION` pins the generator to Talos 1.14; `ensure_supported_version`
refuses cluster nodes on other minor versions. Supporting a new Talos minor release means
regenerating the reference with `talosctl gen config`, updating the documents and the template
tests, and moving the pin.

### Bootstrap

Talos has no config field that bootstraps etcd: one API call does. `bootstrap::bootstrap` sends
`machine.MachineService/Bootstrap` (an empty gRPC frame) over HTTP/2 with `reqwest`, trusting
only the OS CA and presenting the `os:admin` certificate. The gRPC status is read from the
headers (a trailers-only refusal) or the trailers (success), through `http-body-util`.

Talos accepts a second bootstrap while the first is still running, so Sherpa sends it to one
controller per cluster only — the first in its `controllers` list — and retries only that node.
Once etcd has data Talos answers `ALREADY_EXISTS`, which counts as done.

`wait_for_kubernetes` then polls `/api/v1/nodes` with the admin kubeconfig credentials until
every member is Ready, within the lab's `ready_timeout`. A cluster that is not ready is
reported as a non-critical `UpError`, like node readiness.

### Lifecycle

| Operation | Behaviour |
|---|---|
| `down` / `resume` | Unchanged; Talos keeps its state on disk. |
| `redeploy` worker | Config regenerated from the saved secrets and plan; the node rejoins. |
| `redeploy` controller | Refused (`talos::ensure_redeployable`): its etcd member would have to be removed first. |
| `download` | Returns each cluster's talosconfig and kubeconfig (`DownloadLabResponse::clusters`). |
| `destroy` | Nothing extra; the cluster directory goes with the lab directory. |

The client writes `kubeconfig-<cluster>` and `talosconfig-<cluster>` (owner only) on `up` and
`download` (`crates/client/src/cluster_credentials.rs`).

## Limits

- Nodes need internet access for Kubernetes images; there is no image cache or registry mirror.
- One Talos minor version at a time.
- No control-plane redeploy, upgrades, or CNI choice (Flannel only).
