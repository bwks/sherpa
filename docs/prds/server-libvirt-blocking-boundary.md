# PRD: Blocking-Safe libvirt Boundary

## Status

Draft backlog item.

## Problem

The `virt`/libvirt APIs used by Sherpa are blocking FFI-style calls. Some server code correctly isolates these calls with `tokio::task::spawn_blocking`, but other async services still call broad libvirt operations directly. Direct blocking calls inside async tasks can starve Tokio worker threads during slow libvirt operations, large domain/network scans, storage-pool operations, or daemon timeouts.

Sherpa needs a consistent blocking-safe boundary for all libvirt interactions.

## Findings

- `services/scanner.rs` explicitly documents that libvirt calls are blocking and uses `spawn_blocking` in `query_libvirt_states`.
- `services/down.rs` and `services/resume.rs` use `spawn_blocking` around node power operations.
- `services/node_ops.rs` uses `spawn_blocking` for several VM create/delete/disk operations.
- `services/inspect.rs` calls libvirt operations such as `list_all_domains` and storage-pool lookup/listing from an async service path without an obvious blocking wrapper.
- `services/destroy.rs` calls libvirt domain, network, storage-pool, and disk operations from async service code through synchronous helper functions.
- `services/up.rs` creates some libvirt networks directly from the async creation path.
- The `libvirt` crate exposes synchronous APIs around the `virt` crate. This is acceptable for the crate boundary, but server async code needs a consistent execution strategy.

## Goals

- Ensure blocking libvirt calls do not run on Tokio core worker threads.
- Provide one standard pattern for server code to call libvirt safely.
- Limit concurrency against libvirt to avoid overload and lifecycle races.
- Preserve detailed libvirt error context.
- Make future libvirt call sites easy to review.

## Non-goals

- Replacing the `virt` crate.
- Making libvirt itself asynchronous.
- Rewriting all VM orchestration logic in one large change.
- Changing Docker/Bollard async behavior.

## Proposed solution

Introduce a server-side libvirt execution boundary. All server code that calls blocking libvirt operations should go through this boundary or a clearly named helper that uses it.

Recommended design options:

1. **Libvirt executor service**
   - A small server module that owns a semaphore and wraps closures in `spawn_blocking`.
   - Provides consistent tracing fields and timeout behavior.
   - Used by services for direct libvirt calls.

2. **Async wrappers in the server service layer**
   - Each service exposes async helper functions that internally use `spawn_blocking`.
   - Lower upfront abstraction, but easier to drift again.

3. **Async facade in the `libvirt` crate**
   - The crate exposes async methods backed by `spawn_blocking`.
   - More reusable, but mixes Tokio concerns into the infrastructure crate.

Recommended first step: server-side executor service. It keeps Tokio-specific runtime decisions in the server crate and avoids forcing async onto the libvirt crate API.

## Functional requirements

- All broad libvirt scans must use the blocking-safe boundary:
  - list domains;
  - list networks;
  - storage-pool lookup;
  - storage volume listing.
- All libvirt lifecycle mutations must use the blocking-safe boundary:
  - create/destroy/undefine domains;
  - create/destroy/undefine networks;
  - clone/resize/delete disks;
  - lookup and state checks where they may block.
- The boundary must preserve error chains and context.
- The boundary must emit useful tracing spans with operation type and relevant IDs.
- The boundary must allow concurrency limiting.
- Blocking-safe behavior must be testable or at least audit-testable with call-site checks.

## Non-functional requirements

- Avoid unbounded `spawn_blocking` fan-out during large lab operations.
- Do not hold async locks across blocking libvirt calls.
- Keep destructive operations explicit and observable.
- Maintain compatibility with existing `libvirt` crate types and error handling.
- Avoid adding new external dependencies unless justified.

## Action plan

### Phase 1: Audit

- Inventory every server call site that touches `virt` types or `libvirt` crate functions.
- Classify call sites as:
  - already blocking-safe;
  - direct blocking read;
  - direct blocking mutation;
  - direct blocking cleanup/destructive operation.
- Document high-risk paths, especially inspect, destroy, clean, up, and redeploy.

### Phase 2: Boundary design

- Add a server-side libvirt executor abstraction.
- Include:
  - `spawn_blocking` wrapper;
  - operation labels for tracing;
  - optional timeout policy;
  - semaphore-based concurrency limit;
  - consistent error context.
- Decide whether the executor lives in `AppState` or is a lightweight module used by services.

### Phase 3: Migrate read paths

- Move scanner to use the standard boundary if needed.
- Move inspect domain/storage queries to the standard boundary.
- Add tests or review checks that inspect no longer calls blocking libvirt APIs directly from async code.

### Phase 4: Migrate mutation paths

- Move destroy VM/network/storage operations behind the boundary.
- Move up/redeploy network creation and domain operations behind the boundary where not already covered.
- Move any remaining node_ops direct calls behind the boundary.

### Phase 5: Guardrails

- Add documentation in the libvirt crate and server architecture docs explaining the blocking boundary.
- Add lint-like tests or source checks if practical, for example searching server services for direct `virt::` call sites not in approved modules.
- Add tracing metrics for libvirt operation duration if useful.

## Testing plan

- Unit tests for executor timeout/error propagation where possible.
- Service tests for paths migrated to async wrappers using fake closures.
- Regression tests for inspect/destroy code paths to ensure they still return the same domain/network/disk summaries.
- Source-level guard test or CI check to flag direct blocking libvirt calls outside approved wrappers.
- Manual or integration validation against real libvirt guarded by environment/feature flags.

## Acceptance criteria

- All direct libvirt calls from async service functions are either removed or explicitly wrapped in the blocking boundary.
- Inspect and destroy no longer run broad libvirt scans directly on async worker threads.
- The scanner continues to work and uses the same standard pattern.
- Libvirt operation errors retain detailed context.
- Concurrency against libvirt is bounded.
- Future direct `virt::` calls in server services are documented as exceptions or fail a guard test.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Wrapping changes lifetimes of libvirt connection/domain types | Keep owned data extraction inside the blocking closure and return plain Rust data. |
| Too low concurrency slows lab creation | Start with a conservative configurable/default limit and measure. |
| Too high concurrency still overloads libvirt | Use semaphore and tracing to tune. |
| Timeouts leave libvirt operations running in blocking threads | Treat timeouts as caller timeouts and document that blocking work may finish later; avoid unsafe cancellation. |
