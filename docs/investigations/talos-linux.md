# Talos Linux on Sherpa

> Status: implemented for #326. The lasting design is in `docs/architecture/TALOS.md`; this
> file keeps the experiments behind it.

Research for issue #326: what it takes to run Talos Linux VMs, and Kubernetes clusters on them,
in a Sherpa lab. Experiments were run on the development host on 2026-10-17 with Talos
**v1.14.4** (Kubernetes v1.37.1) in throwaway QEMU VMs using user-mode networking. Nothing here
was run through Sherpa itself; the VMs reproduced what Sherpa would do (a `cidata` ISO, a virtio
disk, `host-model` CPU).

## Summary of findings

| Question | Result |
|---|---|
| Image | Image Factory publishes a ready NoCloud qcow2: `factory.talos.dev/image/<schematic>/v<version>/nocloud-amd64.qcow2`. The vanilla schematic (no extensions) is `376567988ad370138ad8b2698212367b8edcb69b5fd68c80be1f2ec7d603b4ba`. 222 MiB on disk, **4.15 GiB virtual**: it must be grown to at least 10 GiB. |
| CPU | Talos needs x86-64-v2. Sherpa's default `host-model` CPU reports level 3 on this host. |
| Config delivery | A `cidata` ISO with `user-data` (the machine config), `meta-data` and `network-config` works with no network and no maintenance mode. Talos read all three at about 8 s after power-on. |
| Config format | Talos 1.14 configs are **multi-document**: one legacy `v1alpha1` document plus about 27 typed documents (`HostnameConfig`, `KubeAPIServerCAConfig`, `KubeNodeConfig`, ...). A `---`-separated stream of **JSON** documents is accepted. |
| Strictness | Decoding is strict: an unknown key in any document is fatal ("unknown keys found during decoding"). Setting a field in legacy `v1alpha1` that a typed document now owns is a validation error ("static hostname is already set in v1alpha1 config"). |
| Network | Sherpa's netplan v2 `network-config` (interface `id0`, match by MAC, static address, default route, nameservers) is applied as-is: static address, gateway and DNS with no DHCP. The v2 `search` domain is ignored (`searchDomains: []`). v1 format also works. |
| Keys | All five private keys work as **PKCS#8 `PRIVATE KEY`**, which is what `rcgen` emits. `talosctl` writes SEC1/PKCS#1 and an `ED25519 PRIVATE KEY` label, but that Ed25519 block is standard PKCS#8 DER under a custom label. |
| API | The Talos API (apid, TCP 50000, mTLS) is reachable about 6 s after the config loads. The client identity is a certificate with `O=os:admin`, signed by the Talos OS CA. |
| Bootstrap | No machine-config field triggers bootstrap. One API call, `machine.MachineService/Bootstrap` with an empty request, is needed on one control plane. It worked as a raw gRPC call over HTTP/2: POST of a 5-byte empty frame, `grpc-status: 0`. `talosctl` is not required. |
| Bootstrap with `reqwest` | `reqwest` 0.13 with its `http2` feature works, and can read gRPC trailers by converting to `http::Response` and collecting the body with `http-body-util`. Success: HTTP 200, `grpc-status: 0` in the **trailers**, a response message in the body. Already bootstrapped: a trailers-only response with `grpc-status: 6` (`ALREADY_EXISTS`, "etcd data directory is not empty") in the **headers** and an empty body. |
| Bootstrap is not guarded | A second Bootstrap sent about 20 ms after the first, while etcd was restarting, was also accepted. Send exactly one Bootstrap, to one control plane. Retry only against the same node, and treat `ALREADY_EXISTS` as done. |
| Kubeconfig | An admin kubeconfig built from the cluster's Kubernetes CA (client certificate `O=system:masters`) works. No API call is needed to fetch it. |
| Timing | From power-on: apid open at 22 s, node `Ready` at 108 s, an nginx deployment `Available` at 115 s. Images were pulled from the internet through NAT. |
| Resources | Minimum: control plane 2 vCPU / 2 GiB / 10 GiB, worker 1 vCPU / 1 GiB / 10 GiB. Recommended: 4 / 4 GiB / 100 GiB and 2 / 2 GiB / 100 GiB. |

## Secrets a cluster needs

From `talosctl gen secrets`. All of them can be generated in Rust with crates already in the
workspace.

| Secret | Type | Rust source |
|---|---|---|
| Talos OS CA (`machine.ca`) | Ed25519 key + self-signed CA | `rcgen` with `PKCS_ED25519` |
| etcd CA | ECDSA P-256 CA | `rcgen` |
| Kubernetes CA | ECDSA P-256 CA | `rcgen` |
| Aggregator CA | ECDSA P-256 CA | `rcgen` |
| Service account key | RSA 4096 from `talosctl` | `rcgen` cannot generate RSA. The `rsa` crate is already compiled in through `ssh-key`. Whether Talos accepts an ECDSA key here is untested. |
| Bootstrap token, trustd token | `[a-z0-9]{6}.[a-z0-9]{16}` | `rand` |
| Secretbox encryption secret, cluster ID and secret | 32 random bytes, base64 | `rand` + `base64` |

Client credentials are derived from these CAs, not stored separately: the talosconfig uses an
`os:admin` certificate signed by the OS CA, and the kubeconfig uses a `system:masters`
certificate signed by the Kubernetes CA.

## Sherpa today

- **No code exists for Talos.** `git grep -i talos` in `crates/` finds nothing.
- **User docs already describe it.** `../sherpa-docs/docs/nodes/linux/talos-linux.md` (sherpa-docs
  commit `38ed29e`, in the nav) documents a `talos_linux` model, Talos 1.10.5, with
  bring-your-own configs and readiness on port 50000. Following it fails today with an unknown model.
- **Existing pieces that fit:**
  - The `cidata` ISO builder (`create_ztp_iso`).
  - The `Cdrom` ZTP path, which already writes a custom `ztp_config` verbatim.
  - Netplan v2 `network-config` (`CloudInitNetwork::ztp_interface`).
  - JSON-as-YAML guest documents (`template::guest_yaml`).
  - `rcgen` lab PKI (`server::tls::generator`).
  - Per-lab NAT with internet access.
  - Management IPs known before boot: gateway `.1`, router and boot server `.2`, nodes `.10 + index`. The dnsmasq range is `.10`–`.254`.
- **Missing:**
  - A per-model readiness port for VMs. `up`, `resume` and `redeploy` hard-code SSH port 22.
  - A model-level default boot disk size.
  - A post-readiness, lab-wide step in `up`.
  - A way to return extra files (talosconfig, kubeconfig) from `up` and `download`.
  - Talos nodes must be left out of the generated SSH config.

## Verified with Sherpa's own code

A throwaway program linked against the `sherpad` library generated a cluster with
`generate_cluster_secrets` and `talos_machine_config`, booted it, and ran `bootstrap` and
`wait_for_kubernetes`: the config loaded (including an **ECDSA service account key** and no
discovery documents), bootstrap returned `Bootstrapped` then `AlreadyBootstrapped`, the node was
Ready about 2 minutes after power-on, the generated kubeconfig worked with `kubectl`, and
`talosctl` read the generated talosconfig. The Kubernetes API certificate includes
`127.0.0.1`; the Talos API certificate does not unless it is added to `machine.certSANs`.

## Not tested

- More than one node: control-plane HA with `Layer2VIPConfig`, and workers joining.
- Dual-stack (IPv6) management networks.
- A lab with no internet access (image cache / registry mirror).
- Talos versions other than 1.14.4. The config schema changed substantially between 1.10 and 1.14.
- Running the real `up` path inside Sherpa (Q35, SATA CD-ROM, libvirt NAT, dnsmasq): covered by
  the ignored `talos_cluster_tests` integration test, which needs root and the Talos image.

## Reproducing

The test kit lived in the session's temporary directory and was not kept. To reproduce:

1. Download the NoCloud qcow2 and `talosctl` for the same version.
2. `talosctl gen secrets` and `gen config`. Drop the `UnattendedInstallConfig` document, because the NoCloud disk image is already installed.
3. Write the documents as JSON separated by `---` into `user-data`, add `meta-data` and `network-config`, and build the ISO with `genisoimage -V cidata`.
4. Grow the qcow2 to 20 GiB and boot it with `qemu-system-x86_64 -enable-kvm -cpu host`, a virtio disk, the ISO as a CD-ROM, and user-mode networking forwarding 50000 and 6443.
5. Bootstrap with an HTTP/2 POST to `/machine.MachineService/Bootstrap` (body `00 00 00 00 00`, `content-type: application/grpc`), using the talosconfig client certificate.
