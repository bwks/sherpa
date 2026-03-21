# Shared Networking Utilities — Test Specifications

> **Crate:** `crates/shared/` (`util/`)
> **External Dependencies:** Some functions require network interfaces (feature-gated "netinfo")
> **Existing Tests:** Extensive inline tests in ip.rs, mac.rs, ssh.rs, port.rs, host.rs, interface.rs

---

## IPv4 Utilities (`util/ip.rs`)

**What to test:**
- `get_ipv4_addr()` returns nth address from network block `[unit]` **P0**
- `get_ipv4_network()` parses valid CIDR string `[unit]` **P0**
- `get_ipv4_network()` rejects invalid CIDR `[unit]` **P0**
- `allocate_management_subnet()` returns next free /24, skips x.x.0.0/24 `[unit]` **P0**
- `allocate_loopback_subnet()` returns next free /24, skips x.x.0.0/24 `[unit]` **P0**
- Allocation with all subnets used returns exhaustion error `[unit]` **P0**
- `get_ip()` computes address from loopback subnet + host offset `[unit]` **P0**

---

## IPv6 Utilities (`util/ip.rs`)

**What to test:**
- `get_ipv6_addr()` returns nth address from IPv6 network block `[unit]` **P0**
- `get_ipv6_network()` parses valid IPv6 CIDR string `[unit]` **P0**
- `allocate_ipv6_management_subnet()` returns next free /64, skips first /64 `[unit]` **P0**
- `allocate_ipv6_loopback_subnet()` returns next free /64, skips first /64 `[unit]` **P0**
- `get_ipv6_ip()` computes address from subnet + host offset `[unit]` **P0**

---

## MAC Utilities (`util/mac.rs`)

**What to test:**
- `random_mac()` generates colon-delimited MAC with correct vendor OUI `[unit]` **P0**
- Generated MAC has correct format (6 octets, colon-separated) `[unit]` **P0**
- `clean_mac()` removes delimiters, trims whitespace, lowercases `[unit]` **P0**
- Clean handles various input formats (colons, dashes, dots) `[unit]` **P1**

---

## Port Utilities (`util/port.rs`)

**What to test:**
- `id_to_port()` maps ID to high port number correctly `[unit]` **P0**
- Port values within valid range (1024-65535) `[unit]` **P1**

---

## DNS Utilities (`util/dns.rs`)

**What to test:**
- `default_dns()` creates DNS config with IPv4 nameserver from management network `[unit]` **P0**
- `default_dns_dual_stack()` creates config with both IPv4 and IPv6 nameservers `[unit]` **P0**
- Nameserver IP derived correctly from network (gateway address) `[unit]` **P1**

---

## DHCP Utilities (`util/dhcp.rs`)

**What to test:**
- `get_dhcp_leases()` fetches leases from HTTP endpoint `[integration]` **P1**
- Returns empty vec on failure (not error) `[unit]` **P0**
- Parses lease format correctly `[unit]` **P1**

---

## SSH Utilities (`util/ssh.rs`)

**What to test:**
- `get_ssh_public_key()` reads and parses SSH public key file `[unit]` **P0**
- Parses algorithm, key data, optional comment `[unit]` **P0**
- `pub_ssh_key_to_md5_hash()` converts key to MD5 hash for Cisco devices `[unit]` **P0**
- `pub_ssh_key_to_sha256_hash()` converts key to SHA-256 colon-separated hash `[unit]` **P0**
- `generate_ssh_keypair()` generates RSA, Ed25519, ECDSA keypairs `[unit]` **P0**
- Generated keys have restrictive permissions `[unit]` **P1**
- `find_user_ssh_keys()` discovers keys in ~/.ssh/ `[unit]` **P1**

---

## Interface Utilities (`util/interface.rs`)

**What to test:**
- `interface_to_idx()` converts interface name to index per device model `[unit]` **P0**
- `interface_from_idx()` converts index to interface name per device model `[unit]` **P0**
- Round-trip: name → index → name produces same result `[unit]` **P0**
- Supports all vendor naming: Arista, Cisco, Juniper, Cumulus, Nokia, Mikrotik, Paloalto, generic `[unit]` **P0**
- `srlinux_to_linux_interface()` converts SR Linux names (e.g., eth-1/3 → e1-3) `[unit]` **P1**
- `node_model_interfaces()` returns all interfaces for a model `[unit]` **P1**
- Invalid interface name returns error `[unit]` **P0**

---

## Host Utilities (`util/host.rs`)

**What to test:**
- `get_hostname()` returns short hostname `[unit]` **P0**
- `get_fqdn()` returns FQDN or None `[unit]` **P1**
- `get_non_loopback_ipv4_addresses()` excludes 127.x.x.x `[integration]` **P1**
- `get_non_loopback_ipv6_addresses()` excludes ::1 `[integration]` **P1**

---

## File System Utilities (`util/file_system.rs`)

**What to test:**
- `create_dir()` creates directories recursively `[unit]` **P0**
- `create_file()` writes content to file `[unit]` **P0**
- `load_file()` reads file content `[unit]` **P0**
- `expand_path()` expands ~ to home directory `[unit]` **P0**
- `file_exists()` / `dir_exists()` correct results `[unit]` **P0**
- `delete_file()` / `delete_dirs()` removes files and directories `[unit]` **P0**
- `copy_file()` copies with buffered I/O `[unit]` **P0**
- `create_symlink()` creates Unix symlink `[unit]` **P1**
- `fix_permissions_recursive()` sets dirs 0o775, files 0o660 `[unit]` **P1**
- `check_file_size()` returns correct range category (0.1-5GB) `[unit]` **P1**
- `create_ztp_iso()` calls genisoimage correctly `[integration]` **P1**
- `create_panos_bootstrap_iso()` creates PAN-OS ISO `[integration]` **P1**
- `copy_to_dos_image()` uses mcopy command `[integration]` **P2**
- `copy_to_ext4_image()` uses e2cp command `[integration]` **P2**

**Existing coverage:** Extensive inline tests for IP, MAC, SSH, interface, host, and port utilities
