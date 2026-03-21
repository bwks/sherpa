# Host Networking — Test Specifications

> **Crate:** `crates/network/`
> **External Dependencies:** Linux kernel, elevated privileges (CAP_NET_ADMIN), rtnetlink
> **Existing Tests:** None

---

## Bridge Creation

**What to test:**
- `create_bridge()` creates Linux bridge interface `[integration]` **P0**
- Bridge created with MTU 9600 (jumbo frames) `[integration]` **P0**
- Bridge interface set to UP state `[integration]` **P1**
- Duplicate bridge name handling `[integration]` **P0**
- Bridge visible via system network tools after creation `[integration]` **P1**

---

## Veth Pair Creation

**What to test:**
- `create_veth_pair()` creates virtual ethernet pair `[integration]` **P0**
- Both ends of veth pair created and accessible `[integration]` **P0**
- Veth interfaces set to UP state `[integration]` **P1**
- Duplicate name handling `[integration]` **P1**

---

## Bridge Enslaving

**What to test:**
- `enslave_to_bridge()` attaches interface to bridge `[integration]` **P0**
- Enslaving to nonexistent bridge fails with error `[integration]` **P0**
- Enslaving nonexistent interface fails with error `[integration]` **P0**
- Interface traffic passes through bridge after enslaving `[integration]` **P2**

---

## Interface Deletion

**What to test:**
- `delete_interface()` removes network interface `[integration]` **P0**
- Deleting nonexistent interface handled gracefully `[integration]` **P1**
- Deleting one end of veth pair removes both ends `[integration]` **P1**
- Interface no longer visible after deletion `[integration]` **P0**

---

## Fuzzy Interface Matching

**What to test:**
- `find_interfaces_fuzzy()` finds interfaces matching pattern `[integration]` **P0**
- Returns empty list when no matches `[integration]` **P0**
- Returns multiple matches when pattern is broad `[integration]` **P1**
- Partial name match works correctly `[integration]` **P1**

---

## Netlink Connection

**What to test:**
- Netlink connection established and handle spawned on Tokio runtime `[integration]` **P1**
- Operations fail gracefully if netlink unavailable `[integration]` **P1**
- Insufficient privileges produce clear error `[integration]` **P0**
