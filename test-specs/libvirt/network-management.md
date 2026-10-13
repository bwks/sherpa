# Libvirt Network Management — Test Specifications

> **Crate:** `crates/libvirt/`
> **External Dependencies:** Running libvirt daemon
> **Existing Tests:** None

---

## Network Types

**What to test for each type (Bridge, Isolated, NAT, Reserved):**
- `create()` defines, starts, and sets autostart for new network `[integration]` **P0**
- Idempotent: network already exists and active — no error `[integration]` **P0**
- Network exists but inactive — started automatically `[integration]` **P1**
- Askama XML template renders valid libvirt network XML `[unit]` **P0**
- Network accessible after creation `[integration]` **P1**

---

## Bridge Network

**What to test:**
- Bridge-mode forwarding enabled in XML `[unit]` **P1**
- Bridge name and network name set correctly `[unit]` **P1**

---

## Isolated Network

**What to test:**
- No forwarding mode in XML (truly isolated) `[unit]` **P1**
- Cannot reach external networks from isolated network `[integration]` **P2**

---

## NAT Network

**What to test:**
- NAT-mode forwarding enabled `[unit]` **P1**
- DHCP range configured if applicable `[unit]` **P1**

---

## Reserved Network

**What to test:**
- Reserved network created with correct parameters `[unit]` **P1**

---

## Network Activation

**What to test:**
- `ensure_network_active()` returns true for existing active network `[integration]` **P0**
- Returns true and starts inactive network `[integration]` **P0**
- Returns false for nonexistent network `[integration]` **P0**
- Autostart set to true on creation `[integration]` **P1**
