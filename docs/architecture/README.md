# Application Architecture

This directory documents how Sherpa is built: crate boundaries, server internals, transports,
networking mechanics and observability. It is written for people changing the code.

User-facing documentation (installation, CLI reference, manifest reference, node guides) lives
in the separate `sherpa-docs` repository at `../sherpa-docs` and is published as the docs site.
Do not document user-facing behaviour here; document it there.

| Document | Covers |
|---|---|
| [`ACCOUNTS.md`](ACCOUNTS.md) | Server usernames: the shared rule, reserved names, case-insensitive storage and lookup, enforcement points; the built-in `admin` account; owner SSH keys. |
| [`DATABASE.md`](DATABASE.md) | The embedded Turso database in `crates/db`: the `Database` handle, connections and the write lock, row mapping and record keys, schema versions, rule triggers, backups, the SurrealDB importer. |
| [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md) | How a VM node's login, environment variables, text files, scripts and certificates reach the guest: cloud-init staging, Ignition, native tools per OS, per-OS cloud-init adjustments. |
| [`PROVISIONERS.md`](PROVISIONERS.md) | Client-side provisioners: the never-serialized manifest field, validation and server refusal, the SSH provisioner's remote steps and transport, and when the CLI runs them. |
| [`SSH_CLIENT.md`](SSH_CLIENT.md) | `sherpa ssh` through the Jato library: connecting through the server, the host key prompt, raw terminal pass-through, `~.`, resizes and exit codes. |
| [`SERVER.md`](SERVER.md) | Server startup, daemon manager, routing, auth, services, TLS, scanner, destroy flow, source map. |
| [`API.md`](API.md) | API transports (REST, WebSocket RPC, CLI), generated spec and OpenAPI, auth levels, streaming, error format. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | Point-to-point link implementation: eBPF redirect, veth/tap plumbing, netem coexistence. |
| [`OTEL.md`](OTEL.md) | OpenTelemetry tracing and metrics instrumentation and exported metric names. |
| [`TALOS.md`](TALOS.md) | Talos Linux nodes and Kubernetes clusters: NoCloud delivery, cluster plan, secrets, machine configs, bootstrap, lifecycle. |
| [`VALIDATION.md`](VALIDATION.md) | Where validators live (`shared::validators`, `topology::validate`, crate `validate` modules) and how client and server checks relate. |

Proposals for architecture changes that are not yet implemented live in [`../prds/`](../prds/).

## Adding a document

- One file per subsystem. Name it after the subsystem, in upper case, with a `.md` extension.
- State what the code does today. Put proposed changes in `../prds/`.
- Reference code by path so a reader can verify the claim.
- Add a row to the table above.
