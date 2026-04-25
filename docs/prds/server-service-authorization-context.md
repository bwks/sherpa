# PRD: Consistent Service Authorization Context

## Status

Draft backlog item.

## Problem

Server services do not use one consistent authorization boundary. Some handlers and WebSocket RPC functions perform ownership/admin checks before calling services. Some services also perform ownership checks internally. Other services accept request structs containing `username` and assume the caller provided a trustworthy identity.

This makes it difficult to know whether a service is safe to call from a new transport or background path. It also creates duplicated authorization logic and leaves room for future mistakes where unverified user-controlled request fields are treated as authenticated identity.

## Findings

- `services/up.rs` contains a TODO noting that `UpRequest.username` is accepted without authentication at the service boundary.
- `services/destroy.rs` contains a similar TODO and also performs an internal ownership check against the database.
- `services/inspect.rs` performs an internal ownership check against the database.
- WebSocket RPC handlers in `api/websocket/rpc.rs` repeatedly authenticate and check `auth_ctx.can_access(owner_username)` before service calls.
- HTML and REST handlers in `api/handlers.rs` also perform ownership checks for lab/node operations.
- Admin-only behavior is split between `AdminUser` extractors, `require_admin_auth`, and RPC helper functions such as `require_admin` / `require_admin_streaming`.
- `AuthContext` already exists in `auth/context.rs`, but it is not consistently passed into services.
- Shared API request types include username fields for some operations, which makes it easy to confuse user input with verified identity.

## Goals

- Establish a single consistent pattern for passing authenticated identity to services.
- Stop trusting caller-supplied usernames as proof of identity.
- Make service functions safe to call from REST, HTML, WebSocket, CLI-backed paths, and future background workflows.
- Reduce duplicated ownership/admin checks across handlers and RPC dispatchers.
- Preserve operation-specific authorization rules.

## Non-goals

- Replacing the route-group authorization work.
- Removing JWTs or changing login behavior.
- Rewriting all API request/response structs in one breaking release.
- Removing all authorization checks from handlers immediately.

## Proposed solution

Introduce a verified service authorization context and make service functions accept it explicitly.

Recommended model:

- Keep `AuthContext` or introduce a server-specific `ServiceAuthContext` containing:
  - verified username;
  - admin flag;
  - optional correlation/request ID;
  - optional transport/source metadata for audit logging.
- Handlers and RPC dispatchers authenticate the request and construct this context.
- Services use this context for owner/admin checks and ignore any user-controlled `username` fields in request bodies.
- Request structs remain input payloads; identity comes only from auth context.
- Add shared helper functions for common authorization decisions:
  - can access lab;
  - require lab owner or admin;
  - require admin;
  - require self or admin for user operations.

## Functional requirements

- Lifecycle services must accept verified auth context, not infer identity from request payloads.
- Lab operations must consistently enforce owner-or-admin access.
- Admin operations must consistently enforce admin access.
- User operations must consistently enforce self-or-admin where applicable.
- Service-level authorization failures must map cleanly to REST 403 and JSON-RPC access-denied errors.
- Any retained `username` fields in public request structs must be treated as compatibility/input fields, not authority.
- Tests must cover direct service calls where possible to ensure services cannot be misused by a new transport.

## Non-functional requirements

- Minimize public API breakage in the first iteration.
- Keep error messages useful but avoid leaking sensitive ownership information to unauthorized users.
- Keep instrumentation fields useful, especially username, lab ID, and operation type.
- Avoid adding circular dependencies between `shared`, `server`, and infrastructure crates.

## Action plan

### Phase 1: Authorization inventory

- List all service entry points under `crates/server/src/services`.
- Classify each as public, authenticated, owner-or-admin, self-or-admin, or admin-only.
- Identify services that currently trust request usernames.
- Identify handlers/RPC paths that duplicate ownership checks.

### Phase 2: Context design

- Decide whether to extend existing `AuthContext` or add a `ServiceAuthContext` in the server crate.
- Define helper methods for common policies.
- Define a service-level authorization error type or standard `anyhow` context convention that maps reliably through `ApiError` and RPC errors.

### Phase 3: Migrate high-risk services first

Priority order:

1. `up_lab` because it creates resources and currently accepts `UpRequest.username`.
2. `destroy_lab` because it deletes resources and mixes caller username plus internal ownership check.
3. `redeploy_node` because it mutates one node inside an existing lab.
4. `down` / `resume` because they mutate runtime state.
5. `inspect` / `download` because they expose lab data.
6. Image and user admin services.

### Phase 4: Transport cleanup

- Update REST handlers to pass verified context into services.
- Update WebSocket RPC dispatch to pass verified context into services.
- Update HTML handlers to pass verified context into services.
- Remove duplicated owner/admin checks only after service-level tests and route-group tests are in place.

### Phase 5: Compatibility cleanup

- Decide whether public request structs should retain `username` fields.
- If retained, document them as ignored/deprecated for authenticated server operations.
- If removed, plan a versioned API migration.

## Testing plan

- Unit tests for auth helper functions.
- Service-level tests with non-admin owner, non-owner, and admin contexts.
- Regression tests proving a forged request username cannot create/destroy/inspect another user's lab.
- REST integration tests for 403 behavior.
- WebSocket RPC tests for access-denied behavior.
- HTML handler tests for owner/admin access where existing test infrastructure supports it.

## Acceptance criteria

- All mutating lab services receive verified auth context.
- No service treats request-body `username` as authenticated identity.
- Owner-or-admin checks are implemented consistently for lab operations.
- Admin-only services cannot be called successfully with a non-admin context.
- Tests cover forged-username cases.
- The TODO comments in `up.rs` and `destroy.rs` are removed because the issue is resolved.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Large signature churn across handlers and RPC code | Migrate service groups incrementally with compatibility wrappers if needed. |
| API request structs are shared with CLI and clients | Do not remove fields until a versioned migration is planned. |
| Duplicate checks cause inconsistent error semantics during migration | Define a single service-level auth error mapping before removing handler checks. |
| Admin behavior changes accidentally | Add owner/admin matrix tests before refactor. |
