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
- Cookie secure flag behavior (HTTPS vs HTTP) `[integration]` **P2**

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
