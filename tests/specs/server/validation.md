# Server Validation — Test Specifications

> **Module:** `crates/server/src/validate/`
> **External Dependencies:** None (pure logic), except `validate_and_resolve_node_versions` which checks filesystem and Docker image lists

Validators that only the server enforces.

---

## Environment Variable Validation — `validate/environment.rs`

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

## Node Image Field Validation — `validate/node_image.rs`

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

**Existing coverage:** 14 tests — private validators (cpu, memory, interface count, mtu, version, prefix) plus 8 public entry point tests covering all failure modes and boundary values.

---

## Version Resolution & Image Existence — `validate/version.rs`

**Function:** `validate_and_resolve_node_versions(nodes, node_images, images_dir) -> Result<Vec<Node>>`

**What to test:**

### Version Resolution
- Node with explicit version uses that version `[unit]` **P0**
- Node without version falls back to default node_image version `[unit]` **P0**
- Default node_image is preferred when multiple configs exist for a model `[unit]` **P1**
- Version not found in database fails with available versions listed `[unit]` **P0**
- No configurations in database for model fails with specific error `[unit]` **P0**
- Returned nodes have version field populated `[unit]` **P0**

### Container Image Resolution
- A container node's image is the repo its version was pulled from, kept as written:
  `docker.io/library/<name>`, `<name>` and a full third-party path `[unit]` **P0**
- A manifest `image` is honoured, for models with a built-in repo as for any other `[unit]` **P0**
- Container with no repo configured in node_image fails naming both fixes `[unit]` **P0**
- Whether Docker has the image is asked of Docker (`inspect_image`) by `up` and redeploy with the
  same reference the container is created from; an official image pulled by its full name brings
  a node up and redeploys, and a missing image refuses a redeploy before teardown
  (`lab_lifecycle::an_official_docker_hub_image_pulled_by_its_full_name_brings_a_container_node_up`)
  `[integration]` **P0**

### VM/Unikernel Disk Verification
- Disk file exists at `{images_dir}/{model}/{version}/virtioa.qcow2` passes `[unit]` **P0**
- Disk file missing fails with expected path and hint `[unit]` **P0**
- Unikernels use the same disk path validation as VMs `[unit]` **P1**

### Multi-Node Scenarios
- Mixed VM and container nodes in same manifest `[unit]` **P1**
- Multiple nodes of same model with different versions `[unit]` **P1**

**Existing coverage:** private validators (version in db, vm disk), `validate_container_repo`, plus public entry point tests covering version resolution, fallback, precedence, container image resolution, multi-node scenarios, and failure cases.

---

## Tailscale Server Settings — `validate/tailscale_gateway.rs`

- Packaged gateway defaults pass validation `[unit]` **P0**
- Empty or whitespace-containing images fail with the field name `[unit]` **P0**
- Relative, empty, NUL-containing, and oversized socket paths fail `[unit]` **P0**
- Zero timeouts and stop grace periods exceeding Docker's signed integer range fail `[unit]` **P0**

---

## Node Readiness Port Check — `services/node_ops.rs`

**Function:** `check_node_port_ready(ip, port) -> Result<bool>`

**What to test:**
- Successful connection to a listening socket returns `true` `[integration]` **P1**
- Connection to a closed port returns `false` `[integration]` **P1**
- Invalid address format returns an error `[unit]` **P1**
- Connection respects the 100ms timeout `[integration]` **P2**

**Existing coverage:** 2 tests (closed port returns false, invalid address returns error)
