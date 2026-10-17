# Database Schema & Seeding — Test Specifications

> **Crate:** `crates/db/`
> **External Dependencies:** Running SurrealDB instance
> **Existing Tests:** Schema applied as part of all integration test setup, but no dedicated schema tests

---

## Schema Application

**What to test:**
- `apply_schema()` creates all 6 tables (lab, node, link, bridge, user, node_image) `[integration]` **P0**
- Schema is idempotent — applying twice does not fail or corrupt data `[integration]` **P0**
- Field constraints enforced after schema application:
  - User: username min 3 chars, alphanumeric + @._- `[integration]` **P0**
  - Node: index range 0-65535 `[integration]` **P1**
  - Link: index range 0-65535 `[integration]` **P1**
- Unique indexes created and enforced:
  - User: unique username `[integration]` **P0**
  - Lab: unique lab_id, unique (name, user) pair `[integration]` **P0**
  - Node: unique (lab, name), unique (lab, index) `[integration]` **P0**
  - Link: unique (node_a, node_b, int_a, int_b) `[integration]` **P0**
  - NodeImage: unique (model, kind, version) `[integration]` **P0**
  - Bridge: unique (index, lab) `[integration]` **P1**
- Enum field validation enforced (NodeState, NodeKind, NodeModel, etc.) `[integration]` **P1**

---

## Admin User Seeding

**What to test:**
- `seed_admin_user()` creates admin user on first run `[integration]` **P0**
- Seeding when admin already exists does not duplicate or fail `[integration]` **P0**
- Seeded admin has correct default credentials `[integration]` **P0**
- Seeded admin has `is_admin = true` `[integration]` **P0**
- Password is properly hashed (not stored in plaintext) `[integration]` **P1**

---

## Database Connection

**What to test:**
- `connect()` establishes connection to SurrealDB `[integration]` **P0**
- Connection with invalid credentials fails `[integration]` **P0**
- Connection to unreachable server fails with timeout `[integration]` **P1**
- Namespace and database selection after auth `[integration]` **P1**
