# Sherpa API Reference

## Source of Truth

The **unified API specification** is the canonical source of truth for all Sherpa interfaces. It is auto-generated from the Rust types in `crates/shared/src/data/` and the operation registry in `crates/shared/src/api_spec.rs`.

**Spec endpoint:** `GET /api/v1/spec`

```bash
curl http://<server>:3030/api/v1/spec | jq .
```

This single JSON document defines every operation, its request/response schemas, and how to invoke it via each transport (REST, WebSocket RPC, CLI). All three interfaces implement from this spec.

When adding or modifying operations, update the registry in `crates/shared/src/api_spec.rs`. The JSON schemas are derived automatically from the `#[derive(JsonSchema)]` annotations on the shared data types.

## Transports

Sherpa exposes the same operations via three transports:

### REST API

- Base URL: `https://<server>:3030/api/v1/`
- Auth: JWT Bearer token in `Authorization` header, or session cookie
- Streaming operations use Server-Sent Events (SSE)
- Paths and methods are defined in the spec under `transports.rest`

### WebSocket JSON-RPC 2.0

- Endpoint: `wss://<server>:3030/ws`
- Auth: Token passed in RPC params
- Streaming operations send `status` messages before the final `rpc_response`
- Method names are defined in the spec under `transports.rpc`

### CLI

- Binary: `sherpa`
- Auth: Token stored in `~/.sherpa/token` after `sherpa login`
- Commands are defined in the spec under `transports.cli`

## Authentication

### Login

```bash
curl -X POST https://<server>:3030/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"alice","password":"SecurePass123!"}'
```

Returns a JWT token valid for 7 days. Use it in subsequent requests:

```
Authorization: Bearer <token>
```

### Auth Levels

Operations require one of three auth levels (defined per-operation in the spec):

- **none** - Public endpoints (health, cert, spec, login)
- **authenticated** - Any logged-in user
- **admin** - Admin users only

## Streaming Operations

Five operations stream progress updates: `lab.create`, `lab.destroy`, `node.redeploy`, `image.pull`, `image.download`.

**Via REST:** Response is an SSE stream. Progress events followed by a final result event.

**Via WebSocket:** Multiple `ServerMessage::Status` messages followed by a final `ServerMessage::RpcResponse`.

**Via CLI:** Progress lines printed to stdout in real-time.

## Error Format

REST errors return:

```json
{
  "error": {
    "code": "ERROR_CODE",
    "message": "Human-readable message",
    "details": "Additional context (optional)"
  }
}
```

WebSocket RPC errors follow JSON-RPC 2.0 error format.

## TLS

The server uses TLS. Download the certificate for trust-on-first-use:

```bash
curl http://<server>:3031/cert > server.crt
```

## CORS

CORS is enabled for all origins with credentials support. Configure specific origins for production.
