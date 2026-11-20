# Lab lifecycle

Bringing a lab up, taking it down and resuming it, destroying it, redeploying one of its nodes, an admin's clean of a lost lab, operations on labs at the same time, the address ranges a lab gets, and the status a lab reports. Nodes, links, images, accounts and ZTP have their own specs.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `lab_up_001_a_container_lab_comes_up_with_every_node_running`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `lab_up`: up builds a lab from a manifest

**Statement:** `sherpa up` with a valid manifest creates every node and link, starts the nodes, and reports the lab ready, or reports each node that failed and exits non-zero.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container lab comes up and every node is running | integration |
| 002 | A VM lab comes up and its node accepts an SSH login | integration |
| 003 | A mixed VM and container lab comes up with its links working (no test with its ID yet; `tests/labs/p2p-links` shows it) | e2e |
| 004 | A node that fails to start fails the node, not the lab, and up exits non-zero | integration |
| 005 | A container that exits shortly after starting fails with its exit code and log | integration |

## `lab_preflight`: up refuses a lab it can't build, before creating anything

**Statement:** up refuses a request it can't carry out with an error that names the problem, and leaves nothing behind: no lab record, no node, no network.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A manifest naming an image that isn't imported is refused, naming the image | integration |
| 002 | An invalid manifest is refused | integration |
| 003 | A lab ID already in use is refused | integration |
| 004 | An unsupported ZTP setting is refused | integration |
| 005 | CPUs pinned by another lab are refused, naming the lab holding them | integration |
| 006 | Not enough free memory is refused (no test yet) | integration |
| 007 | A refusal for lack of resources exits with code 75 (no test yet) | e2e |

## `lab_concurrency`: operations at once leave each lab consistent

**Statement:** two operations on the same lab, or ups of different labs at once, finish with each lab consistent.
**Relied on by:** the user, scripts. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A second up of a lab that is coming up is refused and leaves the first alone | integration |
| 002 | Two destroys of one lab both succeed | integration |
| 003 | Labs brought up at once get distinct management subnets | integration |

## `lab_addressing`: each lab gets address ranges of its own

**Statement:** each lab gets address ranges of its own, which no other lab and no other network on the server uses, and its nodes get addresses at fixed offsets in them.
**Relied on by:** the user, SSH and provisioners, ZTP, the Tailscale gateway. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A new lab takes the first free /24 and /64 of the server's ranges, after the first of each, which is never given out | unit |
| 002 | A range a lab holds, or one that overlaps a network on one of the server's interfaces, is passed over | unit |
| 003 | With no range free, up is refused | unit |
| 004 | A range held by one lab is reported as used and cannot be recorded for a second, in IPv4 and IPv6, and labs with no IPv6 range do not collide | integration |
| 005 | The ranges a lab holds are free again once it is destroyed (no test yet) | integration |
| 006 | Nodes take addresses in manifest order from offset 21, in IPv4 and IPv6, and an index past the node block is refused | unit |
| 007 | The gateway, the lab router and the other infrastructure addresses are apart from each other and from the node block | unit |
| 008 | A configured management range too small for one lab is refused, in IPv4 and IPv6 | unit |
| 009 | The management network a lab's nodes are configured with spans the lab's /24 and /64, from the gateway to the end of each, with the lab router as its boot server | unit |

Labs brought up at once getting distinct ranges is `lab_concurrency_003`. Whether nodes reach each other on the management network is `management_network` in [links and bridges](links-and-bridges.md).

## `lab_down_resume`: down stops a lab and resume brings it back as it was

**Statement:** down stops every node and keeps the lab; resume starts them again with their disks, addresses and links as before, and waits until they are ready, as up does.
**Relied on by:** the user. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container lab goes down and resumes with its nodes running | integration |
| 002 | Resume waits for a VM to be ready and streams its progress | e2e |
| 003 | A resumed VM keeps its guest's keys and the owner can still log in | e2e |
| 004 | A VM's disk changes survive down and resume (no test yet) | e2e |

## `lab_destroy`: destroy removes everything the lab created

**Statement:** destroy removes the lab's nodes, networks, disks, records, folder and CPU pins on the server, and exits non-zero if any part could not be removed.
**Relied on by:** the user, the host's capacity. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container lab is destroyed with nothing left in Docker, the database or the lab folder | integration |
| 002 | A VM lab is destroyed with nothing left in libvirt | integration |
| 003 | A destroyed lab's CPU pins can be pinned again | integration |
| 004 | A partial failure is reported and exits non-zero (no test yet) | integration |

## `lab_redeploy`: redeploy rebuilds one node in place

**Statement:** redeploy rebuilds a node from its stored definition; a refused redeploy leaves the node as it was, and a failure after teardown fails that node.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A redeployed VM comes back and the owner can log in | e2e |
| 002 | A refused redeploy leaves the running node untouched | e2e |
| 003 | A refused redeploy keeps the node's CPU pins | integration |
| 004 | A redeploy to new pins holds its old ones from a racing up until it ends | integration |
| 005 | A redeploy that fails after teardown fails its node | integration |

## `lab_clean`: admin clean removes what a lost lab left behind

**Statement:** an admin's clean removes a lab's resources even when its record or its folder is gone.
**Relied on by:** the admin. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | Clean removes a running lab | integration |
| 002 | Clean removes a lab folder with no record | integration |

## `lab_status`: a lab's status follows its nodes

**Statement:** a lab's status follows the states of its nodes.
**Relied on by:** the user, the web UI, scripts reading `sherpa inspect`. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A lab is `running` when all its nodes are, and `stopped` when all are | unit |
| 002 | A lab is `failed` when any node is `failed` or `missing` | unit |
| 003 | A lab is `starting` when any node is `building` or `starting`, also beside stopped nodes | unit |
| 004 | A lab with some nodes running and some stopped is `partial` | unit |
| 005 | A lab's status changes with the operation's write to its node, without waiting for a scan | integration |
| 006 | The labs list shows each lab's status from its nodes' states as they are now | unit |
| 007 | A lab status this version does not know reads as `unknown` | unit |
| 008 | A lab with no nodes is `empty` | unit |

The node states themselves are `node_state` in [nodes](nodes.md).
