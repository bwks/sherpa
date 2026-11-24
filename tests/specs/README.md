# Sherpa test specs

A spec says what Sherpa must do, as behaviours a user, a guest or another system relies on. Every test proves one case of one behaviour written down here, and its name says which. A test that proves no case in a spec is deleted, or a case is added for it first.

How to run each kind of test is in [running tests](../README.md). The disposable VMs that installer and integration tests run in are defined in [`tests/environments/`](../environments/README.md).

## Specs

One file per area a user sees, flat in this folder.

| Area | Spec | Covers |
|---|---|---|
| Lab lifecycle | [lab-lifecycle.md](lab-lifecycle.md) | up, down, resume, destroy, redeploy, clean, operations on labs at the same time, lab addressing, lab status, which lab a folder holds, lab phases and what the server does at startup with a lab left mid-operation, stopping a running operation |
| Nodes | [nodes.md](nodes.md) | VM, container and unikernel nodes: boot, ready checks, state, stop and start, failures |
| Links and bridges | [links-and-bridges.md](links-and-bridges.md) | P2p links, re-attach after a node restarts, link state and carrier, impairment, private bridges, node interfaces, reaching nodes on the management network, what destroy leaves on the host |
| Images | [images.md](images.md) | import, upload, download, container pulls, scan, default versions, list, show, edit, delete, which image a node gets, who may change images |
| Accounts and auth | [accounts-and-auth.md](accounts-and-auth.md) | user accounts and the built-in admin, login and logout, sessions and tokens, login passwords, SSH keys on an account, device passwords, owner logins on nodes, access control, what only an admin may do |
| Consoles | [consoles.md](consoles.md) | the serial console and the VNC console through sherpad, who may open one, one session per console and taking it over, refusals, how an open console ends, the console sockets on the server, nodes defined with consoles on TCP |
| ZTP | [ztp.md](ztp.md) | the ZTP setting of an image and of a node, ZTP off, how a config reaches a node by each method, the config ISO, a node's own `ztp.config`, the config generated for each model, its management addresses and its accounts, the lab's boot server (DHCP, DNS, TFTP, HTTP), who can read a node's config on the server |
| Provisioning | [provisioning.md](provisioning.md) | what the CLI reads on the user's machine for a node, the text files and systemd units of a first boot, startup scripts, the environment variables of the owner's login, the `ssh` provisioner's files, env files and scripts, when provisioners run, how they wait and what they report, container secret files, what of a provisioner the server never keeps |
| Client lab files and secrets | [client-lab-files.md](client-lab-files.md) | the lab folder's `.sherpa`: lab info, SSH config, known hosts, lab CA certificate, kubeconfig, talosconfig and the application nodes' credential files, when they are saved, who can read them, what destroy removes, the lab's line in the user's `~/.ssh` and `sherpa ssh-config`, node host keys, the administrators Sherpa generates for application nodes, how `sherpa ssh` finds a node, how the CLI reaches nodes |
| Tailnet | [tailnet.md](tailnet.md) | the Tailscale gateway of a lab with `[tailscale]` enabled, the auth key from the user's shell to the gateway and where it never is, the routes the gateway advertises and what it forwards, the gateway status up, resume and inspect report, route approval, what down, resume, single-node operations, destroy, clean and a failed up do to the gateway, the server's `[tailscale]` settings, the cases that need a real tailnet |
| Talos | [talos.md](talos.md) | Kubernetes clusters built from `talos_linux` nodes: the addresses a cluster gets, its secrets, the machine config of each member, the bootstrap and when a cluster is ready, what the kubeconfig and talosconfig hold, Cilium with its Gateway and Hubble UI, the `sherpa` realm of a Keycloak node and the cluster login through it, initialising and unsealing a Vault node and the secrets a cluster reads from it, what down, resume and redeploy do to a cluster, the cases that need a real cluster |
| Install and update | [install-and-update.md](install-and-update.md) | the install, update and uninstall scripts and what they refuse, `sherpad init`, the modes of the server's files, the blank disk templates, the database key, the server's key and its certificate renewed at every start, database migrations and what stored records become, the database backup, the `sherpad` commands that run the server, `sherpa.toml` from an older release, labs and lab folders an older release left, the Windows client install, the client's config, trusting the server's certificate, and the client and server version check |
| Manifests | [manifests.md](manifests.md) | how `manifest.toml` is found and parsed, unknown keys and wrong types, `sherpa validate` and `sherpa new`, where the CLI and the server check differently, node names, which kinds a model takes, which settings a kind and a model take, VM tuning rules and the domain they give, the ends of links and bridges, Talos cluster definitions, provisioner rules, text file modes and environment variable names, the lab's name, description, `[tailscale]`, `provision_concurrency` and `ready_timeout`, node settings that cannot be 0 |
| Inspect and web UI | [inspect-and-web-ui.md](inspect-and-web-ui.md) | what `sherpa inspect` prints and how it exits, the CLI's tables, what the server reports about a lab, the lists of labs, the web UI's lab and node pages with the actions they offer and how they refresh, the job page of a lab made or destroyed in the web UI, the create-lab form, the sidebar, the dashboard and the theme, what keeps a page from running what a user typed, the API documents the server publishes, `sherpa server status` and `/health`, what a refused or failed request is answered with, where the CLI and the web UI show the same thing differently |

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
- JavaScript (`node:test`, in `tests/web/`): the test's name is the ID, `test("web_layout_008_a_new_visitor_gets_the_light_or_dark_theme_of_their_system", ...)`.
- `grep lab_up_` finds every test of the behaviour in any language; `cargo test lab_up_`, `pytest -k lab_up_` or `node --test --test-name-pattern lab_up_ tests/web/*.cjs` runs them. `lab_up_001_` finds the one case.

Case numbers are never reused. A case that is dropped is marked retired in the table, and its number stays taken.

One test proves one case. A test that checks two cases is split.

## Levels, highest first

Each case is tested at the highest level that can show it:

1. **e2e:** the `sherpa` CLI against an installed server: the lab tests (`tests/labs/`) and the VM suite.
   - **e2e (browser):** a browser driving the web UI's pages and their scripts against an installed server. Nothing runs one yet, so these cases have no test.
   - **e2e (fresh VM):** the install, update and uninstall scripts run as root on a bare guest that `tests/harness/test_install.sh` builds on the host and destroys. The guest helper reports each case a step shows by its ID.
2. **integration:** real libvirt, Docker, the database or a running sherpad: the server integration tests (`crates/server/tests/integration/`) and the `#[ignore]` crate tests.
3. **unit:** only for logic with too many branches to cover above: parsing, validation, allocation, template rendering. Still a case of a behaviour, with its ID. The web UI's scripts are unit tested in JavaScript (`tests/web/`), marked unit (JavaScript).
   - **unit (bats):** one function of a shell script, sourced with its commands stubbed and its paths in a temporary folder (`tests/bats/`). The test's name, `@test "<case ID>"`, is the ID.

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

## Tooling outside the specs

These test the test tooling or the build, not Sherpa. They prove no case, carry no case ID, and the name check skips them:

- `crates/agent-bus`, the message bus between the host agent and the agents in lab guests.
- `crates/db/build`: the build script's count of a query file's parameters (`placeholders.rs`).
- `crates/server/src/log_capture.rs`: the helper other tests read log lines with.
- The `--self-test` of the `dev/check-*.sh` scripts, which check the code rules.
- The self-tests of `tests/harness/` (`vm_release_test.py`, `vm_release_guest.py`, `vm_release_faults.py`, `verify_release_test.py` and the rest) other than the three named by a case ID, the VM run's `failure-checks` scenario, and its `baseline`, `install-baseline` and `seed-upgrade` steps, which prepare a run.

## Enforcement

A check script will fail CI on a test whose name is not a known behaviour ID and case number, and on a case number the spec does not list or lists twice. It also reports cases with no test, without failing. It skips the tooling above. It is added once the tree passes it.
