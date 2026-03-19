# IPv6 Dual-Stack Support

Sherpa now supports dual-stack IPv4/IPv6 on all lab management networks. Every lab
gets both an IPv4 `/24` and an IPv6 `/64` management subnet automatically. Every node
receives both addresses.

## ULA Prefix Allocation

| Purpose | Prefix | Per-lab subnet |
|---|---|---|
| Management | `fd00:b00b::/48` | `/64` |
| Loopback | `fd00:1001::/48` | `/64` |

These defaults can be overridden in `sherpa.toml`:

```toml
management_prefix_ipv6 = "fd00:b00b::/48"
```

## Manifest Usage

Nodes can optionally specify a static IPv6 address in `manifest.toml`:

```toml
[[nodes]]
name = "router01"
model = "cisco_cat8000v"
ipv6_address = "fd00:b00b:0:1::a"
```

When omitted, IPv6 addresses are auto-assigned from the management subnet
(offset `10 + node_index`, same scheme as IPv4).

## What's Implemented

### Core
- IPv6 data models, constants, and utility functions
- Database schema fields (all `Option` — backwards compatible)
- IPv6 subnet allocation per lab (management + loopback)
- Per-node IPv6 address assignment and DB persistence
- IPv6 validation (rejects loopback, multicast, unspecified)

### Networking
- Libvirt NAT networks include `<ip family='ipv6'>` element
- Docker bridge networks created with dual-stack IPAM
- Container endpoints receive IPv6 via `EndpointIpamConfig`
- dnsmasq configured for SLAAC (`ra-stateless`) + DHCPv6 DNS option
- AAAA host records generated per node

### VM/Container Provisioning
- Cloud-init (Linux/Alpine): dual-stack addresses, routes, DNS
- Cloudbase-init (Windows): IPv6 static subnet entry
- Ignition (Flatcar): IPv6 Address/Gateway/DNS in networkd unit
- Nokia SR Linux: static IPv6 on mgmt0 subinterface + IPv6 DNS

### Vendor ZTP Templates
All vendor templates conditionally render IPv6 management config when present:

| Vendor | IPv6 config |
|---|---|
| Cisco IOS-XE / IOSv / IOSvL2 | `ipv6 address`, `ipv6 unicast-routing`, `ipv6 route ::/0` |
| Cisco IOS-XR | `ipv6 address`, `address-family ipv6 unicast` static route |
| Cisco NX-OS | `ipv6 address`, `ipv6 route ::/0 vrf management` |
| Cisco ASA | `ipv6 address`, `ipv6 route management ::/0` |
| Cisco ISE | `ipv6_addr=`, `ipv6_default_gw=` |
| Cisco FTDv | Already had IPv6 fields (`IPv6Addr`, `IPv6Gw`, `IPv6Mask`) |
| Arista vEOS / cEOS | `ipv6 address`, `ipv6 route ::/0` |
| Juniper JunOS | `family inet6 { address; }`, `rib inet6.0` static route |
| Aruba AOS-CX | `ipv6 static`, `ipv6 route ::/0` |
| Cumulus Linux | `nv set interface eth0 ip address/gateway` (IPv6) |
| Nokia SR Linux | Static IPv6 on mgmt0, IPv6 DNS servers |
| MikroTik RouterOS | `/ipv6 address add`, `/ipv6 route add` |
| Palo Alto PAN-OS (init-cfg) | `ipv6-address=`, `ipv6-default-gateway=` |
| FRR | `ipv6 address`, `ipv6 route ::/0` |

### Display and Client
- Devices table includes "Mgmt IPv6" column
- Lab info table shows IPv6 network/gateway/router when present
- Inspect API returns `mgmt_ipv6` per device
- Client init prompts for optional server IPv6 address
- Server init reads `SHERPA_SERVER_IPV6` from env
- Server binds IPv6 listener alongside IPv4 when `server_ipv6` is configured
  (separate sockets for WS/WSS and HTTP cert endpoint)
- `SHERPA_SERVER_IPV6` env var overrides config at runtime
- IPv6 addresses auto-added to TLS certificate SANs

## Not Yet Implemented

The following items are out of scope for this release and tracked for future work:

### Palo Alto PAN-OS Bootstrap XML
The `paloalto_panos_bootstrap.jinja` template generates a full XML bootstrap config
(327 lines). IPv6 management config requires adding `<ipv6-address>`, `<ipv6-default-gateway>`,
and related elements in multiple locations within the XML structure. This needs careful
testing against PAN-OS bootstrap validation.

### SONiC Linux ZTP
The `sonic/ztp_user.jinja` template only handles user creation (useradd, SSH keys, sudo).
Management IP configuration for SONiC is handled externally via ZTP JSON, not this template.
IPv6 support for SONiC requires extending the `SonicLinuxZtp` struct and generating the
appropriate ZTP JSON with IPv6 management config.

### SSH Config IPv6 HostName
SSH config entries use IPv4 for `HostName`. An option to prefer IPv6 for SSH connectivity
could be added as a user preference.
