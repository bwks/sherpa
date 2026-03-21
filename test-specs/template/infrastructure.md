# Infrastructure Templates — Test Specifications

> **Crate:** `crates/template/`
> **External Dependencies:** None (template rendering)
> **Existing Tests:** None

---

## Libvirt Domain XML (`domain.rs`)

**What to test:**
- `DomainTemplate` renders valid libvirt domain XML `[unit]` **P0**
- CPU architecture and model set correctly `[unit]` **P0**
- Memory allocation in correct units `[unit]` **P0**
- CPU count and VMX nesting support `[unit]` **P1**
- Disk definitions included (virtio, IDE, SATA bus types) `[unit]` **P0**
- Network interfaces rendered in correct order (management first) `[unit]` **P0**
- Interface type (virtio, e1000, rtl8139) set per device model `[unit]` **P1**
- Reserved interfaces rendered separately from data interfaces `[unit]` **P1**
- BIOS type (UEFI, SeaBIOS) configured `[unit]` **P1**
- Machine type (pc, q35, virt) set correctly `[unit]` **P1**
- QEMU command-line arguments appended when present `[unit]` **P1**
- Windows-specific settings applied when is_windows=true `[unit]` **P1**
- Telnet serial console port configured `[unit]` **P1**
- Lab ID and network names embedded `[unit]` **P1**

---

## Dnsmasq Configuration (`dnsmasq.rs`)

**What to test:**
- Template renders valid dnsmasq configuration `[unit]` **P0**
- TFTP server IPv4 address configured `[unit]` **P0**
- DHCP range (start/end) configured `[unit]` **P0**
- Gateway IPv4 set `[unit]` **P0**
- IPv6 DHCP range included when provided `[unit]` **P1**
- ZTP records (hostname → IP/MAC mappings) rendered `[unit]` **P0**
- DNS entries generated from ZTP records `[unit]` **P1**

---

## SSH Client Config (`ssh.rs`)

**What to test:**
- Template renders valid SSH config file `[unit]` **P0**
- Host entries generated from ZTP records `[unit]` **P0**
- Proxy settings (user, server IP) configured `[unit]` **P0**
- Each node gets a Host block with correct hostname `[unit]` **P1**

---

## PyATS Inventory (`pyats.rs`)

**What to test:**
- `PyatsInventory::from_manifest()` builds inventory from topology `[unit]` **P0**
- `to_yaml()` produces valid YAML `[unit]` **P0**
- Each device has alias, connection details, credentials, OS, platform, type `[unit]` **P0**
- Management IP and SSH port configured per device `[unit]` **P0**
- Custom ZTP username/password applied when provided `[unit]` **P1**
- Devices without management IP excluded or handled `[unit]` **P1**

---

## Vault Configuration (`vault.rs`)

**What to test:**
- Template renders valid Vault config `[unit]` **P1**
- Node name set in configuration `[unit]` **P1**
