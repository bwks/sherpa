# Tailnet

Connecting a lab to its owner's Tailscale tailnet: the gateway container a lab with `[tailscale]` enabled gets on its management network, the auth key that enrolls it, from the variable on the user's machine to the gateway, the routes the gateway advertises and what it forwards, the gateway status Sherpa reports, what a whole-lab down, a resume, a single-node operation, a destroy, a clean and a failed up do to the gateway, and the server's `[tailscale]` settings. The gateway is made, enrolled, stopped and removed by the server through Docker; the client's part is reading the key and printing the status. Parsing `[tailscale]` in `manifest.toml`, checking the name in `auth_key_env` and refusing a container node with the gateway's name (`node_naming_005`) are in the Manifests spec; the gateway's address at offset 3 is `lab_addressing_007` in [lab lifecycle](lab-lifecycle.md), and the lab operations themselves are there too; the lab keeping its record when anything cannot be removed is `lab_destroy`; the rest of what `sherpa inspect` shows is in the Inspect and web UI spec; a client with no route to a node being told to join the tailnet is `ssh_access_006` in [client lab files](client-lab-files.md).

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `tailnet_key_002_an_unset_variable_is_refused_carrying_it`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above. Unit cases marked "mock Docker" run the gateway code against a Docker API that answers from a script. Cases marked "real tailnet" join a tailnet at Tailscale; what they need is at the end.

## `tailnet_join`: up joins a lab to its owner's tailnet through a gateway

**Statement:** `sherpa up` of a lab whose manifest enables `[tailscale]` creates one gateway container for the lab, on the lab's management network only, and enrolls it in the tailnet the key belongs to; an up that cannot enroll fails and leaves nothing behind, and a lab without `[tailscale]` enabled gets no gateway.
**Relied on by:** the user, to reach nodes from machines off the server; `sherpa ssh` and provisioners without `--ssh-jump`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A lab with `[tailscale]` enabled comes up with its gateway connected: up prints the Tailscale table with a tailnet address, and the gateway holds the third address of the management subnet (real tailnet) | e2e |
| 002 | From another device on the tailnet that accepts routes, once the routes are approved, a node's management address answers SSH with no `--ssh-jump` (real tailnet) | e2e |
| 003 | A gateway created on a Docker network and enrolled with a valid key reports connected (real tailnet) | integration |
| 004 | An up whose key the tailnet refuses fails, and the lab is rolled back: no gateway, no identity volume, no lab record (needs server TLS and a route to Tailscale, no tailnet) | e2e |
| 005 | A lab with no `[tailscale]` table, or with `enabled = false`, comes up with no gateway container and no Tailscale table (no test yet) | integration |
| 006 | The gateway container is not privileged and not on the host's network: it has `NET_ADMIN` and `NET_RAW`, the one TUN device, one network, the lab's management network, and its lab's labels | unit |
| 007 | The gateway's container stays recorded on the lab through the lab's phase changes, so down, resume and inspect find it | integration |
| 008 | An up of a lab ID that already has a gateway identity volume is refused, and the volume is left as it is (no test yet) | integration |
| 009 | In a lab of a virtual machine and a container node, from another device on the tailnet that accepts routes, once the routes are approved, both nodes' management addresses answer: the virtual machine on SSH, the container, which runs no SSH server, to ping (real tailnet) | e2e |
| 010 | In a lab with an IPv6 management network, from another device on the tailnet that accepts routes, once the routes are approved, a node answers SSH on its IPv4 and its IPv6 management address (real tailnet) | e2e |

## `tailnet_key`: the auth key goes from the user's shell to the gateway and nowhere else

**Statement:** the auth key is read on the user's machine from the environment variable the manifest names, sent to the server only over TLS with the certificate checked, handed to the gateway on a command's standard input, and is never in the manifest, the database, a log, a container's configuration or a command line.
**Relied on by:** the user and the tailnet's admin: the key enrolls devices in their tailnet. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The key is the value of the variable `auth_key_env` names, and the manifest that is sent does not hold it | unit |
| 002 | A variable that is not set is refused, and the error carries the variable | unit |
| 003 | A server address that is not `wss://`, or a connection set to `insecure`, is refused before the key is sent | unit |
| 004 | With `[tailscale]` absent or not enabled no variable is read and no key is sent, over any connection | unit |
| 005 | A key that is empty, only the `tskey-auth-` prefix, has whitespace, or is another kind of Tailscale credential is refused with an error that carries nothing of it | unit |
| 006 | The key's debug output is `[REDACTED]`, and the request still carries the key itself | unit |
| 007 | The server refuses an up with `[tailscale]` enabled that has no key, and one on a server with TLS off | unit |
| 008 | The server refuses a key sent for a lab whose manifest does not enable `[tailscale]` | unit |
| 009 | What is written to a gateway command's standard input reaches a file in the gateway's memory-only `/run`, and the file is gone when the command ends | integration |
| 010 | After an up, the key is not on the server: not in the database, the lab folder, sherpad's log, the gateway's `docker inspect`, its processes' command lines or its `/run` (real tailnet) | e2e |
| 011 | A lab with `[tailscale]` enabled created from the web UI is refused, saying to use the CLI: the web UI sends no key (no test yet; needs server TLS) | integration |

## `tailnet_routes`: the gateway advertises the lab's management subnets and forwards to nothing else

**Statement:** the gateway advertises the lab's own management subnets, its IPv4 /24 and its IPv6 /64 when it has one, and forwards what arrives from the tailnet only to them, from its own address.
**Relied on by:** the user; the other labs and networks of the server, which a lab's gateway must not open. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The routes the gateway advertises are the lab's management /24, and its /64 when the lab has one, and no other (no test yet) | unit |
| 002 | A started gateway's forwarding rules for traffic from the tailnet accept its routes and drop the rest, in IPv4 and IPv6 | integration |
| 003 | From another tailnet device, a node of a second lab and an address of the server outside the lab's subnets do not answer through the first lab's gateway (real tailnet) | e2e |
| 004 | A node sees a connection from the tailnet as coming from the gateway's address, and answers it with no route to the tailnet (real tailnet) | e2e |
| 005 | The gateway takes no routes and no DNS settings from the tailnet (real tailnet) | e2e |

## `tailnet_status`: Sherpa reports the gateway's connection and route approval

**Statement:** up, a whole-lab down, a whole-lab resume and `sherpa inspect` report the gateway's state, whether its routes are approved, the routes, its tailnet addresses and any warning, in one Tailscale table; routes that are not approved are a warning, not a failure.
**Relied on by:** the user, to know whether the lab is reachable and what to do when it is not. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A connected gateway whose every advertised route is allowed reads as approved, with no warning | unit |
| 002 | With one advertised route not allowed it reads as connected and pending, with a warning that names the routes and the route approval docs | unit |
| 003 | When the daemon reports no allowed addresses the approval is unknown, with a warning | unit |
| 004 | A running daemon whose key has expired reads as needing authentication, not connected, and never approved | unit |
| 005 | A logged-out daemon, which reports no addresses and no device of its own, reads as needing authentication, with a pointer to recovery | unit |
| 006 | The status is read on a tailnet with thousands of devices: only the gateway's own status is asked for (mock Docker) | unit |
| 007 | The Tailscale table shows the gateway's state, the approval, the routes, the addresses and the warnings | unit |
| 008 | A long warning wraps inside the table, which stays within 80 columns (no test yet) | unit |
| 009 | A gateway that is not running reads as stopped, with no warning (no test yet) | integration |
| 010 | When the gateway's status cannot be read, inspect still answers, with the gateway unavailable and a warning (no test yet) | integration |
| 011 | An up whose routes are not yet approved succeeds with the warning, and once they are approved `sherpa inspect` shows them approved with nothing restarted (no test yet; real tailnet, and approving by hand needs a Tailscale API credential the test setup does not have) | e2e |
| 012 | `sherpa inspect` of a lab with a gateway prints the Tailscale table (real tailnet) | e2e |
| 013 | `sherpa down` prints the Tailscale table the server sends, before the nodes' outcomes, and no table when the server sends none | unit |

## `tailnet_suspend`: down takes the gateway off the tailnet and resume enrolls it again

**Statement:** a whole-lab down logs the gateway out of the tailnet when its daemon holds an identity and stops it, and reports a logout that fails as a failure; a whole-lab resume of a lab whose gateway is not connected logs out any identity the gateway kept and enrolls it as a new device, with the key the client sends when the variable is set, before any node starts, and stops the gateway again when the enrollment fails; a whole-lab resume leaves a connected gateway as it is and needs no key; stopping, resuming or redeploying one node leaves the gateway as it is.
**Relied on by:** the user; the tailnet's admin, who expects no device for a lab that is down. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After `sherpa down` the gateway is stopped, its device is gone from the tailnet when the key was ephemeral, and the nodes no longer answer from the tailnet (real tailnet) | e2e |
| 002 | `sherpa resume` in the lab folder with the variable set prints the gateway connected again, and a node answers from the tailnet again (real tailnet) | e2e |
| 003 | A gateway that was stopped reports stopped, and enrolled again reports connected (real tailnet) | integration |
| 004 | Retired: a failed logout is now a failure of down; see 016 | — |
| 005 | A gateway that is already stopped is neither logged out nor stopped again (mock Docker) | unit |
| 006 | The server refuses a whole-lab resume of a lab whose gateway is not connected that has no key, carrying the variable `auth_key_env` names, a malformed key, or TLS off | unit |
| 007 | The server refuses a key sent for a lab with no gateway, and one sent with a single-node resume; without a key both go on | unit |
| 008 | A whole-lab `sherpa resume` in a folder whose manifest enables `[tailscale]` sends the key when the variable is set, checked as up checks it: a malformed key, a server that is not `wss://` and an `insecure` connection are refused; with the variable unset it sends no key and the server decides | unit |
| 009 | A single-node resume, and a resume in a folder with no manifest, send no key over any connection | unit |
| 010 | A gateway stopped and started again has its daemon up and its forwarding rules in place | integration |
| 011 | `sherpa down --node` and `sherpa resume --node` leave the gateway connected, and the resume needs no key (real tailnet) | e2e |
| 012 | A redeploy of a node leaves the gateway connected (real tailnet) | e2e |
| 013 | A whole-lab resume whose enrollment fails starts no node and leaves the gateway stopped, and the next resume with a valid key enrolls it (real tailnet) | e2e |
| 014 | A whole-lab resume from the web UI of a lab whose gateway is not connected is refused, saying to run `sherpa resume` in the lab folder; one whose gateway is connected resumes. The web UI itself greys start out for every lab with a gateway (026) on purpose, as it does not check each gateway's live state when it lists labs (real tailnet) | e2e |
| 015 | A whole-lab down lists the gateway beside the nodes as stopped, and a gateway that cannot be stopped as failed (no test yet) | integration |
| 016 | Down reports a gateway whose logout fails as a failed result, saying it was stopped but not logged out, and still stops it (mock Docker) | unit |
| 017 | A resume of a gateway still logged in from an unplanned stop, once its daemon has loaded its state, logs it out, then enrolls it with the key (mock Docker) | unit |
| 018 | A resume whose logout of the gateway's old identity fails is refused, and nothing is enrolled; that no node starts is 013 (mock Docker) | unit |
| 019 | After `systemctl restart docker` with the lab up, `sherpa resume` brings the gateway back as a new device, not its old identity (real tailnet; runs only with `--restart-docker`) | e2e |
| 020 | A resume of a gateway whose logout at down failed, which reports needing login but still holds its node key, logs it out, then enrolls it with the key (mock Docker) | unit |
| 021 | The server needs no key for a whole-lab resume of a lab whose gateway reads connected, and a key sent anyway, even malformed or with TLS off, is not refused and not used | unit |
| 022 | A resume whose enrollment fails logs the gateway out when its daemon holds an identity, stops it and fails; the next down leaves it as it is, and the next resume enrolls it (mock Docker) | unit |
| 023 | A resume whose enrollment fails and whose gateway then cannot be stopped fails with an error that carries both the enrollment error and the rollback error (mock Docker) | unit |
| 024 | Down of a running gateway whose daemon holds no identity stops it without a logout and reports it stopped (mock Docker) | unit |
| 025 | After `sherpa down --node` on every node, a whole-lab `sherpa resume` with the variable unset succeeds, and the gateway is the same tailnet device as before (real tailnet) | e2e |
| 026 | The web UI's labs list, the admin's list and the lab's page offer a lab with a gateway no start: a disabled start that says to resume the lab with `sherpa resume` in its lab folder, and stop as for any lab; a lab without a gateway offers start | unit |

## `tailnet_removal`: destroy takes the gateway and its identity away, and only its own

**Statement:** destroy, an admin's clean and the rollback of a failed up log a running gateway out, then remove the gateway container and its identity volume; they touch only a gateway and a volume labelled as the lab's, and when the logout fails or either cannot be removed the lab keeps its record so the removal can be run again.
**Relied on by:** the user and the tailnet's admin: no device and no identity outlive the lab; the other labs on the server. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | After `sherpa destroy` the lab's gateway container and identity volume are gone, and its device is gone from the tailnet when the key was ephemeral (real tailnet) | e2e |
| 002 | A running gateway is logged out before it is removed (mock Docker) | unit |
| 003 | Removing the gateway of a lab that has none succeeds (mock Docker) | unit |
| 004 | A container with the gateway's name but another lab's labels is refused, and nothing is removed (mock Docker) | unit |
| 005 | When the gateway container cannot be removed, its identity volume is left in place (mock Docker) | unit |
| 006 | Node containers whose names look like a gateway's, and carry no gateway labels, are removed as nodes (mock Docker) | unit |
| 007 | A gateway labelled for another lab whose name ends in this lab's ID is left alone (mock Docker) | unit |
| 008 | A gateway that cannot be removed is reported by destroy as a Tailscale failure, the lab's folder and record are kept, and a second destroy finishes (mock Docker) | unit |
| 009 | An identity volume that cannot be removed is reported and retried the same way (mock Docker) | unit |
| 010 | On real Docker, a gateway and its volume are both gone after removal, and a volume another container still holds fails the removal until it is let go (no test yet) | integration |
| 011 | `sherpa server clean <lab_id>` removes a lab's gateway and identity volume as destroy does (real tailnet) | e2e |
| 012 | A logout that fails is reported by destroy as a Tailscale failure saying the device may remain on the tailnet; the gateway and its volume are still removed, the lab's folder and record are kept, and a second destroy finishes (mock Docker) | unit |

A lab keeping its record after any resource fails to be removed is `lab_destroy_004` in [lab lifecycle](lab-lifecycle.md).

## `tailnet_settings`: the server's `[tailscale]` settings shape every gateway

**Statement:** the `[tailscale]` table of the server's `sherpa.toml` sets the gateway's image, the daemon's socket path and the timeouts of its commands, with a default for each one left out; sherpad does not start with a value it cannot use.
**Relied on by:** the admin: a registry of their own, a slow link to Tailscale. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | sherpad starts with a `sherpa.toml` that has no `[tailscale]` table, and a table with one setting changes only that one (no test yet) | integration |
| 002 | A `[tailscale]` table with a value of the wrong type, an unknown setting or a negative timeout is refused when the file is loaded | unit |
| 003 | An empty image or one with whitespace, a socket path that is relative, has no file name, has a NUL or is too long, a timeout of zero and a stop timeout Docker cannot take are each refused at start with that setting's error | unit |
| 004 | The image, the socket path and the enrollment timeout that are set are the ones the gateway and its commands get, and a socket path with shell characters is passed as an argument, never as script text | unit |
| 005 | A gateway command that outlasts the command timeout fails as timed out (mock Docker) | unit |
| 006 | The stop timeout that is set is the grace period Docker is given when the gateway is stopped (mock Docker) | unit |
| 007 | A gateway whose socket path is not the default starts, reports its status and stops (no test yet) | integration |
| 008 | A gateway image the server does not have is pulled when the first gateway is created (no test yet) | integration |
| 009 | A command timeout not longer than the enrollment timeout is refused at start, carrying both timeouts | unit |

## Cases that need a real tailnet

`tailnet_join_001` to `003`, `009` and `010`, `tailnet_key_010`, `tailnet_routes_003` to `005`, `tailnet_status_011` and `012`, `tailnet_suspend_001` to `003`, `011` to `014`, `019` and `025`, and `tailnet_removal_001` and `011` join a tailnet at Tailscale. `tailnet_join_004` needs a route to Tailscale but no tailnet, as the key it uses is refused. The integration cases are run as in [Live tailnet tests](../README.md#live-tailnet-tests); the e2e cases are the [tailnet lab test](../labs/tailnet/README.md), run on the Sherpa server. They need:

- a tailnet and an auth key for it that is reusable, ephemeral, pre-approved and tagged, given to the run in `SHERPA_TEST_TAILSCALE_AUTH_KEY`, as the VM code suite takes it;
- a tailnet policy, set up beforehand, whose `autoApprovers` approve the server's management ranges for the key's tag, and whose access rules let that tag reach those ranges: the routes of every gateway the run makes must be approved as they are advertised;
- a server with TLS on and a CLI that reaches it over `wss://` with the certificate checked, as nothing enrolls otherwise;
- a route from the test server to Tailscale's coordination and relay servers.

The second device on the tailnet is a probe the run makes: a `tailscale/tailscale` container of the gateway's image, in a Docker network of its own on the server that does not share a lab's management bridge, enrolled with the same key and accepting routes, and kept from every private range and the server's address except through the tailnet. Whether a node answers from the tailnet is a ping or the node's SSH banner read from inside the probe. Whether a device joined or left the tailnet is read from the probe's peer list, matched by the gateway's host name and node ID, with bounded polling for the tailnet to catch up; there is no Tailscale API credential. At the end the probe logs out and is removed, and a second throwaway probe checks that no device of the run is left in its peer list.

Without an API credential, nothing approves a route by hand, so `tailnet_status_011`, which needs a key whose tag has no automatic approval and a route approved after the up, has no test.
