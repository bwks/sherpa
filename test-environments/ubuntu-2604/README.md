# Ubuntu 26.04 test environment

A bare Ubuntu 26.04 LTS guest. See [test-environments](../README.md) for usage.
The [VM release-test harness](../../test-specs/install/vm-release-harness.md)
builds its guests from this manifest, using [release-test.toml](release-test.toml).

## Image

The image build and checksum are recorded in the
[release-test plan](../../test-specs/integration/vm-release-test-plan.md)
(Phase 0): `ubuntu_linux` version `26.04`, Canonical build `20260927`, SHA-256
`8800651811af9a85465ad1d552add729947bb16488dddb4a9b5305a3d97332b2`. The stored
image was rechecked against that checksum on 2026-10-16.

## Verified baseline

Verified on 2026-10-16 with `scripts/test_env.py`, lab `3c85384c`, including a
destroy and fresh `up`:

| Check | Result |
|---|---|
| Operating system | Ubuntu 26.04.1 LTS |
| Resources | 8 vCPUs, 100 GiB disk |
| Guest initialization | cloud-init `done`, no errors; SSH and passwordless sudo work |
| Server baseline | No Sherpa, Docker, libvirt or SurrealDB; `/opt/sherpa` absent |
| Nested virtualization | `/dev/kvm` API version 12; VM handle created |
| Guest network access | All five endpoints reachable |

## Earlier baseline (release-test VM)

Verified on 2026-10-03 for lab `f804c420`, domain `testbox-f804c420`:

| Check | Result |
|---|---|
| Operating system | Ubuntu 26.04.1 LTS, `VERSION_ID=26.04` |
| Resources | 4 vCPUs, 7.75 GiB usable memory, 60 GiB disk |
| Guest initialization | Cloud-init finished without errors; SSH and passwordless sudo work |
| Nested virtualization | CPU exposes VMX; `/dev/kvm` reports API version 12; creating a temporary KVM VM succeeds |
| Server baseline | Sherpa, Docker, libvirt and SurrealDB commands and runtime packages absent; `/opt/sherpa` absent |
| Installer prerequisite | Curl is already present in the official image; no extra packages were installed |
| Guest network access | Ubuntu repository metadata, GitHub release API and Docker installer reachable; Docker Hub and GHCR return their expected authentication challenges |

The existing host has nested virtualization enabled, and Sherpa's normal
host-model CPU configuration exposes the required features to the guest. No changes
to the host's virtualization settings were needed. Existing VMs were left unchanged.

The KVM check created and closed an in-memory VM handle; it did not boot an inner
guest. Registry checks establish connectivity, not successful image pulls.
The automated runner creates separate VMs. The original baseline lab was destroyed
on 2026-10-03 using `sherpa destroy --yes`; its VM, both disks, router, networks and
lab records were removed. A subsequent `sherpa inspect` confirmed the lab was not
found. The checks above are a historical baseline record, not a retained VM.

