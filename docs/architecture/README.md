# Application Architecture

This directory documents how Sherpa is built: crate boundaries, server internals, transports,
networking mechanics and observability. It is written for people changing the code.

User-facing documentation (installation, CLI reference, manifest reference, node guides) lives
in the separate `sherpa-docs` repository at `../sherpa-docs` and is published as the docs site.
Do not document user-facing behaviour here; document it there.

| Document | Covers |
|---|---|
| [`AGENT_BUS.md`](AGENT_BUS.md) | Development tool, not released: the NATS bus between the host agent and guest agents, its messages and flows, NKey permissions, how guests run Claude Code, base sessions, usage, state files. |
| [`DATABASE.md`](DATABASE.md) | The embedded Turso database in `crates/db`: the `Database` handle, connections and the write lock, row mapping and record keys, schema versions, triggers, backups, and upgrades from before 0.5.0. |
| [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md) | How a VM node's login, environment variables, text files, scripts and certificates reach the guest: cloud-init staging, Ignition, native tools per OS, per-OS cloud-init adjustments. |
| [`PROVISIONERS.md`](PROVISIONERS.md) | Client-side provisioners: the never-serialized manifest field, validation and server refusal, the SSH provisioner's remote steps and transport, and when the CLI runs them. |
| [`LAB_FOLDER.md`](LAB_FOLDER.md) | The files the CLI generates for a lab in `.sherpa/` beside the manifest: layout and modes, the SSH config rewrite, how a folder names its lab (`lab-info.toml`, the lab ID rule, which commands need a manifest), destroy, and the temporary migration from the old loose layout. |
| [`SERVER.md`](SERVER.md) | Server startup, daemon manager, routing, auth, services, TLS, scanner, destroy flow, source map. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | P2p links: the eBPF TC redirect and why it is a netlink filter, re-attach, netem, container veths, checksum offload, scanner repair and carrier. |
| [`NODE_SETTINGS.md`](NODE_SETTINGS.md) | A node's settings groups in the image definition (`NodeConfig`) and the manifest node (`topology::Node`): the groups, where their types live, unknown keys refused, which kinds take which settings and the validators. |
| [`TALOS.md`](TALOS.md) | Talos nodes and Kubernetes clusters: the design decisions behind readiness, controllers, Cilium, Keycloak and Vault, machine configs, bootstrap and shutdown, and the limits. |
| [`ZTP.md`](ZTP.md) | Zero touch provisioning: the one `Ztp` setting, where it is validated, its readers, the boot server and its refresh on redeploy, containers with ZTP off, the SONiC container, blank disks. |
| [`ZTP_ISO.md`](ZTP_ISO.md) | The ISO 9660 writer for node config media: how config files reach the guest, the three name views and the writer's design decisions. |
| [`BUILD.md`](BUILD.md) | Cargo profiles (`dev`, `test`, `release`, `dist`): debug info, the optimised crypto dependencies, the linker, the build scripts and nextest. |
| [`RELEASE.md`](RELEASE.md) | The one workspace version, the bump pull request, the manual Release workflow, the `Version` check, docs-only pull requests, and why `ebpf-redirect` is not versioned with Sherpa. |
| [`STATE_MACHINES.md`](STATE_MACHINES.md) | Lifecycles held in enums, flags, strings and side effects: node and lab state, the scanner, operation leases, up/destroy/redeploy stages, provisioning, sessions, jobs, streams, TLS trust, links, images, the ZTP setting. States, transitions, who sets and reads them, with diagrams. |

## Adding a document

- One file per subsystem. Name it after the subsystem, in upper case, with a `.md` extension.
- State what the code does today. Put proposed changes in a GitHub issue.
- Reference code by path so a reader can verify the claim.
- Add a row to the table above.
