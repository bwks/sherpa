# Storage Pools — Test Specifications

> **Crate:** `crates/libvirt/`
> **External Dependencies:** Running libvirt daemon, filesystem permissions
> **Existing Tests:** None

---

## Pool Creation

**What to test:**
- `SherpaStoragePool::create()` defines directory-based pool `[integration]` **P0**
- Pool directory created on filesystem `[integration]` **P0**
- Pool is active (started) after creation `[integration]` **P0**
- Pool has autostart enabled `[integration]` **P0**
- Idempotent: pool already exists — no error, logs debug message `[integration]` **P0**
- Directory creation failure (insufficient permissions) produces error `[integration]` **P1**
- Pool name and path set correctly `[integration]` **P1**

---

## Pool Refresh

**What to test:**
- Pool refresh detects new volumes added externally `[integration]` **P1**
- Pool refresh on empty pool succeeds `[integration]` **P2**
