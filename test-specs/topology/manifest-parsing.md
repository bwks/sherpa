# Topology Manifest Parsing — Test Specifications

> **Crate:** `crates/topology/`
> **External Dependencies:** Filesystem (for load_file/write_file)
> **Existing Tests:** 4 inline tests (ready_timeout, skip_ready_check, ztp_config deserialization)

---

## Manifest Loading and Parsing

**What to test:**
- `Manifest::load_file()` parses valid TOML manifest `[unit]` **P0**
- Minimal manifest: name + one node (no links, no bridges) `[unit]` **P0**
- Full manifest with all optional fields populated `[unit]` **P0**
- Invalid TOML syntax produces parse error `[unit]` **P0**
- Missing required field (name) produces error `[unit]` **P0**
- Missing required field (nodes) produces error `[unit]` **P0**
- Unknown fields ignored (forward compatibility) `[unit]` **P1**

---

## Manifest Writing

**What to test:**
- `Manifest::write_file()` writes valid TOML `[unit]` **P0**
- Round-trip: load → write → load produces equivalent manifest `[unit]` **P0**
- TOML formatting: inline tables for nodes, trailing commas for arrays `[unit]` **P2**

---

## Example Generation

**What to test:**
- `Manifest::example()` produces valid manifest `[unit]` **P0**
- Example contains nodes (UbuntuLinux, FedoraLinux) and a link `[unit]` **P1**
- Generated lab name is unique (petname) `[unit]` **P1**
- Example manifest is parseable by load_file round-trip `[unit]` **P0**

---

## Node Parsing

**What to test:**
- Node with all optional fields parsed correctly `[unit]` **P0**
- Node with only required fields (name, model) uses defaults `[unit]` **P0**
- `ready_timeout` field parsed as u64 seconds `[unit]` **P0**
- `skip_ready_check` per-node flag parsed `[unit]` **P0**
- `ztp_config` field parsed (file path or base64) `[unit]` **P0**
- `volumes` parsed as Vec<VolumeMount> (src, dst) `[unit]` **P1**
- `startup_scripts` parsed as Vec<String> (file paths) `[unit]` **P1**
- `text_files`, `binary_files`, `systemd_units` parsed `[unit]` **P1**
- `environment_variables` parsed as Vec<String> `[unit]` **P1**
- `cpu_count`, `memory`, `boot_disk_size` optional overrides `[unit]` **P1**
- `data_interface_count` optional per-node data interface override parsed correctly `[unit]` **P0**
- `ipv4_address`, `ipv6_address` optional loopback IPs `[unit]` **P1**

---

## Link Parsing

**What to test:**
- `Link2` parsed with src/dst in "node::interface" format `[unit]` **P0**
- `Link2::expand()` splits node name and interface name correctly `[unit]` **P0**
- Optional `p2p` flag parsed `[unit]` **P1**
- Invalid format (missing ::) produces error `[unit]` **P0**
- Links list is optional (can be omitted from manifest) `[unit]` **P1**

---

## Bridge Parsing

**What to test:**
- Bridge with name and links in "node::interface" format `[unit]` **P0**
- `Bridge::parse_links()` produces BridgeExpanded with BridgeLink structs `[unit]` **P0**
- Invalid link format (missing ::) produces error `[unit]` **P0**
- Bridges list is optional (can be omitted from manifest) `[unit]` **P1**

---

## Progressive Expansion (Link2 → Expanded → Detailed)

**What to test:**
- LinkExpanded contains parsed node/interface names `[unit]` **P0**
- LinkDetailed contains resolved indices and NodeModel references `[unit]` **P0**
- BridgeExpanded → BridgeDetailed expansion with indices `[unit]` **P0**
- NodeExpanded contains original Node data plus index `[unit]` **P1**

**Existing coverage:** 4 tests cover ready_timeout, skip_ready_check, and ztp_config deserialization
