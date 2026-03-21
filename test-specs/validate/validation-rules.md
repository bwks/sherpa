# Validate Crate — Test Specifications

> **Crate:** `crates/validate/`
> **External Dependencies:** None (pure logic), except `validate_and_resolve_node_versions` which checks filesystem and Docker image lists
> **Existing Tests:** 42 unit tests across 6 of 7 modules

---

## TCP Connectivity — `connection.rs`

**Function:** `tcp_connect(address, port) -> Result<bool>`

**What to test:**
- Successful connection to a listening socket returns `true` `[integration]` **P1**
- Connection to a closed port returns `false` `[integration]` **P1**
- Invalid address format returns an error `[unit]` **P1**
- Connection respects the 100ms timeout `[integration]` **P2**

**Existing coverage:** None

---

## Duplicate Device Detection — `device.rs`

**Function:** `check_duplicate_device(devices) -> Result<()>`

**What to test:**
- Unique device names pass `[unit]` **P0**
- Duplicate device names fail with the offending name in the error `[unit]` **P0**
- Empty device list passes `[unit]` **P2**
- Single device passes `[unit]` **P2**

**Existing coverage:** All 4 cases covered

---

## Environment Variable Validation — `environment.rs`

**Function:** `validate_environment_variables(entries, node_name) -> Result<()>`

**What to test:**
- Valid `KEY=VALUE` entries pass (literals and `$VAR` references) `[unit]` **P0**
- Empty value (`KEY=`) is accepted `[unit]` **P1**
- Value containing `=` is accepted (only splits on first `=`) `[unit]` **P1**
- Missing `=` separator fails `[unit]` **P0**
- Empty key (`=value`) fails `[unit]` **P0**
- Key starting with a digit fails `[unit]` **P1**
- Key with invalid characters (dashes, dots, spaces) fails `[unit]` **P1**
- Empty entry list passes `[unit]` **P2**
- Error messages include the node name for context `[unit]` **P1**

**Existing coverage:** All cases covered

---

## IPv6 Address Validation — `ipv6.rs`

**Function:** `validate_manifest_ipv6_addresses(nodes) -> Result<()>`

**What to test:**
- Valid unicast IPv6 addresses pass `[unit]` **P0**
- Nodes without an IPv6 address are skipped `[unit]` **P0**
- Unspecified address (`::`) is rejected `[unit]` **P0**
- Loopback address (`::1`) is rejected `[unit]` **P0**
- Multicast addresses (`ff00::/8`) are rejected `[unit]` **P0**
- Multiple nodes validated — first invalid address triggers the error `[unit]` **P1**
- Error messages include the node name and address `[unit]` **P1**

**Existing coverage:** 5 tests cover core cases. Gap: multi-node validation and error message content assertions.

---

## Link & Bridge Validation — `link.rs`

### Management Interface Protection

**Function:** `check_mgmt_usage(device_name, mgmt_interface_index, links, bridges) -> Result<()>`

**What to test:**
- Management interface (index 0) used in a point-to-point link is rejected `[unit]` **P0**
- Management interface used in a bridge connection is rejected `[unit]` **P0**
- Non-management interfaces are allowed in both links and bridges `[unit]` **P0**
- Device not involved in any links/bridges passes `[unit]` **P1**

**Existing coverage:** Covered

---

### Duplicate Interface Detection

**Function:** `check_duplicate_interface_link(links, bridges) -> Result<()>`

**What to test:**
- Each interface used exactly once across all links and bridges passes `[unit]` **P0**
- Same interface used in two point-to-point links fails `[unit]` **P0**
- Same interface used in a link and a bridge fails `[unit]` **P0**
- Same interface used in two different bridges fails `[unit]` **P0**
- Different interfaces on the same device are fine `[unit]` **P1**

**Existing coverage:** 2 tests (link+bridge duplicate, multi-bridge duplicate)

---

### Link Device Existence

**Function:** `check_link_device(devices, links) -> Result<()>`

**What to test:**
- All link endpoints reference defined devices passes `[unit]` **P0**
- Link referencing an undefined device fails with the device name in error `[unit]` **P0**
- Empty links list passes `[unit]` **P2**

**Existing coverage:** No dedicated tests

---

### Bridge Device Existence

**Function:** `check_bridge_device(devices, bridges) -> Result<()>`

**What to test:**
- All bridge members reference defined devices passes `[unit]` **P0**
- Bridge referencing an undefined device fails with the device name in error `[unit]` **P0**
- Empty bridges list passes `[unit]` **P2**

**Existing coverage:** 2 tests (valid devices, undefined device)

---

### Interface Bounds Checking

**Function:** `check_interface_bounds(device_name, device_model, data_interface_count, reserved_interface_count, dedicated_management_interface, links, bridges) -> Result<()>`

Index scheme: `0 = mgmt | 1..reserved_count = reserved | (1+reserved_count)..(1+reserved_count+data_count-1) = data`

**What to test:**
- Data interface within valid range passes `[unit]` **P0**
- Interface index exceeding maximum fails `[unit]` **P0**
- Management interface (index 0) rejected when `dedicated_management_interface = true` `[unit]` **P0**
- Management interface (index 0) allowed when `dedicated_management_interface = false` `[unit]` **P0**
- Reserved interfaces (between mgmt and data range) rejected `[unit]` **P0**
- First data interface after reserved range passes `[unit]` **P0**
- Device with only 1 data interface — boundary at eth1/eth2 `[unit]` **P1**
- Device with reserved interfaces — correct max index calculation `[unit]` **P1**
- Links not involving the checked device are ignored `[unit]` **P1**
- Empty links and bridges pass `[unit]` **P2**
- Bridge connections follow the same bounds rules as links `[unit]` **P0**
- Bridge reserved interface rejected `[unit]` **P0**

**Existing coverage:** 18 tests covering all the above cases well

---

## Node Image Field Validation — `node_image.rs`

**Function:** `validate_node_image_update(cpu_count, memory, data_interface_count, interface_mtu, version, interface_prefix) -> Result<()>`

**What to test:**
- All fields valid passes `[unit]` **P0**
- CPU count of 0 fails (minimum is 1) `[unit]` **P0**
- Memory below 64 MB fails `[unit]` **P0**
- Data interface count of 0 fails (minimum is 1) `[unit]` **P0**
- MTU below 576 fails `[unit]` **P0**
- MTU above 9600 fails `[unit]` **P0**
- MTU at boundaries (576 and 9600) passes `[unit]` **P1**
- Empty version (including whitespace-only) fails `[unit]` **P0**
- Version 4 characters or fewer fails `[unit]` **P0**
- Empty interface prefix fails `[unit]` **P0**
- First failing field produces the error (validation order matters) `[unit]` **P2**

**Existing coverage:** 4 tests cover private validators (cpu, memory, mtu, version). Gaps: no test for the public `validate_node_image_update` entry point, no test for `interface_prefix`, no test for `data_interface_count`.

---

## Version Resolution & Image Existence — `version.rs`

**Function:** `validate_and_resolve_node_versions(nodes, node_images, images_dir, docker_images) -> Result<Vec<Node>>`

**What to test:**

### Version Resolution
- Node with explicit version uses that version `[unit]` **P0**
- Node without version falls back to default node_image version `[unit]` **P0**
- Default node_image is preferred when multiple configs exist for a model `[unit]` **P1**
- Version not found in database fails with available versions listed `[unit]` **P0**
- No configurations in database for model fails with specific error `[unit]` **P0**
- Returned nodes have version field populated `[unit]` **P0**

### Container Image Verification
- Container image present in Docker (`repo:version` format) passes `[unit]` **P0**
- Container image missing from Docker fails with `docker pull` hint `[unit]` **P0**
- Container with no repo configured in node_image fails `[unit]` **P0**

### VM/Unikernel Disk Verification
- Disk file exists at `{images_dir}/{model}/{version}/virtioa.qcow2` passes `[unit]` **P0**
- Disk file missing fails with expected path and hint `[unit]` **P0**
- Unikernels use the same disk path validation as VMs `[unit]` **P1**

### Multi-Node Scenarios
- Mixed VM and container nodes in same manifest `[unit]` **P1**
- Multiple nodes of same model with different versions `[unit]` **P1**

**Existing coverage:** 6 tests cover private validators (version in db, container image). Gaps: no test for the public entry point, no test for `validate_vm_disk`, no test for version fallback/resolution logic, no multi-node scenarios.

---

## Coverage Gap Summary

| Area | Status | Priority |
|------|--------|----------|
| `tcp_connect` | No tests | P1 |
| `check_link_device` | No dedicated tests | P0 |
| `validate_node_image_update` (public entry) | Not tested directly | P0 |
| `validate_and_resolve_node_versions` (public entry) | Not tested directly | P0 |
| `validate_vm_disk` | Not tested | P0 |
| `interface_prefix` validation | Not tested | P1 |
| `data_interface_count` validation | Not tested | P1 |
| Multi-node IPv6 validation | Not tested | P2 |
| Error message content assertions | Sparse | P2 |
