# Private bridges (`[[bridges]]`)

> Status: fixed for #345. The lasting behaviour is in `docs/architecture/SERVER.md`
> (container data interfaces) and `docs/architecture/P2P-ARCHITECTURE.md`. The lab test
> `tests/labs/private-bridge/` covers findings 1 to 5.

Issue #345 reported, from reading the code, that container members of a `[[bridges]]` entry are
never attached to the bridge. A live lab confirmed it and turned up three more defects in the
same feature.

## Lab used

Server built from the #376 branch (bridge code unchanged from `main`) on the bare-metal dev host.

| Node | Model | Kind | Connections |
|---|---|---|---|
| dev01 | `alpine_linux` | VM | eth1 in bridge `private`; eth2 P2p link to fr03 |
| fr01 | `frr_linux` | container | eth1 in bridge `private`; eth2 link to fr02 eth2 |
| fr02 | `frr_linux` | container | eth1 in bridge `private`; eth2 link to fr01 eth2 |
| fr03 | `frr_linux` | container | eth1 in bridge `private`; eth2 P2p link to dev01 |

## Findings

### 1. Container bridge members are not attached (`up`)

`brs0-<lab-id>` had one member, the VM's tap. No container joined it, and no Docker network was
created for any container bridge member.

`up` builds `container_link_net_lookup` from `[[links]]` only. For a `NodeInterface::Bridge`
interface the lookup returns `None`, and `filter_map` drops the attachment. P2p containers
(fr03) build veths only for P2p links and disabled interfaces, so their bridge members are
dropped as well.

### 2. Dropping an attachment renames the container's other interfaces (`up`)

Linked attachments carry `linux_interface_name: None` for every model except SR Linux, so Docker
names them in attach order. With eth1 dropped, fr01's eth2 link came up **as `eth1`** inside the
container, and there was no `eth2`:

| In container | Docker network | Host parent |
|---|---|---|
| `eth1` | `fr01-etha0-<lab-id>` (the eth2 link) | `bra0-<lab-id>` |
| `eth3`..`eth8` | `fr01-iso3..8-<lab-id>` | isolated bridge |

Disabled attachments have explicit names, which is why eth3 onwards were right. P2p containers
name veths by index and were not shifted.

### 3. Redeploy handles bridge members differently, and breaks VMs

- **Containers:** redeploy has no bridge handling either, but treats the bridge member as
  disabled. fr01 came back with eth1 on its isolated network (admin down) and eth2 correct. The
  names are right, but the member is still not on the bridge.
- **VMs:** for any data interface without a link, redeploy takes the first bridge whose `nodes`
  list contains the node ("For now, treat as bridge interface"). The DB bridge record stores
  nodes, not interfaces. With `data_interface_count = 4` on dev01, eth3 and eth4 were also put on
  `brs0`. libvirt rejected the domain (`non unique alias detected:
  ua-net-dev01-<lab-id>-shared-bridge-brs0-<lab-id>`), and the VM was left **destroyed**.
  Redeploy has no rollback.

### 4. Destroy cannot clean the database for any lab with a bridge

A lab of two FRR containers in one bridge, with nothing else, fails on `destroy`:
`Database: Failed to clean`, then `Lab Directory: Failed to delete`, which is skipped by design
when the database step fails. `sherpa server clean` uses the same `cleanup_database`, so it fails
too.

`cleanup_database` deletes links, then nodes, then the lab, and never deletes bridges. The bridge
schema has `nodes TYPE array<record<node>> REFERENCE ON DELETE UNSET`. Deleting the lab's nodes in
one statement fails in SurrealDB 3.0:

```
Incorrect arguments for function array::union(). Argument 1 was the wrong type.
Expected `array` but found `NONE`
```

`db::delete_lab_bridges` exists but is not called. The log also keeps only the outer context
("Failed to delete lab nodes"); the SurrealDB error was found with the db test.

Labs `bridge-345-fe4edfdc` and `bridge-345b-45bced9f` were left in the dev server's database and
lab directory by this investigation. Their host resources (domains, containers, Docker networks,
interfaces) are gone.

## Ruled out

- **Docker refusing several macvlans on one parent:** the per-node isolated networks already put
  several `bridge`-mode macvlans on one Linux bridge (fr01 had six on `bri2-<lab-id>`).
- **Destroy leaving Docker networks:** destroy removes every Docker network whose name contains
  the lab ID, so new per-member networks need no destroy changes.

### 5. P2p redirects attached through TCX (found while testing the fix)

The lab test's fr03 redeploy broke the fr03–dev01 P2p link in both directions. aya 0.13
attaches with TCX on kernels 6.6 and later (the dev host runs 6.8), but `attach_p2p_redirect`
removed the old program with `qdisc_detach_program`, which only sees legacy TC filters. The
surviving end, dev01's tap, kept its old program in front of the new one, redirecting to the
deleted veth.

A TCX link also lives only as long as its file descriptor in `sherpad`. On a lab with a working
P2p link, `systemctl restart sherpad` took pings from 3/3 to 0/3. With the program attached as
a legacy TC filter on `clsact`, the same restart left 3/3.

Labs created before the fix keep TCX attachments, so their P2p links break on the next
`sherpad` restart, or already have, until they are recreated.

## Not a defect

After a redeploy the node has new MAC addresses, and its peers use their stale neighbour
entries until they expire (about 8 s on Alpine: DELAY, PROBE, then FAILED and a new ARP). The
lab test flushes neighbour caches before each ping round.
