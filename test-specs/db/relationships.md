# Database Relationships — Test Specifications

> **Crate:** `crates/db/`
> **External Dependencies:** Running SurrealDB instance
> **Existing Tests:** Cascade deletes partially tested via CRUD tests

---

## Foreign Key Integrity

**What to test:**
- Node references valid lab (lab FK) `[integration]` **P0**
- Node references valid node_image (image FK) `[integration]` **P0**
- Link references valid nodes (node_a, node_b FKs) `[integration]` **P0**
- Link references valid lab (lab FK) `[integration]` **P0**
- Bridge references valid lab and nodes `[integration]` **P0**
- Lab references valid user (user FK) `[integration]` **P0**
- Creating node with nonexistent lab reference handled `[integration]` **P0**
- Creating node with nonexistent image reference handled `[integration]` **P0**

---

## Cascade Delete Behavior

**What to test:**
- Deleting a lab cascades to all its nodes `[integration]` **P0**
- Deleting a lab cascades to all its links `[integration]` **P0**
- Deleting a lab cascades to all its bridges `[integration]` **P0**
- Deleting a user cascades to all their labs (and transitively nodes/links/bridges) `[integration]` **P0**
- Deleting a node cascades to its links `[integration]` **P0**
- Bridge node references unset (not cascade) when node deleted `[integration]` **P1**

---

## Reference Rejection

**What to test:**
- Deleting a node_image that is referenced by nodes is rejected `[integration]` **P0**
- Error message identifies the blocking reference `[integration]` **P1**

---

## Computed Reverse References

**What to test:**
- Lab computed fields: nodes, links, bridges populated correctly `[integration]` **P1**
- User computed field: labs populated correctly `[integration]` **P1**
- Node computed fields: links, bridges populated correctly `[integration]` **P1**
- NodeImage computed field: nodes populated correctly `[integration]` **P1**

---

## Cross-Table Query Correctness

**What to test:**
- List nodes by lab returns only nodes in that lab `[integration]` **P0**
- List links by lab returns only links in that lab `[integration]` **P0**
- List links by node returns only links involving that node `[integration]` **P0**
- List labs by user returns only labs owned by that user `[integration]` **P0**
- Count queries match actual record counts `[integration]` **P1**

---

## Authorization Checks

**What to test:**
- Lab ownership verified before operations (get_lab_owner_username) `[integration]` **P0**
- Non-owner cannot access another user's lab data `[integration]` **P0**
- Admin bypass for lab operations (if applicable) `[integration]` **P1**
