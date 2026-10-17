# User Management End-to-End — Test Specifications

> **Scope:** Cross-crate integration testing user workflows
> **External Dependencies:** SurrealDB, running server
> **Existing Tests:** None

---

## User Creation

**What to test:**
- Admin creates new user via RPC → user exists in DB `[e2e]` **P0**
- Created user can log in with provided credentials `[e2e]` **P0**
- Duplicate username rejected `[e2e]` **P0**
- Non-admin cannot create users `[e2e]` **P0**
- Password stored as Argon2id hash (not plaintext) `[e2e]` **P0**

---

## Authentication Flow

**What to test:**
- Login with valid credentials → JWT token returned `[e2e]` **P0**
- Token used for subsequent RPC calls `[e2e]` **P0**
- Token validated on each request (username and admin status) `[e2e]` **P0**
- Expired token rejected with auth error `[e2e]` **P0**
- Invalid credentials return auth error (no token) `[e2e]` **P0**

---

## Password Management

**What to test:**
- User changes own password → old password no longer works `[e2e]` **P0**
- Admin changes another user's password `[e2e]` **P0**
- Password strength requirements enforced on change `[e2e]` **P0**

---

## SSH Key Management

**What to test:**
- User adds SSH public key to profile `[e2e]` **P0**
- Invalid SSH key format rejected `[e2e]` **P0**
- User removes SSH key from profile `[e2e]` **P1**
- SSH keys stored and retrievable `[e2e]` **P0**

---

## Permission Boundaries

**What to test:**
- Admin user can access admin RPC methods `[e2e]` **P0**
- Non-admin user rejected from admin RPC methods `[e2e]` **P0**
- User can only see/manage their own labs `[e2e]` **P0**
- Admin can see/manage all labs `[e2e]` **P0**
- Admin routes (HTTP) reject non-admin users `[e2e]` **P0**

---

## User Deletion

**What to test:**
- Admin deletes user → user removed from DB `[e2e]` **P0**
- Deleted user's labs cascade-deleted `[e2e]` **P0**
- Last admin cannot be deleted (safety check) `[e2e]` **P0**
- Non-admin cannot delete users `[e2e]` **P0**
- Deleted user's token no longer valid `[e2e]` **P1**

---

## User Information

**What to test:**
- `user.info` returns username, admin status, SSH keys `[e2e]` **P0**
- `user.list` returns all users (admin only) `[e2e]` **P0**
