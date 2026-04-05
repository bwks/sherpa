# Server Services — Test Specifications

> **Crate:** `crates/server/` (`services/`)
> **External Dependencies:** SurrealDB, Docker, libvirt, Linux networking, filesystem
> **Existing Tests:** None

---

## Lab Startup (`up`)

**What to test:**
- Full lab creation from valid manifest (DB records, networks, nodes) `[e2e]` **P0**
- Multi-phase progress reporting (setup, validation, DB, networks, links, bridges, ZTP, creation) `[integration]` **P1**
- Manifest validation failures abort before resource creation `[integration]` **P0**
- IP network allocation (management + loopback subnets, both IPv4 and IPv6) `[integration]` **P0**
- Collision avoidance: used subnets skipped during allocation `[integration]` **P0**
- VM nodes: ZTP generated, disks cloned, domains created `[integration]` **P0**
- Container nodes: ZTP generated, Docker networks created, containers started `[integration]` **P0**
- Point-to-point links: bridges and veth pairs created between nodes `[integration]` **P0**
- Broadcast bridges: multi-node bridge connectivity `[integration]` **P0**
- SSH readiness wait with timeout `[integration]` **P1**
- Lab already exists produces error `[integration]` **P0**
- Partial failure triggers cleanup/rollback `[integration]` **P0**
- DB operations use shared AppState connection, not ephemeral connections `[unit]` **P0**
- Image not found in DB or filesystem produces error `[integration]` **P0**

---

## Lab Destruction (`destroy`)

**What to test:**
- All containers killed and removed `[integration]` **P0**
- All VMs undefined and disks deleted `[integration]` **P0**
- Docker networks deleted `[integration]` **P0**
- Libvirt networks deleted `[integration]` **P0**
- Host network interfaces (bridges, veths) deleted `[integration]` **P0**
- DB records cleaned up (lab, nodes, links, bridges) `[integration]` **P0**
- Lab directory deleted from filesystem `[integration]` **P0**
- Lab ownership validated before destruction `[integration]` **P0**
- Partial failures reported but destruction continues `[integration]` **P0**
- Streaming progress during destruction `[integration]` **P1**

---

## Node Shutdown (`down`)

**What to test:**
- VM nodes: graceful ACPI shutdown when guest agent available `[integration]` **P0**
- VM nodes: force power-off when no guest agent `[integration]` **P0**
- Container nodes: paused via Docker `[integration]` **P0**
- All nodes or specific node targeted `[integration]` **P0**
- Node state updated to Stopped in DB `[integration]` **P0**
- Node not found produces error `[integration]` **P0**

---

## Node Resume (`start_lab_nodes`)

**What to test:**
- VM in SHUTOFF state: cold boot via domain.create() `[integration]` **P0**
- VM in PAUSED state: resume via domain.resume() `[integration]` **P0**
- Container: unpaused via Docker `[integration]` **P0**
- All nodes or specific node targeted `[integration]` **P0**
- Node state updated to Running in DB `[integration]` **P0**

---

## Node Redeploy

**What to test:**
- DB operations use shared AppState connection, not ephemeral connections `[unit]` **P0**
- Node destroyed and recreated with fresh ZTP config `[integration]` **P0**
- Old node config directory cleaned up `[integration]` **P1**
- Networks recreated for VMs `[integration]` **P1**
- Container networks recreated `[integration]` **P1**
- Unikernel nodes explicitly unsupported (error) `[integration]` **P1**
- Streaming progress during redeploy `[integration]` **P1**

---

## Lab Inspection

**What to test:**
- Returns lab info (networks, gateways, subnets) `[integration]` **P0**
- Returns per-node state, VNC ports, disk paths `[integration]` **P0**
- Returns links and bridges `[integration]` **P0**
- Lab ownership validated `[integration]` **P0**
- Missing lab file on filesystem handled `[integration]` **P1**
- Concurrent DB queries (nodes, links, bridges) via try_join `[integration]` **P2**

---

## Image Management

**What to test:**
- Import VM/Unikernel: source file validated, copied to images dir, tracked in DB `[integration]` **P0**
- Import VM/Unikernel: first image for model marked as default `[integration]` **P1**
- Import container: tar archive loaded into Docker daemon via `load_image()` `[integration]` **P0**
- Import container: tar.gz archive loaded into Docker daemon `[integration]` **P1**
- Import container: image recorded in DB after successful load `[integration]` **P0**
- Import container: first image for model marked as default `[integration]` **P1**
- Import container: nonexistent source file rejected before Docker load `[integration]` **P0**
- Scan: walks filesystem and Docker for discoverable images `[integration]` **P0**
- Scan: bulk upsert to DB `[integration]` **P1**
- Download: streams from URL with progress, imports to DB `[integration]` **P1**
- Delete: removes DB record and disk files `[integration]` **P0**
- Delete: blocked if nodes reference the image `[integration]` **P0**
- List: filtered by model/kind `[integration]` **P0**
- Show: returns full NodeConfig for default version when no version specified `[integration]` **P0**
- Show: returns full NodeConfig for specific version when version specified `[integration]` **P1**
- Show: returns error when no image found for model `[integration]` **P0**
- Show: returns error when specified version not found `[integration]` **P1**
- Pull container: Docker pull with progress, DB upsert `[integration]` **P0**

### Web UI Upload (multipart)

- Upload via multipart: file written to temp path, import service invoked, image tracked in DB `[integration]` **P0**
- Upload with missing file field returns error `[unit]` **P0**
- Upload with missing model field returns error `[unit]` **P0**
- Upload with missing version field returns error `[unit]` **P0**
- Upload with empty version string returns error `[unit]` **P0**
- Upload with invalid model string returns error `[unit]` **P0**
- Upload default field absent defaults to false `[unit]` **P1**
- Upload default field "on" sets default to true `[unit]` **P1**
- Upload temp file cleaned up after successful import `[integration]` **P1**
- Upload temp file cleaned up after failed import `[integration]` **P1**
- Upload rejects non-admin users `[integration]` **P0**

---

## Admin Lab Cleanup (`clean`)

**What to test:**
- Same teardown as destroy but without ownership check `[integration]` **P0**
- Tolerates missing lab info file `[integration]` **P1**
- Tolerates missing DB records `[integration]` **P1**
- Logs warnings instead of errors for missing resources `[integration]` **P2**

---

## Lab Listing

**What to test:**
- Returns labs owned by user with node counts `[integration]` **P0**
- Empty result for user with no labs `[integration]` **P1**

---

## Node Operations (Core Helpers)

**What to test:**
- `generate_container_ztp()` produces correct env/volumes/capabilities per model:
  - Arista cEOS, Nokia SR Linux, FRR, GitLab, SurrealDB, Vault `[unit]` **P0**
- `generate_vm_ztp()` produces correct boot configs per model `[unit]` **P0**
- `build_domain_template()` produces valid libvirt XML structure `[unit]` **P0**
- `clone_node_disks()` clones all disks for a node `[integration]` **P0**
- `start_container_node()` creates container with correct networks/volumes `[integration]` **P0**
- `destroy_vm_node()` undefines domain and cleans interfaces `[integration]` **P0**
- `destroy_container_node()` kills and removes container `[integration]` **P0**

---

## Progress Tracking

**What to test:**
- `ProgressSender::send_phase()` serializes phase with count `[unit]` **P1**
- `ProgressSender::send_status()` serializes status with kind `[unit]` **P1**
- Messages delivered via WebSocket `[integration]` **P1**
