# Sherpa API Reference

For server architecture, startup, routing, auth, services, TLS, and background components, see [`SERVER.md`](SERVER.md).

## Source of Truth

The **unified API specification** is the canonical source of truth for all Sherpa interfaces. It is auto-generated from the Rust types in `crates/shared/src/data/` and the operation registry in `crates/shared/src/api_spec.rs`.

**Spec endpoint:** `GET /api/v1/spec`

```bash
curl http://<server>:3030/api/v1/spec | jq .
```

**OpenAPI endpoint:** `GET /api/v1/openapi.json`

```bash
curl http://<server>:3030/api/v1/openapi.json | jq .
```

This generated API metadata defines every operation, its request/response schemas, and how to invoke it via each transport (REST, WebSocket RPC, CLI). The unified spec is the Sherpa-specific contract; the OpenAPI document is generated from it for HTTP tooling and Swagger UI.

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
  -d '{"username":"alice","password":"<password>"}'
```

Returns a JWT token valid for 7 days. Use it in subsequent requests:

```
Authorization: Bearer <token>
```

The token names a server-side login session and works only while that session exists.
`POST /api/v1/auth/logout` (RPC `auth.logout`) ends it; a password change ends the user's other
sessions. See [`SERVER.md`](SERVER.md#sessions-and-identity).

### Auth Levels

Operations require one of three auth levels (defined per-operation in the spec):

- **none** - Public endpoints (health, cert, spec, login)
- **authenticated** - Any logged-in user
- **admin** - Admin users only

## Streaming Operations

The generated API registry marks five canonical operations as streaming: `lab.create`, `lab.destroy`, `node.redeploy`, `image.pull`, `image.download`.

The current server implementation also streams `image.import` progress over REST and WebSocket.

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

`sherpad` logs the certificate's SHA-256 fingerprint when it loads it
(`CertificateManager::load_server_config`), in the same format the client shows.

The client builds its TLS configuration in `shared::tls::TlsConfigBuilder::build_with_trust_flow`:

1. `insecure` skips verification; `ca_cert_path` trusts only that CA.
2. A certificate in the client's trust store (`~/.sherpa/trusted_certs`) is trusted, with the
   public CAs (`webpki-roots`) as well when `validate_certs` is on.
3. Otherwise, with `validate_certs` on, only the public CAs are trusted. `PublicCaVerifier`
   wraps rustls's WebPKI verifier and flags a refusal for `UnknownIssuer`. When the handshake
   fails with the flag set (`ServerTls::refused_unknown_issuer`), `WebSocketClient::connect`
   runs `trust_on_first_use` and connects again.
4. With `validate_certs` off, `trust_on_first_use` runs before connecting.

`trust_on_first_use` fetches `/cert` from the server's HTTP port
(`[server_connection] http_port`), shows the certificate and its fingerprint, and saves it to
the trust store if the user accepts. A certificate refused for any reason other than an unknown
issuer (expired, wrong name, a CA certificate used as the server's) is an error, not a prompt.

## CORS

The server sends no CORS headers, so browsers refuse cross-origin calls. The web UI calls only
its own origin and the CLI is not a browser, so neither needs a grant. A layer that mirrors the
request origin with credentials would let any web page call the API with a signed-in user's
session cookie, which the REST routes also accept.
