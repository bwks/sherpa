# Lab Lifecycle End-to-End — Test Specifications

> **Scope:** Cross-crate integration testing the full lab lifecycle
> **External Dependencies:** SurrealDB, Docker, libvirt, Linux networking, filesystem
> **Existing Tests:** None

---

## Happy Path: Full Lifecycle

**What to test:**
- Client sends manifest via WebSocket → server validates, provisions, returns results `[e2e]` **P0**
- Database records created: lab, nodes, links, bridges `[e2e]` **P0**
- Management networks provisioned (libvirt NAT + Docker bridge) `[e2e]` **P0**
- VM nodes: disks cloned, domains defined and started `[e2e]` **P0**
- Container nodes: Docker containers created and started `[e2e]` **P0**
- Point-to-point links: veth pairs and bridges created between nodes `[e2e]` **P0**
- Broadcast bridges: multi-node bridge connectivity established `[e2e]` **P0**
- Inspect returns accurate lab state (nodes, links, bridges, IPs) `[e2e]` **P0**
- Down gracefully stops all nodes, DB states updated to Stopped `[e2e]` **P0**
- Resume restarts all stopped nodes, DB states updated to Running `[e2e]` **P0**
- Destroy tears down all resources and cleans DB `[e2e]` **P0**

---

## Mixed Node Types

**What to test:**
- Lab with both VM and container nodes `[e2e]` **P0**
- Each node type provisioned with correct subsystem (libvirt vs Docker) `[e2e]` **P0**
- Links between VM and container nodes via veth bridges `[e2e]` **P1**

---

## ZTP Configuration

**What to test:**
- VM nodes get correct vendor-specific ZTP configs `[e2e]` **P0**
- Container nodes get correct env vars, volumes, capabilities per model `[e2e]` **P0**
- Custom ZTP config files from manifest applied `[e2e]` **P1**
- Startup scripts executed on container nodes `[e2e]` **P1**

---

## IP Network Allocation

**What to test:**
- Management subnets allocated without collision across labs `[e2e]` **P0**
- Loopback subnets allocated without collision `[e2e]` **P0**
- IPv6 management subnets allocated when configured `[e2e]` **P1**
- Node management IPs assigned within allocated subnet `[e2e]` **P0**

---

## Failure Scenarios

**What to test:**
- Invalid manifest rejected before any resources created `[e2e]` **P0**
- Partial failure during provisioning triggers cleanup `[e2e]` **P0**
- Destroy of partially-created lab cleans up whatever was created `[e2e]` **P0**
- Lab already exists produces error (no duplicate labs) `[e2e]` **P0**
- Missing image (not imported) produces error before provisioning `[e2e]` **P0**

---

## Progress Streaming

**What to test:**
- Client receives phase progress messages during `up` `[e2e]` **P1**
- Client receives status messages during `destroy` `[e2e]` **P1**
- Progress messages arrive in correct phase order `[e2e]` **P2**

---

## Node Redeploy

**What to test:**
- Single node destroyed and recreated within running lab `[e2e]` **P0**
- Other nodes in lab unaffected by redeploy `[e2e]` **P0**
- Fresh ZTP config applied to redeployed node `[e2e]` **P1**

---

## Concurrent Labs

**What to test:**
- Multiple labs for same user do not conflict `[e2e]` **P1**
- Network/IP allocations isolated between labs `[e2e]` **P1**
