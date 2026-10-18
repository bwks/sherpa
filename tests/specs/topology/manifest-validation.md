# Manifest Validation — Test Specifications

> **Module:** `crates/topology/src/validate/`
> **External Dependencies:** None (pure logic)
> **Callers:** client `sherpa validate` / `sherpa up`, and server `up` / `redeploy`

These checks take manifest types (`Node`, `LinkDetailed`, `BridgeDetailed`, `Manifest`) and
are shared by the client and the server.

---

## Duplicate Device Detection — `validate/device.rs`

**Function:** `check_duplicate_device(devices) -> Result<()>`

**What to test:**
- Unique device names pass `[unit]` **P0**
- Duplicate device names fail with the offending name in the error `[unit]` **P0**
- Empty device list passes `[unit]` **P2**
- Single device passes `[unit]` **P2**

**Existing coverage:** All 4 cases covered

**Function:** `check_device_count(devices) -> Result<()>`

**What to test:**
- A manifest that fills the management node block (234 nodes) passes `[unit]` **P0**
- One node more fails with the count and the limit in the error `[unit]` **P0**

**Existing coverage:** Both cases covered

---

## IPv6 Address Validation — `validate/ipv6.rs`

**Function:** `validate_manifest_ipv6_addresses(nodes) -> Result<()>`

**What to test:**
- Valid unicast IPv6 addresses pass `[unit]` **P0**
- Nodes without an IPv6 address are skipped `[unit]` **P0**
- Unspecified address (`::`) is rejected `[unit]` **P0**
- Loopback address (`::1`) is rejected `[unit]` **P0**
- Multicast addresses (`ff00::/8`) are rejected `[unit]` **P0**
- Multiple nodes validated — first invalid address triggers the error `[unit]` **P1**
- Error messages include the node name and address `[unit]` **P1**

**Existing coverage:** 8 tests cover all cases including multi-node and error message assertions.

---

## Link & Bridge Validation — `validate/link.rs`

### Management Interface Protection

**Function:** `check_mgmt_usage(device_name, mgmt_interface_index, links, bridges) -> Result<()>`

**What to test:**
- Management interface (index 0) used in a point-to-point link is rejected `[unit]` **P0**
- Management interface used in a bridge connection is rejected `[unit]` **P0**
- Non-management interfaces are allowed in both links and bridges `[unit]` **P0**
- Device not involved in any links/bridges passes `[unit]` **P1**

**Existing coverage:** Covered
