# Sherpa test specs

A spec says what Sherpa must do, as behaviours a user, a guest or another system relies on. Every test proves one case of one behaviour written down here, and its name says which. A test that proves no case in a spec is deleted, or a case is added for it first.

How to run each kind of test is in [running tests](../README.md). The disposable VMs that installer and integration tests run in are defined in [`tests/environments/`](../environments/README.md).

## Specs

One file per area a user sees, flat in this folder.

| Area | Spec | Covers |
|---|---|---|
| Lab lifecycle | [lab-lifecycle.md](lab-lifecycle.md) | up, down, resume, destroy, redeploy, clean, operations on labs at the same time, lab addressing, lab status |
| Nodes | [nodes.md](nodes.md) | VM, container and unikernel nodes: boot, ready checks, state, stop and start, failures |
| Links and bridges | [links-and-bridges.md](links-and-bridges.md) | P2p links, re-attach after a node restarts, link state and carrier, impairment, private bridges, node interfaces, reaching nodes on the management network, what destroy leaves on the host |
| Images | [images.md](images.md) | import, upload, download, container pulls, scan, default versions, list, show, edit, delete, which image a node gets, who may change images |
| Accounts and auth | [accounts-and-auth.md](accounts-and-auth.md) | user accounts and the built-in admin, login and logout, sessions and tokens, login passwords, SSH keys on an account, device passwords, owner logins on nodes, access control, what only an admin may do |
| Consoles | [consoles.md](consoles.md) | the serial console and the VNC console through sherpad, who may open one, one session per console and taking it over, refusals, how an open console ends, the console sockets on the server, nodes defined with consoles on TCP |
| ZTP | [ztp.md](ztp.md) | the ZTP setting of an image and of a node, ZTP off, how a config reaches a node by each method, the config ISO, a node's own `ztp.config`, the config generated for each model, its management addresses and its accounts, the lab's boot server (DHCP, DNS, TFTP, HTTP), who can read a node's config on the server |
| Provisioning | [provisioning.md](provisioning.md) | what the CLI reads on the user's machine for a node, the text files, binary files and systemd units of a first boot, startup and user scripts, the environment variables of the owner's login, the `ssh` provisioner's files, env files and scripts, when provisioners run, how they wait and what they report, container secret files, what of a provisioner the server never keeps |
| Client lab files and secrets | [client-lab-files.md](client-lab-files.md) | the lab folder's `.sherpa`: lab info, SSH config, known hosts, lab CA certificate, kubeconfig, talosconfig and the application nodes' credential files, when they are saved, who can read them, what destroy removes, the lab's line in the user's `~/.ssh` and `sherpa ssh-config`, node host keys, the administrators Sherpa generates for application nodes, how `sherpa ssh` finds a node, how the CLI reaches nodes |
| Tailnet | [tailnet.md](tailnet.md) | the Tailscale gateway of a lab with `[tailscale]` enabled, the auth key from the user's shell to the gateway and where it never is, the routes the gateway advertises and what it forwards, the gateway status up, resume and inspect report, route approval, what down, resume, single-node operations, destroy, clean and a failed up do to the gateway, the server's `[tailscale]` settings, the cases that need a real tailnet |
| Talos | not yet written | Talos clusters, Cilium, OIDC, Vault |
| Install and update | not yet written | install, update and uninstall scripts, database migrations, image folders of renamed models after an update, lab folders and saved credentials an older release left |
| Manifests | not yet written | manifest parsing and validation, which kinds a model takes and which settings a kind takes |
| Inspect and web UI | not yet written | what Sherpa shows about labs and nodes |

The folders here named after crates (`client/`, `server/`, `integration/` and the rest) are the old specs, written by crate. Each is deleted once the area specs that replace it are written. Until then they describe what exists, not how to write a new test.

## Behaviours

Each spec lists behaviours. A behaviour has:

- **ID:** a short snake_case name, unique across all specs, such as `lab_up`. No behaviour ID starts with another behaviour ID followed by `_`: `lab_up` and `lab_up_refusals` can't both exist, so a search for one behaviour never finds another's tests. An ID is never reused; a dropped behaviour is marked retired, not deleted.
- **Statement:** one sentence of what must hold.
- **Relied on by:** the user, the admin, a guest, another system.
- **Priority:** P0 to P3, as the issue labels.
- **Cases:** a table with the case number, what it shows, and the level it is tested at.

A behaviour is written as a heading, `` ## `lab_up`: up builds a lab from a manifest ``, then its statement, who relies on it and its priority, then the cases. [lab-lifecycle.md](lab-lifecycle.md) is the example to follow.

## Cases and test IDs

A case is one test. The test's name is its ID: the behaviour ID, a three-digit case number, and a description.

- Behaviour `lab_up`, case 001: `lab_up_001_a_container_lab_comes_up_with_every_node_running`.
- Rust: `fn lab_up_001_a_container_lab_comes_up_with_every_node_running()`.
- Python (pytest): `def test_lab_up_001_a_container_lab_comes_up_with_every_node_running()`.
- `grep lab_up_` finds every test of the behaviour in any language; `cargo test lab_up_` or `pytest -k lab_up_` runs them. `lab_up_001_` finds the one case.

Case numbers are never reused. A case that is dropped is marked retired in the table, and its number stays taken.

One test proves one case. A test that checks two cases is split.

## Levels, highest first

Each case is tested at the highest level that can show it:

1. **e2e:** the `sherpa` CLI against an installed server: the lab tests (`tests/labs/`) and the VM suite.
2. **integration:** real libvirt, Docker, the database or a running sherpad: the server integration tests (`crates/server/tests/integration/`) and the `#[ignore]` crate tests.
3. **unit:** only for logic with too many branches to cover above: parsing, validation, allocation, template rendering. Still a case of a behaviour, with its ID.

### e2e lab tests

A lab test is a pytest module. A module-scoped fixture brings the lab up once and destroys it at the end, even when a case fails. Each case is a test function using it, defined in case order. Nothing rebuilds a lab per case; only tests whose subject is a fresh build, such as the installer, start from scratch.

- A case that changes the lab, such as down, leaves it as the next case needs it: it puts it back, or it comes last.
- A case that changes an account works on a throwaway account the fixture creates and deletes. A test never changes the account running it.
- One lab test may hold cases of several behaviours, so a lab is not booted twice to show two things.

### Integration tests

An integration test sets up and tears down what it uses; integration tests never share a node. A case that needs a running VM shared with other cases is an e2e case.

Use the cheapest node that shows the case: a `generic` container on `alpine`, or the `alpine_linux` VM, unless the case is about a particular model.

## What a test must not depend on

A test fails on a regression of its case, and only then. It must not depend on:

- **the past:** asserting removed code is gone. Delete the code and its tests instead.
- **nothing:** asserting what the compiler, the type system or a library already guarantees, or what a mock was told to return.
- **the implementation:** expected values computed the way the code computes them, private state, which helper was called, or a copy of a constant, table, match or default. If the test would only fail when someone deliberately edits the thing it mirrors, it proves nothing.

A bug fix starts with a test that fails on the bug, as a case of the behaviour the bug broke; add the case to the spec if it is missing.

## Enforcement

A check script will fail CI on a test whose name is not a known behaviour ID and case number, and on a case number the spec does not list or lists twice. It also reports cases with no test, without failing. It is added once the tree passes it.
