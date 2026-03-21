# WebSocket Client — Test Specifications

> **Crate:** `crates/client/` (`ws_client/`)
> **External Dependencies:** Running Sherpa server, TLS certificates
> **Existing Tests:** None

---

## Connection Establishment

**What to test:**
- Plain WebSocket (ws://) connection succeeds `[integration]` **P0**
- Secure WebSocket (wss://) connection succeeds with valid cert `[integration]` **P0**
- TLS trust-on-first-use flow: fetches cert, saves to trust store `[integration]` **P1**
- Connection to unreachable server fails with timeout `[integration]` **P0**
- Connection timeout respects configured duration `[integration]` **P1**
- Initial "connected" message read after handshake `[integration]` **P1**

---

## RPC Request/Response

**What to test:**
- Request serialized and sent correctly `[integration]` **P0**
- Response matched by request ID `[integration]` **P0**
- Non-matching messages (status, logs) ignored during `call()` `[integration]` **P1**
- Ping/pong frames handled transparently `[integration]` **P2**
- Connection closed by server before response returns error `[integration]` **P0**

---

## Streaming Operations

**What to test:**
- `call_streaming()` delivers status messages via callback `[integration]` **P0**
- Streaming completes when final RPC response received `[integration]` **P0**
- Progress messages delivered in order `[integration]` **P1**
- Connection drop during streaming produces error `[integration]` **P1**

---

## Graceful Disconnection

**What to test:**
- `close()` sends WebSocket close frame `[integration]` **P1**
- Client handles server-initiated close gracefully `[integration]` **P1**
