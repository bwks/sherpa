# Application Architecture

This directory documents how Sherpa is built: crate boundaries, server internals, transports,
networking mechanics and observability. It is written for people changing the code.

User-facing documentation (installation, CLI reference, manifest reference, node guides) lives
in the separate `sherpa-docs` repository at `../sherpa-docs` and is published as the docs site.
Do not document user-facing behaviour here; document it there.

| Document | Covers |
|---|---|
| [`AGENT_BUS.md`](AGENT_BUS.md) | Development tool, not released: the NATS bus between the host agent and guest agents, its messages and flows, NKey permissions, how guests run Claude Code, base sessions, usage, state files. |
| [`DATABASE.md`](DATABASE.md) | The embedded Turso database: why one writer and a connection per operation, build-time query checks, positional mapping, schema version rules and Turso's limits, CPU pin triggers, lab definitions as written, sealed secrets and the database key. |
| [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md) | How a VM's owner login, device password, files, environment and scripts reach the guest, and why: delivery per OS, password forms, cloud-init staging, Windows account and key isolation. |
| [`PROVISIONERS.md`](PROVISIONERS.md) | Client-side provisioners: why the server never holds their contents, the never-serialized manifest field, the SSH transport's retry and prompt rules and concurrency, container secret files in RAM. |
| [`LAB_FOLDER.md`](LAB_FOLDER.md) | The files the CLI generates for a lab in `.sherpa/` beside the manifest: layout and modes, the SSH config rewrite, how a folder names its lab (`lab-info.toml`, the lab ID rule, which commands need a manifest), destroy, and the temporary migration from the old loose layout. |
| [`SERVER.md`](SERVER.md) | The server's design and invariants: startup order, storage permissions, sessions and the admin boundary, up and destroy, operation leases, admission, redeploy, the scanner, readiness and host key trust, consoles. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | Point-to-point link implementation: eBPF redirect, veth/tap plumbing, netem coexistence. |
| [`NODE_SETTINGS.md`](NODE_SETTINGS.md) | Node settings in images and manifests: model and kind pairs, one kind's group per image, unknown keys refused, the one per-kind settings table, container limits. |
| [`TALOS.md`](TALOS.md) | Talos Linux nodes and Kubernetes clusters: NoCloud delivery, cluster plan, secrets, machine configs, Cilium, Keycloak login and Vault secrets, bootstrap, lifecycle. |
| [`ZTP.md`](ZTP.md) | Zero touch provisioning: the one `Ztp` setting, the methods each model takes, the validator and its callers, the manifest override, the readers, the blank disk templates and the scripts that make them, container start settings against ZTP, and migration 0016. |
| [`ZTP_ISO.md`](ZTP_ISO.md) | The ISO 9660 writer for node config media: callers and labels, the primary, Rock Ridge and Joliet trees, sector layout, limits. |
| [`BUILD.md`](BUILD.md) | Build profiles in `Cargo.toml`, and the no-`ring` crypto invariant, which only `cargo tree` checks. |
| [`RELEASE.md`](RELEASE.md) | The one workspace version, the bump pull request (`mise run release:bump`), the manual Release workflow, the `Version` check, docs-only pull requests skipping the build jobs, and why `ebpf-redirect` is not versioned with Sherpa. |
| [`STATE_MACHINES.md`](STATE_MACHINES.md) | Lifecycles held in enums, flags, strings and side effects: node and lab state, the scanner, operation leases, up/destroy/redeploy stages, provisioning, sessions, jobs, streams, TLS trust, links, images, the ZTP setting. States, transitions, who sets and reads them, with diagrams. |

## Adding a document

- One file per subsystem. Name it after the subsystem, in upper case, with a `.md` extension.
- State what the code does today. Put proposed changes in a GitHub issue.
- Reference code by path so a reader can verify the claim.
- Add a row to the table above.
