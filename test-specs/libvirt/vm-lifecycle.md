# VM Lifecycle — Test Specifications

> **Crate:** `crates/libvirt/`
> **External Dependencies:** Running libvirt daemon with QEMU/KVM
> **Existing Tests:** None

---

## QEMU Connection

**What to test:**
- `Qemu::connect()` establishes connection to qemu:///system `[integration]` **P0**
- Connection failure when libvirt daemon is not running `[integration]` **P0**
- QemuConnection implements Send + Sync for async use `[unit]` **P1**
- Drop implementation properly closes connection `[integration]` **P1**
- libvirt error handler callback suppressed `[integration]` **P2**

---

## VM Creation

**What to test:**
- `create_vm()` defines domain from XML and starts it `[integration]` **P0**
- Valid domain XML produces running VM `[integration]` **P0**
- Invalid domain XML returns error `[integration]` **P0**
- Domain already defined with same name handled `[integration]` **P0**
- VM transitions to running state after creation `[integration]` **P1**

---

## Management IP Retrieval

**What to test:**
- `get_mgmt_ip()` returns IP when guest agent is available `[integration]` **P0**
- Returns None when guest agent is unavailable `[integration]` **P0**
- Returns first IP from first interface (management assumption) `[integration]` **P1**
- Domain not found returns error `[integration]` **P0**
- VM powered off returns no IP `[integration]` **P1**

---

## Lifecycle Race Conditions

**What to test:**
- VM destroyed between state check and operation handled `[integration]` **P1**
- Concurrent create requests for same domain name handled `[integration]` **P2**
- Connection lost during operation produces clear error `[integration]` **P1**
