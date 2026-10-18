# Server Authentication — Test Specifications

> **Crate:** `crates/server/` (`auth/`, `api/extractors.rs`)
> **External Dependencies:** Running SurrealDB
> **Existing Tests:** None

---

## JWT Token Handling

**What to test:**
- Valid JWT accepted by middleware `[integration]` **P0**
- Expired JWT rejected with appropriate error `[integration]` **P0**
- Malformed JWT rejected `[integration]` **P0**
- Missing JWT produces unauthenticated error `[integration]` **P0**
- JWT claims contain correct username and is_admin flag `[unit]` **P0**
- Token created with 7-day expiry `[unit]` **P1**

---

## Cookie-Based Sessions

**What to test:**
- Login sets authentication cookie `[integration]` **P0**
- Cookie used for subsequent HTTP requests `[integration]` **P0**
- Logout clears cookie `[integration]` **P0**
- Expired cookie rejected `[integration]` **P1**
- Expired or invalid cookie is cleared before redirecting to login `[integration]` **P0**
- Duplicate auth cookies accept a valid replacement regardless of ordering `[unit]` **P0**
- Stale cookie followed by successful login can access a protected page `[integration]` **P0**
- Cookie secure flag behavior (HTTPS vs HTTP) `[integration]` **P2**
- Session tracing records safe validation and recovery metadata without raw tokens or passwords `[unit]` **P0**

---

## Auth Extractors

**What to test:**
- `AuthenticatedUser` extractor validates token and populates auth context `[integration]` **P0**
- `AdminUser` extractor rejects non-admin users `[integration]` **P0**
- `AdminUser` extractor accepts admin users `[integration]` **P0**
- Auth context carries username and is_admin correctly `[integration]` **P0**
- Missing auth header returns 401 `[integration]` **P0**

---

## Login/Signup Flow

**What to test:**
- Login with valid credentials returns JWT token `[integration]` **P0**
- Login with invalid password returns error `[integration]` **P0**
- Login with nonexistent username returns error `[integration]` **P0**
- Signup creates new user and returns token `[integration]` **P0**
- Signup with existing username fails `[integration]` **P0**
- Password strength requirements enforced at signup `[integration]` **P1**

---

## RPC Auth Middleware

**What to test:**
- WebSocket RPC requests validated against auth context `[integration]` **P0**
- Admin-only RPC methods reject non-admin callers `[integration]` **P0**
- Auth token passed in RPC request parameters `[integration]` **P1**

## Redeploy ownership

- Only the current database lab owner may redeploy; admins have no bypass `[unit]` `[integration]` **P0**
- Non-owner regular users and admins are denied before node state changes or teardown across WebSocket, JSON/SSE and cookie routes `[integration]` **P0**
- Authorization uses database ownership; caller-supplied usernames are ignored `[integration]` **P0**
- An admin who owns the lab may redeploy; other admin operations retain their access rules `[unit]` `[integration]` **P0**

The ignored `redeploy_only_allows_current_owner_on_all_transports` test in
`lab_lifecycle_tests.rs` uses the disposable test database and local API sockets.
It creates database metadata only and does not start or destroy lab resources.

Start the disposable database with `SHERPA_DEV_DB_USER=sherpa`, matching the
account used by the DB client, and supply its test password to the test runner:

```sh
SHERPA_DEV_DB_USER=sherpa dev/testdb start
SHERPA_DB_PASSWORD="${SHERPA_DEV_DB_PASS:-root}" cargo test -p sherpad --test lab_lifecycle_tests redeploy_only_allows_current_owner_on_all_transports -- --ignored --nocapture
```

## Database-backed identity

- The admin flag comes from the user record, not the token: a demoted admin's existing token loses admin rights on RPC, REST and web `[integration]` **P0**
- A valid token for a deleted user is rejected: RPC `AuthRequired`, REST 401, web session cleared `[integration]` **P0**

Tests: `demoted_admin_token_loses_admin_rights_on_all_transports` and
`deleted_user_token_is_rejected_on_all_transports` in `auth_tests.rs`.

## Lab operation authorization

- Owner-or-admin operations (inspect, destroy, down, resume, download, link impairment) allow the owner and admins and refuse other users, on RPC, REST and every web route `[integration]` **P0**
- Refused streaming requests send no progress, and a refused destroy leaves the lab `[integration]` **P0**
- Authorization failures are typed (`ServiceError`) and map to 403/404/409 and RPC `AccessDenied`/`NotFound`, never by matching error text `[unit]` **P0**
- `GET /api/v1/labs` requires login; regular users list only their own labs; admins may pass `?username=` `[integration]` **P0**
- A web job page and stream are served only to the job's creator or an admin, and a refused request does not consume the job `[unit]` `[integration]` **P0**

Tests: `lab_authorization_tests.rs`, `auth::policy` unit tests, `api::errors` unit tests and
`daemon::state` unit tests. The integration tests seed lab records only; they need the
disposable test database and local Docker and libvirt sockets, but no root.

```sh
SHERPA_DEV_DB_USER=sherpa dev/testdb start
SHERPA_DB_PASSWORD="${SHERPA_DEV_DB_PASS:-root}" cargo test -p sherpad --test lab_authorization_tests -- --ignored --test-threads=1
```

## User deletion

- Deleting a user who owns labs is refused on RPC (`ServerError`, labs listed), REST (409) and web (notification), and the user and labs remain `[integration]` **P0**
- Once their labs are gone, the user can be deleted `[integration]` **P0**
- Self-delete is refused on RPC (`AccessDenied`), REST (403) and web (notification) `[integration]` **P0**
- Only admins may delete users; the built-in `admin` and the last admin cannot be deleted `[unit]` `[integration]` **P0**
- The built-in `admin` cannot create, resume or redeploy labs on RPC, REST or web, and can still stop and destroy them `[unit]` `[integration]` **P0** (`built_in_admin_cannot_start_lab_work_on_any_transport`)
- The schema rejects deleting a lab owner, including on a database upgraded from the old cascading definition `[integration]` (db) **P0**

Tests: `deleting_a_user_who_owns_labs_is_refused_on_all_transports` and
`self_delete_is_refused_on_all_transports` in `user_management_tests.rs`, the
`services::user_delete` unit tests, and the db `relationships` tests.
