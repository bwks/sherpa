# Lab tests

Each directory here is a lab under test: one or more Sherpa manifests plus a runner that drives
the `sherpa` CLI against an installed server, the way a user does. This is different from
`tests/environments/`, which holds the disposable servers that the VM suites install Sherpa on.

| Lab | Covers | Time |
|---|---|---|
| [private-bridge](private-bridge/README.md) | `[[bridges]]` with a VM and containers, through up, redeploy and destroy. A pytest module | about 5–10 min |
| [p2p-containers](p2p-containers/README.md) | P2p links on network OS, application and OS containers, some without `ip` or a shell, through up, redeploy and destroy. A pytest module | about 2–5 min |
| [container-ztp](container-ztp/README.md) | The `ztp` setting on cEOS, SR Linux and FRR containers: config mounts and the lab owner's login with ZTP on, neither with it off, after up and after a redeploy | about 5–15 min |
| [manifest-commands](manifest-commands/README.md) | What `sherpa validate`, `sherpa new` and a refused `sherpa up` print and exit with: no manifest, an unknown key, a failing check, no login or server, an existing manifest and `--force`. Brings no lab up. A pytest module | seconds |
| [account-lockout](account-lockout/README.md) | Failed logins locking a throwaway account and `admin`, and `sherpa server user unlock` and, with `--stop-server` on the server host, `sherpad user unlock` unlocking them. Brings no lab up. Needs a CLI logged in as an admin. A pytest module | seconds |
| [cisco-iol](cisco-iol/README.md) | `cisco_iol` routers built from your own CML image: generated config and SSH, P2p links, TCP into IOL, `write memory` across a cold boot. Needs the IOL image | about 3–5 min |
| [p2p-links](p2p-links/README.md) | P2p (eBPF) link requirements on VM ⟷ VM, VM ⟷ container and container ⟷ container: L2 protocols, LLDP and LACP bundles, capture, per-direction impairment, jumbo frames, lifecycle | about 15–20 min |
| [nixos-linux](nixos-linux/README.md) | `nixos_linux` VMs built from [bwks/nixos-image](https://github.com/bwks/nixos-image): cloud-init, the pinned host key, the lab owner's login, group and sudo through wheel, eth interface names, a text file, an SSH provisioner. A pytest module | about 5 min |
| [console](console/README.md) | `sherpa console` in a pseudo-terminal and `sherpa vnc` with a scripted viewer, through sherpad, with no system `telnet` or `ssh`: login prompt, `Ctrl+]` and `~.`, the RFB banner and Ctrl+C, a console in use naming its holder, `--force`, another user and an admin refused, a container refused. A pytest module | about 1–5 min |
| [ssh-access](ssh-access/README.md) | How `sherpa ssh` and SSH provisioners reach a node: directly, or through the Sherpa server with `--ssh-jump`, told apart by which process holds the connection the node sees. Needs a route to the lab's management network | about 2 min |
| [owner-login](owner-login/README.md) | A throwaway owner's login on a Linux VM and a network OS container, by key and by device password, with admin rights; no keys refused at up and redeploy; a rotated key not pushed on resume; down and resume, redeploy; what an admin who does not own the lab may and may not do. Needs a CLI logged in as an admin and a route to the lab's management network | about 5 min |
| [talos-linux](talos-linux/README.md) | Sherpa-built Talos Kubernetes clusters: a one-controller Flannel lab and a three-controller Cilium lab with Keycloak login and Vault secrets, through up, redeploy, down and resume, a stopped controller and destroy. Two pytest modules, run at once | about 30–40 min |

## Rules

- A lab test uses only the `sherpa` CLI and tools inside the lab. It never calls server
  internals or reads the server's database. The exception is a lab whose nodes cannot be
  reached that way (no shell, no SSH server): its runner runs on the Sherpa server and uses
  Docker to reach them, and its README says so.
- Every run works in its own directory under `.tmp/lab-tests/` and gives each lab a unique name,
  so the checked-in manifests are never modified.
- A run always destroys its labs, including on failure, Ctrl-C and SIGTERM, unless
  `--keep-failed` is given. After destroying a lab it checks nothing was left behind.
- Pin every version: node images, and tool downloads with SHA-256 checksums.
- Files `sherpa up` and `sherpa download` write, all in the lab folder's `.sherpa/` (SSH key and
  config, `lab-info.toml`, cluster credentials), are git-ignored by the root `.gitignore`,
  including when a lab is run by hand inside its folder. Lab manifests are the exception and are tracked.
- Every runner's preflight stops unless the CLI is logged in and the user has a device password
  (`sherpa server user info` shows `Device password: set`): `sherpa up` is refused without one.
  Set it once with `sherpa server user device-password`.
  The manifest-commands runner brings no lab up and checks the login only for its one case that
  sends a manifest to the server.
  The owner-login lab instead has the runner's admin login create a throwaway admin, which creates
  a throwaway owner with a device password; their passwords are generated for the run, typed at
  the CLI's prompts and never printed, and both accounts are deleted at the end.
  The console lab likewise has an admin create the other user it needs, and deletes it at the end.
  The account-lockout runner brings no lab up and checks no device password: its admin login
  creates a throwaway user for each case, which it locks, unlocks and deletes at the end. With
  `--stop-server` it also locks `admin`, and leaves it unlocked.
- Each runner has a `--self-test` mode for its pure logic, which runs in seconds.
- What the runners share lives in [`labkit.py`](labkit.py): the failure and outcome types,
  running and logging a command, masking the run's secrets, the manifest TOML, the device
  password preflight, generated passwords and a command in a pseudo-terminal whose prompts
  the runner answers. The console, owner-login, p2p-containers, nixos-linux, private-bridge, manifest-commands, account-lockout and talos-linux runners import it; its own
  unit tests run with `uv run tests/labs/labkit.py`.

Lab tests are started by hand. How and when end-to-end tests run is being planned in
[issue #330](https://github.com/bwks/sherpa/issues/330).
