# PRD: API Spec Streaming Parity for `image.import`

## Status

Draft backlog item.

## Problem

The generated API registry and the server implementation disagree about whether `image.import` is a streaming operation.

The server implementation streams image import progress over REST SSE and WebSocket JSON-RPC status messages. The registry in `crates/shared/src/api_spec.rs` marks `image.import` as non-streaming. This means `/api/v1/spec`, `/api/v1/openapi.json`, docs, generated clients, and tests can describe behavior that does not match the actual server.

## Findings

- `crates/shared/src/api_spec.rs` defines `image.import` with `streaming: false` and no REST `stream_type`.
- `api_spec.rs` has a test that expects exactly five streaming operations.
- `crates/server/src/api/handlers.rs` documents and implements `import_image_json` as SSE streaming.
- `crates/server/src/api/router.rs` registers `POST /api/v1/images/import` to `import_image_json`.
- `crates/server/src/api/websocket/handler.rs` includes `image.import` in the streaming RPC method list.
- `crates/server/src/api/websocket/rpc.rs` dispatches `image.import` through `handle_streaming_rpc_request` and uses `handle_image_import_streaming`.
- `docs/API.md` was updated to note the mismatch instead of hiding it.

## Goals

- Make the generated API spec match implemented server behavior.
- Ensure OpenAPI accurately represents REST streaming responses.
- Ensure CLI/client code can rely on the spec for streaming behavior.
- Add tests that prevent future spec/implementation drift.

## Non-goals

- Redesigning the full API generation pipeline.
- Changing image import business logic.
- Removing image import streaming unless a separate product decision is made.

## Recommended decision

Treat image import as a streaming operation in the public contract.

Reasoning:

- Import can be long-running.
- REST and WebSocket handlers already stream it.
- Users benefit from progress during large image imports.
- Changing implementation to non-streaming would reduce observability and likely degrade UX.

## Functional requirements

- `image.import` must be marked `streaming: true` in `build_operations()`.
- The REST binding for `image.import` must expose `stream_type: "sse"`.
- OpenAPI output for `POST /api/v1/images/import` must describe `text/event-stream` success content.
- WebSocket RPC docs and generated spec must agree that `image.import` sends status messages before final response.
- Tests must expect six streaming operations if `image.import` remains streaming.
- Documentation must stop calling the mismatch a rough edge once fixed.

## Non-functional requirements

- No breaking change to the actual REST or WebSocket behavior.
- Preserve admin-only requirement for image import.
- Preserve request and final response schemas.
- Avoid duplicating streaming-operation lists in multiple places without tests to catch drift.

## Action plan

### Phase 1: Confirm intended behavior

- Confirm that image import should remain streaming for REST, WebSocket, and CLI.
- Verify CLI behavior for `sherpa server image import` and whether it already consumes progress.

### Phase 2: Update API registry

- Change `image.import` operation metadata to streaming.
- Set REST stream type to SSE.
- Ensure response schema remains `ImportResponse` for the final complete event / final RPC response.

### Phase 3: Update tests

- Update streaming operation count tests from five to six.
- Add an explicit assertion that `image.import` is streaming.
- Add an OpenAPI test that `POST /api/v1/images/import` returns `text/event-stream` content.
- Add a consistency test comparing the WebSocket handler streaming-method list against the API registry's RPC streaming methods, if practical.

### Phase 4: Update docs

- Remove any mismatch language from `docs/API.md` and `docs/SERVER.md` after implementation.
- Document `image.import` as a normal streaming operation.

## Testing plan

- Unit tests in `shared::api_spec` for operation metadata.
- OpenAPI structure tests for the import endpoint.
- REST integration test confirming `POST /api/v1/images/import` returns an SSE response for an authenticated admin request, with heavy backend work mocked or guarded if necessary.
- WebSocket RPC test confirming `image.import` is routed through streaming behavior.

## Acceptance criteria

- `/api/v1/spec` marks `image.import` as streaming.
- `/api/v1/openapi.json` marks `POST /api/v1/images/import` as `text/event-stream`.
- Tests no longer assert only five streaming operations.
- Runtime REST/WebSocket behavior remains unchanged.
- Docs no longer describe a mismatch.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Generated clients treat all streaming ops identically and image import has different final semantics | Preserve `ImportResponse` as the final result schema and document event shape. |
| CLI implementation does not consume import streaming correctly | Include CLI behavior in Phase 1 and add a CLI-level test or manual validation item. |
| Spec and WebSocket streaming list drift again | Add a test comparing registry metadata to dispatcher streaming methods or centralize the list. |
