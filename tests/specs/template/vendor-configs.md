# Vendor Configuration Templates — Test Specifications

> **Crate:** `crates/template/`
> **External Dependencies:** None (Askama template rendering)
> **Existing Tests:** None

---

## General Template Validation

**What to test for every vendor template:**
- Template renders without error when all required fields provided `[unit]` **P0**
- Rendered output contains correct hostname `[unit]` **P0**
- Rendered output contains management IP configuration `[unit]` **P0**
- Rendered output contains user credentials (username, password, SSH key) `[unit]` **P0**
- Rendered output contains DNS configuration `[unit]` **P0**
- IPv6 management address included when provided `[unit]` **P1**
- IPv6 omitted when not provided `[unit]` **P1**
- Output is syntactically valid for the target platform `[unit]` **P1**

---

## Cisco Templates

**What to test:**
- **IOS/IOSv/IOSvL2** — management interface name configurable, correct IOS syntax `[unit]` **P0**
- **IOS-XE** — optional license_boot_command included when set `[unit]` **P1**
- **IOS-XR** — correct XR CLI syntax `[unit]` **P0**
- **NX-OS** — correct NXOS syntax `[unit]` **P0**
- **ASA** — correct ASA configuration syntax `[unit]` **P0**
- **FTDv** — JSON output with EULA, FirewallMode, IPv4/IPv6 mode fields, "Yes"/"No" serialization for ManageLocally `[unit]` **P0**
- **ISE** — management IPv4 required (not optional), correct ISE syntax `[unit]` **P0**

---

## Arista Templates

**What to test:**
- **vEOS** — correct EOS ZTP configuration syntax `[unit]` **P0**
- **cEOS** — container-specific EOS configuration `[unit]` **P0**
- vEOS and cEOS produce different output for same inputs `[unit]` **P1**

---

## Juniper Templates

**What to test:**
- **vJunos** — correct Junos set-style or XML syntax `[unit]` **P0**
- Management interface name configurable `[unit]` **P1**

---

## Nokia SR Linux

**What to test:**
- `build_srlinux_config()` produces valid JSON `[unit]` **P0**
- YANG config includes system, interfaces, network-instance sections `[unit]` **P0**
- Static IP or DHCP mode based on input `[unit]` **P1**
- Dual-stack IPv4/IPv6 handling `[unit]` **P1**
- ACL factory config included from JSON file `[unit]` **P1**
- SSH key and admin user configured `[unit]` **P0**

---

## Other Vendors

**What to test:**
- **Palo Alto PAN-OS** — init template and bootstrap template produce different outputs `[unit]` **P0**
- **Aruba AOS-CX** — correct AOS-CX syntax `[unit]` **P0**
- **Cumulus Linux** — correct Cumulus ZTP script syntax `[unit]` **P0**
- **Mikrotik RouterOS** — correct RouterOS syntax, management interface configurable `[unit]` **P0**
- **SONiC** — ZTP file map JSON structure, config_db.json with hostname and AAA `[unit]` **P0**
- **SONiC** — user setup script template rendering `[unit]` **P1**
- **FRR** — config, daemons, and startup templates all render `[unit]` **P0**
- **Vault** — config template renders with node_name `[unit]` **P1**

---

## Cloudbase-Init (Windows)

**What to test:**
- YAML #cloud-config output for Windows `[unit]` **P0**
- Network config with static/DHCPv4 and optional DHCPv6 `[unit]` **P0**
- Sherpa user created with SSH keys and groups `[unit]` **P0**
- Write-files section rendered correctly `[unit]` **P1**
