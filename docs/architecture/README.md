# Application Architecture

This directory documents how Sherpa is built: crate boundaries, server internals, transports,
networking mechanics and observability. It is written for people changing the code.

User-facing documentation (installation, CLI reference, manifest reference, node guides) lives
in the separate `sherpa-docs` repository at `../sherpa-docs` and is published as the docs site.
Do not document user-facing behaviour here; document it there.

| Document | Covers |
|---|---|
| [`SERVER.md`](SERVER.md) | Server startup, daemon manager, routing, auth, services, TLS, scanner, destroy flow, source map. |
| [`API.md`](API.md) | API transports (REST, WebSocket RPC, CLI), generated spec and OpenAPI, auth levels, streaming, error format. |
| [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md) | Point-to-point link implementation: eBPF redirect, veth/tap plumbing, netem coexistence. |
| [`OTEL.md`](OTEL.md) | OpenTelemetry tracing and metrics instrumentation and exported metric names. |

Proposals for architecture changes that are not yet implemented live in [`../prds/`](../prds/).

## Adding a document

- One file per subsystem. Name it after the subsystem, in upper case, with a `.md` extension.
- State what the code does today. Put proposed changes in `../prds/`.
- Reference code by path so a reader can verify the claim.
- Add a row to the table above.
