# Application Architecture

This directory documents how Sherpa is built: crate boundaries, server internals, transports,
networking mechanics and observability. It is written for people changing the code.

User-facing documentation (installation, CLI reference, manifest reference, node guides) lives
in the separate `sherpa-docs` repository at `../sherpa-docs` and is published as the docs site.
Do not document user-facing behaviour here; document it there.

| Document | Covers |
|---|---|
| [`AGENT_BUS.md`](AGENT_BUS.md) | Development tool, not released: the bus, enrolment, TLS and permissions, the guest loop, the record, work items, host usage and time, the Generation section, and how to run it. |
| [`DATABASE.md`](DATABASE.md) | The embedded Turso database: why one writer and a connection per operation, build-time query checks, positional mapping, schema version rules and Turso's limits, CPU pin triggers, lab definitions as written, sealed secrets and the database key. |
| [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md) | How a VM's owner login, device password, files, environment and scripts reach the guest, and why: delivery per OS, password forms, cloud-init staging, Windows account and key isolation. |
| [`PROVISIONERS.md`](PROVISIONERS.md) | Client-side provisioners: why the server never holds their contents, the never-serialized manifest field, the SSH transport's retry and prompt rules and concurrency, container secret files in RAM. |
| [`SERVER.md`](SERVER.md) | The server's design and invariants: startup order, storage permissions, sessions and the admin boundary, up and destroy, operation leases, admission, redeploy, the scanner, readiness and host key trust, consoles, the TLS listener, server certificate ownership and client key pinning. |
| [`CONTAINER_VM.md`](CONTAINER_VM.md) | Privileged container models in their own QEMU guest: the root filesystem through Docker and virtio-fs, the domain, the guest init, the kernel, the security boundary and cost. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | P2p links: the eBPF TC redirect and why it is a netlink filter, re-attach, netem, container veths, checksum offload, scanner repair and carrier. |
| [`NODE_SETTINGS.md`](NODE_SETTINGS.md) | Node settings in images and manifests: model and kind pairs, one kind's group per image, unknown keys refused, the one per-kind settings table, container limits. |
| [`TALOS.md`](TALOS.md) | Talos nodes and Kubernetes clusters: the design decisions behind readiness, controllers, Cilium, Keycloak and Vault, machine configs, bootstrap and shutdown, and the limits. |
| [`ZTP.md`](ZTP.md) | Zero touch provisioning: the one `Ztp` setting, where it is validated, its readers, the boot server and its refresh on redeploy, containers with ZTP off, the SONiC container, blank disks. |
| [`ZTP_ISO.md`](ZTP_ISO.md) | The ISO 9660 writer for node config media: how config files reach the guest, the three name views and the writer's design decisions. |
| [`BUILD.md`](BUILD.md) | Build profiles in `Cargo.toml`, and the no-`ring` crypto invariant, which only `cargo tree` checks. |
| [`RELEASE.md`](RELEASE.md) | The one workspace version, the bump pull request, the manual Release workflow, the `Version` check, docs-only pull requests, and why `ebpf-redirect` is not versioned with Sherpa. |

## Adding a document

- One file per subsystem. Name it after the subsystem, in upper case, with a `.md` extension.
- State what the code does today. Put proposed changes in a GitHub issue.
- Reference code by path so a reader can verify the claim.
- Add a row to the table above.
