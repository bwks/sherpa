# PRD: Route-Group Authorization

## Status

Draft backlog item.

## Problem

Sherpa server route protection is currently discovered by reading each handler signature and handler body. `api/router.rs` registers public, authenticated, and admin routes in one router, then handlers decide whether a request is allowed by using extractors such as `AuthenticatedUserFromCookie`, `AdminUser`, `AuthenticatedUser`, or manual calls like `require_admin_auth`.

This makes route reviews error-prone. A route can be added to the router without an auth extractor and still compile. The current public `GET /api/v1/labs?username=...` route is an example of behavior that is only obvious after inspecting the handler, not the route table.

## Findings

- `crates/server/src/api/router.rs` builds one mixed Axum router.
- Router-wide layers are CORS and `TraceLayer`; there is no auth route group or auth middleware layer.
- HTML auth is mostly encoded in handler parameters:
  - `AuthenticatedUserFromCookie` redirects unauthenticated browser users.
  - `AdminUser` redirects unauthenticated users and returns a 403 template for non-admin users.
- REST auth is encoded in handler parameters and manual checks:
  - `AuthenticatedUser` accepts bearer tokens or cookies.
  - Admin REST handlers call `require_admin_auth(&auth)` inside the handler.
- WebSocket RPC authorization is separate and lives in `api/websocket/rpc.rs`.
- The generated API spec has auth metadata, but route registration does not validate against it.
- Current code behavior: `GET /api/v1/labs?username=...` is registered as public because `get_labs_json` has no auth extractor.

## Goals

- Make protection level visible from route construction.
- Prevent accidental unauthenticated route additions.
- Keep browser, REST, and WebSocket auth behavior semantically correct.
- Preserve distinct error behavior:
  - HTML unauthenticated: redirect to login.
  - HTML unauthorized admin: 403 page.
  - REST unauthenticated: JSON 401.
  - REST unauthorized: JSON 403.
  - WebSocket RPC: JSON-RPC auth/access error.
- Add route-policy tests that fail when a protected route is accidentally public.

## Non-goals

- Replacing JWT authentication.
- Changing the public API contract without an explicit migration decision.
- Rewriting WebSocket RPC dispatch as part of the first iteration.
- Introducing a second web framework.

## Proposed solution

Introduce explicit route groups in `api/router.rs` and make each group carry its intended auth policy.

Recommended route groups:

| Group | Auth policy | Examples |
|---|---|---|
| Public HTML | no auth | `/login`, `/signup` |
| Authenticated HTML | cookie auth | `/`, `/labs`, `/profile` |
| Admin HTML | cookie auth + admin | `/admin/users`, `/admin/tools`, `/admin/images` |
| Public API | no auth | `/health`, `/cert`, `/api/v1/spec`, `/api/v1/openapi.json`, `/api/docs`, `/api/v1/auth/login` |
| Authenticated API | bearer or cookie | lab inspect/create/destroy, image list/show, user self info/password |
| Admin API | bearer or cookie + admin | image mutations, admin tools, user create/list/delete |

Implementation direction:

1. Create route construction functions for each auth group.
2. Apply group-specific middleware or typed route layers rather than relying only on handler signatures.
3. Keep the existing extractors during migration, but make them redundant rather than the only protection.
4. Add a policy matrix that lists route, method, handler, auth group, and expected unauthenticated/unauthorized behavior.
5. Add tests that exercise the matrix with missing auth and non-admin auth.
6. Decide explicitly whether `GET /api/v1/labs?username=...` remains public or moves to authenticated access.

## Functional requirements

- Every route registered by `build_router()` must belong to exactly one auth group.
- Public routes must be explicitly listed and reviewed.
- Authenticated HTML routes must reject missing/invalid cookies with the existing login redirect behavior.
- Admin HTML routes must reject missing cookies with a login redirect and non-admin cookies with a 403 page.
- Authenticated REST routes must reject missing/invalid auth with a JSON 401 response.
- Admin REST routes must reject non-admin users with a JSON 403 response.
- Route policy metadata must be testable without relying on manual source inspection.
- API spec auth metadata must be checked against the REST route policy where operations are represented in the spec.

## Non-functional requirements

- Must not weaken any existing authenticated/admin route.
- Must preserve existing public endpoints unless a deliberate breaking-change task is created.
- Must keep CORS and tracing behavior intact.
- Must remain compatible with Axum and current Askama/HTMX flows.

## Action plan

### Phase 1: Inventory and policy matrix

- Enumerate all routes from `api/router.rs`.
- Classify each route as public, authenticated, or admin.
- Record expected rejection behavior for HTML and REST separately.
- Flag routes whose current behavior is questionable, especially public lab listing.

### Phase 2: Route grouping

- Split `build_router()` into group-specific router builders.
- Apply auth layers to authenticated/admin groups.
- Keep handler-level extractors temporarily to reduce risk.
- Ensure `/ws` remains added by `daemon/server.rs`, with RPC auth handled separately.

### Phase 3: Tests

- Add route-policy tests for each group.
- Add regression tests for accidental unauthenticated access to protected routes.
- Add tests for non-admin access to admin routes.
- Add a consistency test between REST route policy and `shared::api_spec` auth metadata.

### Phase 4: Cleanup

- Remove redundant manual `require_admin_auth` checks where group middleware fully covers the route.
- Keep explicit ownership checks in operation-specific handlers or move them into service authorization as part of the separate service-auth PRD.
- Update `docs/SERVER.md` after implementation to remove the rough-edge warning.

## Acceptance criteria

- Route registration clearly separates public, authenticated, and admin routes.
- New routes cannot be added without selecting an auth group.
- Tests fail if a protected route is accidentally exposed as public.
- Existing browser redirect behavior is preserved.
- Existing REST JSON 401/403 behavior is preserved.
- WebSocket RPC behavior is unchanged.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| HTML redirects break when moved behind middleware | Add browser-route integration tests before refactor. |
| REST and HTML need different rejection formats | Use separate HTML and REST auth layers. |
| Public endpoint behavior changes accidentally | Maintain a checked public-route allowlist. |
| Duplicated auth checks during migration cause confusing errors | Migrate in phases and remove redundant checks only after tests pass. |
