# Lab tests

Each directory here is a lab under test: one or more Sherpa manifests plus a runner that drives
the `sherpa` CLI against an installed server, the way a user does. This is different from
`tests/environments/`, which holds the disposable servers that the VM suites install Sherpa on.

| Lab | Covers | Time |
|---|---|---|
| [private-bridge](private-bridge/README.md) | `[[bridges]]` with a VM and containers, through up, redeploy and destroy | about 5–10 min |
| [p2p-containers](p2p-containers/README.md) | P2p links on network OS, application and OS containers, some without `ip` or a shell, through up, redeploy and destroy | about 2–5 min |
| [talos-linux](talos-linux/README.md) | Sherpa-built Talos Kubernetes clusters | about 25–35 min |

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
- Files `sherpa up` and `sherpa download` write (SSH key and config, `lab-info.toml`, cluster
  credentials) are git-ignored by the root `.gitignore`, including when a lab is run by hand
  inside its folder. Lab manifests are the exception and are tracked.
- Each runner has a `--self-test` mode for its pure logic, which runs in seconds.

Lab tests are started by hand. How and when end-to-end tests run is being planned in
[issue #330](https://github.com/bwks/sherpa/issues/330).
