# Database Schema & Seeding — Test Specifications

> **Crate:** `crates/db/`
> **External Dependencies:** None (in-memory embedded database)
> **Existing Tests:** Schema applied as part of all integration test setup, but no dedicated schema tests

---

## Schema Application

**What to test:**
- `apply_schema()` creates all 6 tables (lab, node, link, bridge, user, node_image) `[integration]` **P0**
- Schema is idempotent — applying twice does not fail or corrupt data `[integration]` **P0**
- Field constraints enforced after schema application:
  - User: username 3–32 chars matching `[a-z_][a-z0-9_-]*`, also when written directly with a query `[integration]` **P0**
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

## Built-in Admin Protection

**What to test:**
- Deleting `admin` by id, by username or with a bulk `DELETE user` is refused, and a bulk delete removes nobody `[integration]` **P0**
- Renaming `admin` or clearing its admin flag is refused through `update_user` and direct queries `[integration]` **P0**
- An `admin` record without the admin flag cannot be created `[integration]` **P0**
- `admin`'s password and SSH keys can still change `[integration]` **P0**
- Other admins can still be demoted and deleted `[integration]` **P0**

Tests: `crates/db/tests/user/admin_protection_tests.rs`.

---

## Database Connection

**What to test:**
- `open()` creates the database file, and reopening it keeps the data `[unit]` **P0**
- `open_in_memory()` gives each call its own empty database `[unit]` **P0**
- `apply_schema()` sets `PRAGMA user_version` to `SCHEMA_VERSION` and is idempotent `[unit]` **P0**
- `apply_schema()` refuses a database with a newer schema version `[unit]` **P0**
- `Database::backup()` copies an open database and refuses an existing file `[unit]` **P0**
- Writes from many tasks through one `Database` all succeed (the write lock queues them) `[unit]` **P0**
