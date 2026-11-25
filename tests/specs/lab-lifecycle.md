# Lab lifecycle

Bringing a lab up, taking it down and resuming it, destroying it, redeploying one of its nodes, an admin's clean of a lost lab, operations on labs at the same time, the address ranges a lab gets, the status a lab reports, which lab a folder holds, the phase a lab is in and what the server does at startup with a lab left mid-operation, and stopping a running operation. Nodes, links, images, accounts and ZTP have their own specs.

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
| 008 | A CPU pin written straight to the database, past the check that refuses it at up, is refused by the database when another node's pins forbid it, and allowed when it is a QEMU thread beside another node's thread | unit |
| 009 | A CPU pin row written to the database is never changed: an update of it is refused and the row is as it was | unit |

## `lab_concurrency`: operations at once leave each lab consistent

**Statement:** two operations on the same lab, or ups of different labs at once, finish with each lab consistent.
**Relied on by:** the user, scripts. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A second up of a lab that is coming up is refused and leaves the first alone | integration |
| 002 | Two destroys of one lab both succeed | integration |
| 003 | Labs brought up at once get distinct management subnets | integration |
| 004 | A second reservation of a lab ID that is being created is refused as a conflict that says the lab already exists and is creating, and the first's record is left as it was | integration |
| 005 | A destroy stops the operations running on its lab, each ending with an error that says the lab is being destroyed, and goes on only once they have ended | unit |
| 006 | A destroy of a lab with nothing running does not wait | unit |
| 007 | An operation started while its lab is being destroyed waits for the destroy, then runs as any other | unit |
| 008 | Stopping the operations of one lab leaves those of another running | unit |
| 009 | An operation queued behind a requested stop, with a destroy queued behind both, is stopped for the destroy, and operations run again once the destroy is done | unit |
| 010 | An operation stopped on request keeps that reason when a destroy follows before it has ended | unit |
| 011 | An operation waits while the scanner observes its lab | unit |
| 012 | With operations, stops and scans of one lab racing, a stop or a scan never holds the lab beside an operation or beside another stop or scan | unit |

Cases 005 to 012 are the lab's lease in the server: an operation (up, down, resume, redeploy) holds it shared, and a destroy, a clean, a requested stop and the scanner hold it alone. A stop the user asks for is `lab_cancel`. What keeps a second up from a lab that exists is its record, `lab_phase`.

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

The node states themselves are `node_state` in [nodes](nodes.md). A lab's status says how its nodes are; which operation owns the lab is its phase, `lab_phase`.

## `lab_identity`: a lab command acts on the lab of its folder

**Statement:** a lab command acts on the lab of the folder it runs in: the lab `.sherpa/lab-info.toml` records, with the ID the server gave it, or else the logged-in user's lab of the name in `manifest.toml`; a folder whose two files name different labs, whose `lab-info.toml` does not parse, or that has neither, is refused.
**Relied on by:** the user: every lab command, destroy included, goes to the lab found this way; folders made by `sherpa download`. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A folder with only `lab-info.toml`, as `sherpa download` makes, is the lab it records, by name and ID | unit |
| 002 | With a manifest of the same name beside it, the lab has the ID `lab-info.toml` records, for the commands that load the manifest and for those that do not | unit |
| 003 | A folder with only a manifest is the lab of the manifest's name, with the ID of the login's Sherpa username and that name, so another Sherpa user gets another ID | unit |
| 004 | A new lab's ID is eight hex digits from the owner's Sherpa username and the lab's name: the same for the same two every time, and another for another user or another name | unit |
| 005 | A `lab-info.toml` and a manifest that name different labs are refused, with an error that carries both labs and the file and says to destroy the recorded lab or remove the file | unit |
| 006 | A folder with neither file is refused, and the error carries the `lab-info.toml` it looked for | unit |
| 007 | `up`, `redeploy` and `provision` in a folder with no manifest are refused, saying a folder made by `sherpa download` has none and where to run the command | unit |
| 008 | A manifest that does not load beside a `lab-info.toml` does not stop a command that needs no manifest: the lab is the recorded one; with no `lab-info.toml` the command fails with the manifest's error | unit |
| 009 | A `lab-info.toml` that is there but does not parse stops the command, beside a manifest too, with an error that carries the file and says to restore it with `sherpa download` or remove it; only a folder with no `lab-info.toml` is the lab of its manifest | unit |
| 010 | A folder with only a manifest and no login is refused, since the lab's ID comes from the login (no test yet) | unit |
| 011 | After the lab's name is changed in the `manifest.toml` of a running lab's folder, `sherpa inspect` and `sherpa up` exit non-zero naming both labs, and the running lab is untouched (no test yet) | e2e |

Names are compared, never IDs, so a lab keeps the ID it was given. Refusing to connect to the nodes of a folder whose lab is another user's is `access_control_016` in [accounts and auth](accounts-and-auth.md). The files of `.sherpa`, `lab-info.toml` among them, are `saved_lab_files` in [client lab files](client-lab-files.md); `sherpa ssh` from a downloaded folder is `ssh_target_005`, and a console from one is `console_access_005`. An up whose lab ID is in use on the server is `lab_preflight_003`.

## `lab_phase`: a lab's phase says which operation owns it

**Statement:** a lab's record holds its phase: `creating` while up builds it, `ready` once built, `destroying` while a destroy, a clean or a failed up's rollback removes it, `destroy failed` when that left something behind, `interrupted` when the server stopped while it was created, and `unknown` for one this version does not know; only a `ready` lab takes down, resume and redeploy, a lab in any phase can be destroyed, and at startup the server marks each lab left `creating` or `destroying`, and removes nothing.
**Relied on by:** the user and the web UI, which are told what to do with a lab that is not ready; the admin, finding labs left behind; down, resume and redeploy, which never run on a half-built or half-removed lab. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The allowed moves: `creating` to `ready` when built, `creating` to removed when its up gives it back, any phase to `destroying`, `destroying` to removed or to `destroy failed`; any other move is refused, and the refusal carries the move and the phase | unit |
| 002 | Only `ready` takes operations on the lab's nodes; every other phase refuses them | unit |
| 003 | The refusal of each phase that is not `ready` carries the phase, which its message says what to do from | unit |
| 004 | A lab that lost its operation moves from `creating` to `interrupted` and from `destroying` to `destroy failed`, and both can then be destroyed | unit |
| 005 | A phase this version does not know reads as `unknown`, which is not ready and can be destroyed | unit |
| 006 | The record up reserves a lab with is `creating` | integration |
| 007 | Up moves the lab it reserved to `ready` | integration |
| 008 | A refused move leaves the lab's phase as it was: an up cannot give back a lab that is already `ready` | integration |
| 009 | A destroy that left something behind leaves the record `destroy failed`; a destroy of that lab that removes everything removes the record | integration |
| 010 | A phase write made from a phase the lab has since left changes nothing, and a lab that does not exist is not moved | integration |
| 011 | A record is removed only from the phase it was read in, and removing it again is no error | integration |
| 012 | Down, resume and redeploy of a lab that is not `ready` are refused with a conflict that says what to do, and its nodes are untouched | integration |
| 013 | An up of a lab that exists and is not `ready` is refused saying it exists and naming its phase | unit |
| 014 | At startup a lab left `creating` becomes `interrupted` and one left `destroying` becomes `destroy failed`, `ready` and `destroy failed` labs are left as they are, no record is removed, and a second run finds nothing to mark | integration |
| 015 | A lab that cannot be marked at startup is counted and logged, and the others are still marked | integration |
| 016 | After sherpad is stopped during an up and started again, `sherpa inspect` shows the lab `interrupted` with what to do, down is refused, and `sherpa destroy` removes it (no test yet) | e2e |

The nodes an up or a redeploy was building when the server stopped are `node_failure_010` in [nodes](nodes.md). How a lab that is not ready is shown is in [inspect and web UI](inspect-and-web-ui.md): `inspect_command_005`, `labs_list_005`, `007`, `009` and `010`, `lab_page_003` and `005`. A second up of a lab that is coming up is `lab_concurrency_001` and `004`; a lab ID another lab uses is `lab_preflight_003`. The Tailscale gateway kept on the record through phase changes is `tailnet_join_007`.

## `lab_cancel`: a running operation can be stopped on request

**Statement:** the lab's owner or an admin can stop the up, resume or redeploy running on a lab, by the `cancel` RPC or `POST /api/v1/labs/{id}/cancel`: the operation stops at its next step, the answer comes once it has ended and says `stopped: true` only when an up, resume or redeploy was running or waiting for the lab and was stopped, and a stopped up removes what it created; a down, a destroy, a clean and the scanner are never stopped by it, and when only they hold the lab it answers `stopped: false` at once; an operation is not stopped by its client going away, and Ctrl-C in the CLI asks before it stops anything.
**Relied on by:** the user; scripts and other clients of the API. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | Cancel of a lab with nothing running answers `stopped: false` with the lab's ID | integration |
| 002 | `POST /api/v1/labs/{id}/cancel` answers as the RPC does | integration |
| 003 | A requested stop ends a waiting operation with an error that says it was stopped on request, returns only once it has ended, saying an operation was running, and operations started after it run as any other | unit |
| 004 | A cancelled up of a container lab ends with nothing of the lab left in Docker or the database, and cancel answers `stopped: true` only once it is gone | integration |
| 005 | A cancelled up whose rollback leaves something behind leaves the lab `destroy failed` | integration |
| 006 | A cancelled resume stops waiting for its nodes: the nodes it started keep running and the lab stays `ready` | integration |
| 007 | A cancelled redeploy ends with an error that says it was stopped on request, its node is `failed`, and the lab stays `ready` | integration |
| 008 | An operation whose REST client disconnects runs on to its end | unit |
| 009 | Asked whether to stop, only `y` or `yes`, in any case, stops the operation; no answer or any other leaves it | unit |
| 010 | Ctrl-C during `sherpa up`, answered no: the CLI says the lab creation keeps running on the server and exits non-zero having saved no lab files, and the lab comes up (no test yet) | e2e |
| 011 | Ctrl-C during `sherpa up`, answered yes: the CLI waits, says the lab creation was stopped, and the lab is gone from the server (no test yet) | e2e |
| 012 | Cancel during a down answers `stopped: false` at once, and the down is not stopped | unit |
| 013 | Cancel during the scanner's observation of a lab with nothing running answers `stopped: false` at once, and cancels nothing | unit |
| 014 | Cancel of an up still waiting for the lab behind a scan stops it and answers `stopped: true` once it has ended; an up that is running is 003 and 004 | unit |

Who may cancel, the owner or an admin, and a lab that does not exist, are `access_control_013` in [accounts and auth](accounts-and-auth.md). A destroy stopping the operations of its lab is `lab_concurrency_005`. The node a failed redeploy leaves is `lab_redeploy_005`.
