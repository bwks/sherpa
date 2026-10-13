# Database CRUD Operations — Test Specifications

> **Crate:** `crates/db/`
> **External Dependencies:** Running SurrealDB instance
> **Existing Tests:** 28 integration test files covering CRUD for all tables

---

## Lab Table

**What to test:**
- Create lab with all required fields `[integration]` **P0**
- Create lab with duplicate lab_id rejected `[integration]` **P0**
- Create lab with duplicate (name, user) pair rejected `[integration]` **P0**
- Get lab by record ID, by lab_id, by name+user `[integration]` **P0**
- List all labs, list by user, count labs, count by user `[integration]` **P0**
- Update lab fields (name, networks) `[integration]` **P0**
- Delete lab by record ID, by lab_id `[integration]` **P0**
- `delete_lab_cascade()` removes lab and all child records `[integration]` **P0**
- `delete_lab_safe()` refuses if child records exist `[integration]` **P1**
- `validate_lab_id()` checks format/uniqueness `[integration]` **P1**
- `get_lab_owner_username()` returns correct owner `[integration]` **P1**
- Network allocation queries: `get_used_management_networks()`, `get_used_ipv6_management_networks()` `[integration]` **P1**

**Existing coverage:** Comprehensive CRUD tests exist

---

## Node Table

**What to test:**
- Create node with required fields (name, index, lab, image) `[integration]` **P0**
- Duplicate (lab, name) rejected `[integration]` **P0**
- Duplicate (lab, index) rejected `[integration]` **P0**
- Get node by ID, by name+lab `[integration]` **P0**
- List all nodes, list by lab, count by lab `[integration]` **P0**
- Update node fields `[integration]` **P0**
- Update management IP/MAC individually: `update_node_mgmt_ipv4()`, `update_node_mgmt_ipv6()`, `update_node_mgmt_mac()` `[integration]` **P0**
- `update_node_state()` transitions state correctly `[integration]` **P0**
- Delete node by ID `[integration]` **P0**
- `delete_node_cascade()` removes node and its links `[integration]` **P0**
- `delete_nodes_by_lab()` bulk delete `[integration]` **P1**

**Existing coverage:** Comprehensive CRUD tests exist

---

## Link Table

**What to test:**
- Create link with all fields (node_a, node_b, interfaces, bridge names) `[integration]` **P0**
- Unique (node_a, node_b, int_a, int_b) enforced `[integration]` **P0**
- Get by ID, get by peers `[integration]` **P0**
- List all, list by lab, list by node `[integration]` **P0**
- Count all, count by lab, count by node `[integration]` **P1**
- Update link fields `[integration]` **P0**
- Delete by ID, delete by lab, delete by node `[integration]` **P0**

**Existing coverage:** Comprehensive CRUD tests exist

---

## Bridge Table

**What to test:**
- Create bridge with index, name, lab, nodes `[integration]` **P0**
- Unique (index, lab) enforced `[integration]` **P0**
- Get by record, get by index `[integration]` **P0**
- List all bridges, list by lab `[integration]` **P0**
- Delete bridge, delete all bridges by lab `[integration]` **P0**

**Existing coverage:** Partial (tested as part of lab operations)

---

## User Table

**What to test:**
- Create user with username, password_hash, is_admin `[integration]` **P0**
- Unique username enforced `[integration]` **P0**
- Username validation (min 3 chars, allowed characters) `[integration]` **P0**
- `get_user_for_auth()` returns user with password_hash for login `[integration]` **P0**
- List users, count users `[integration]` **P0**
- Update user (password, SSH keys, admin status) `[integration]` **P0**
- Delete user by username `[integration]` **P0**
- `delete_user_safe()` prevents deleting last admin `[integration]` **P0**

**Existing coverage:** Comprehensive CRUD tests exist

---

## NodeImage Table

**What to test:**
- Create node_image with model, kind, version, and 29 config fields `[integration]` **P0**
- Unique (model, kind, version) enforced `[integration]` **P0**
- `upsert_node_image()` creates or updates `[integration]` **P0**
- Get by ID, by model+kind+version `[integration]` **P0**
- `get_default_node_image()` returns image with default=true `[integration]` **P0**
- `set_default_image()` changes default flag `[integration]` **P1**
- List by kind, list by IDs, get versions for model `[integration]` **P0**
- Update image config fields `[integration]` **P0**
- Delete image (fails if nodes reference it) `[integration]` **P0**

**Existing coverage:** Comprehensive CRUD tests exist
