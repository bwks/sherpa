# Container Networking — Test Specifications

> **Crate:** `crates/container/`
> **External Dependencies:** Running Docker daemon
> **Existing Tests:** None

---

## Network Creation

**What to test:**
- `create_docker_bridge_network()` creates bridge-mode network `[integration]` **P0**
- `create_docker_macvlan_network()` creates macvlan network `[integration]` **P0**
- `create_docker_macvlan_bridge_network()` creates macvlan bridge network `[integration]` **P1**
- Duplicate network name handling (error or idempotent) `[integration]` **P0**
- Network created with correct driver and subnet `[integration]` **P1**

---

## Network Deletion

**What to test:**
- `delete_network()` removes Docker network `[integration]` **P0**
- Deleting network in use by running container fails gracefully `[integration]` **P0**
- Deleting nonexistent network handled gracefully `[integration]` **P1**

---

## Network Listing

**What to test:**
- `list_networks()` returns all Docker networks `[integration]` **P0**
- Default Docker networks (bridge, host, none) included in results `[integration]` **P2**
