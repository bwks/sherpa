# Cloud-Init & Ignition Templates — Test Specifications

> **Crate:** `crates/template/`
> **External Dependencies:** None (template rendering)
> **Existing Tests:** None

---

## Cloud-Init

### User Data (`CloudInitConfig`)

**What to test:**
- Renders valid YAML #cloud-config document `[unit]` **P0**
- Hostname and FQDN set correctly `[unit]` **P0**
- `CloudInitUser::sherpa()` creates user with SSH key and sudo access `[unit]` **P0**
- SSH password auth setting applied `[unit]` **P1**
- Packages list included when provided `[unit]` **P1**
- Write-files section renders with correct encoding and permissions `[unit]` **P1**
- Runcmd section renders commands in order `[unit]` **P1**
- DNS resolv.conf config rendered when provided `[unit]` **P1**

### Network Config (`CloudInitNetwork`)

**What to test:**
- Renders valid network v2 config YAML `[unit]` **P0**
- `ztp_interface()` creates management interface config `[unit]` **P0**
- Static IPv4 address with gateway and DNS `[unit]` **P0**
- DHCP mode when no static IP provided `[unit]` **P1**
- IPv6 address included when provided `[unit]` **P0**
- IPv6 omitted when not provided `[unit]` **P1**
- MAC address match configured `[unit]` **P1**

### Meta Data (`MetaDataConfig`)

**What to test:**
- Renders YAML with instance_id, local_hostname `[unit]` **P0**
- Public keys included when provided `[unit]` **P1**

---

## Ignition (Flatcar/CoreOS)

### Config Generation (`IgnitionConfig`)

**What to test:**
- `IgnitionConfig::new()` produces valid JSON `[unit]` **P0**
- JSON output parses as valid Ignition spec `[unit]` **P0**
- `to_json_pretty()` produces formatted JSON `[unit]` **P1**

### Users (`IgnitionUser`)

**What to test:**
- User with name, password_hash, SSH keys, groups `[unit]` **P0**
- Multiple users in single config `[unit]` **P1**

### Files (`IgnitionFile`)

**What to test:**
- File with path, mode, contents, ownership `[unit]` **P0**
- `disable_resolved()` creates correct resolv.conf `[unit]` **P1**
- `ztp_interface()` creates network config for management interface `[unit]` **P0**
- `dnsmasq_config()` creates dnsmasq configuration file `[unit]` **P1**
- Static IPv4 and optional IPv6 in network config `[unit]` **P0**
- Docker compose helpers (raw, conf) render correctly `[unit]` **P1**

### Systemd Units (`IgnitionUnit`)

**What to test:**
- Unit with name, enabled, contents, dropins `[unit]` **P0**
- `systemd_resolved()` disables resolved `[unit]` **P1**
- `mount_container_disk()` creates mount unit `[unit]` **P1**
- `dnsmasq()` creates dnsmasq service unit `[unit]` **P1**
- `srlinux()` / `ceos()` create vendor-specific units `[unit]` **P1**
- Masked units handled correctly `[unit]` **P2**

### Links (`IgnitionLink`)

**What to test:**
- Symlink with path, target, hard/soft flag `[unit]` **P1**
- `docker_compose_raw()` creates correct symlink `[unit]` **P1**

### Filesystems (`IgnitionFileSystem`)

**What to test:**
- Filesystem with device, format, label, wipe flag `[unit]` **P1**
