# Ubuntu 24.04 test environment

A bare Ubuntu 24.04 LTS guest. See [test-environments](../README.md) for usage.

## Image

| Property | Recorded value |
|---|---|
| Sherpa model | `ubuntu_linux` |
| Sherpa version | `24.04-20260926` |
| Canonical build | `20260926` |
| Image filename | `ubuntu-24.04-server-cloudimg-amd64.img` |
| Server path | `/opt/sherpa/images/ubuntu_linux/24.04-20260926/virtioa.qcow2` |
| SHA-256 | `6a81c37564db9b1ee84e141922625e1d7c5b389b99bb3c572e0243607d5bb4d2` |

Source: [Canonical build 20260926](https://cloud-images.ubuntu.com/releases/noble/release-20260926/)
and its [SHA256SUMS](https://cloud-images.ubuntu.com/releases/noble/release-20260926/SHA256SUMS).
Imported on 2026-10-16 with `sherpa server image pull` and verified against that
checksum. It is not the default `ubuntu_linux` image.

The server's older `24.04` image does not match any Canonical build checked
(20251001 to 20251213), so it is not used here. It was left in place because
existing labs may use it as a backing disk.

## Verified baseline

Verified on 2026-10-16 with `scripts/test_env.py`, lab `0f5802dc`, including a
destroy and fresh `up`:

| Check | Result |
|---|---|
| Operating system | Ubuntu 24.04.5 LTS |
| Resources | 8 vCPUs, 100 GiB disk |
| Guest initialization | cloud-init `done`, no errors; SSH and passwordless sudo work |
| Server baseline | No Sherpa, Docker, libvirt or SurrealDB; `/opt/sherpa` absent |
| Nested virtualization | `/dev/kvm` API version 12; VM handle created |
| Guest network access | All five endpoints reachable; one archive TLS handshake timeout recovered on retry |
