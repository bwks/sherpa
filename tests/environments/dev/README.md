# Sherpa dev server

A long-lived Sherpa server for development, built as a lab by the host's Sherpa. Unlike the
other environments it is not disposable: it holds toolchains and checkouts, and Sherpa is
developed, installed and tested inside it with root, without `sudo` on the host. The design and
its decisions are in [the investigation note](../../../docs/investigations/sherpa-dev-server.md).

## Node

| Setting | Value |
|---|---|
| Image | `ubuntu_linux` `24.04-20260926` (see [ubuntu-2404](../ubuntu-2404/README.md)) |
| vCPUs | 14, as 1 socket × 7 cores × 2 threads, `host-passthrough` |
| Host CPUs | vCPUs on sibling pairs 9/25 to 15/31; QEMU and disk I/O threads on 8/24 |
| Memory | 48 GiB, strict on NUMA node 1 |
| Disk | 300 GiB full copy, virtio, `cache = "none"`, `io = "native"` |
| Network | 4 virtio-net queues |

The CPU numbers fit a host with two 8-core, 2-thread sockets where CPUs 8–15 and 24–31 are NUMA
node 1. On another host, change `performance_tuning` to match `lscpu -e`.

## Provisioning

`provision/setup.sh` runs through the SSH provisioner as the node's login user, after the node
is up. It is safe to run again with `sherpa provision`. It:

1. installs build packages: `libvirt-dev`, `clang`, `llvm`, `gh`, `shellcheck`, `bats` and others;
2. installs Rust with rustup: stable with `rustfmt`, `clippy` and `llvm-tools`, nightly with
   `rust-src`, and `bpf-linker` 0.10.2 for the eBPF program (0.12 needs a system LLVM
   that Ubuntu 24.04 does not ship);
3. sets up git to authenticate with `GH_TOKEN` through `gh`, committing as the token's GitHub
   account with its no-reply address;
4. clones `bwks/sherpa` and `bwks/sherpa-docs` into `~/code/rust/`. When they already exist it
   only fetches, so work in progress is never touched;
5. creates a virtualenv for the docs at `~/.venvs/sherpa-docs`;
6. installs a Sherpa server with the checkout's `scripts/sherpa_install.sh`, with a generated
   database password that only the guest keeps;
7. initializes the server unattended: it generates the `admin` password, a `dev` user (an
   administrator) with its own password, and an SSH key `~/.ssh/id_ed25519` for it, kept in
   `~/.env.d/sherpa-account.env` (mode `600`). It runs `sherpad init` with them and starts
   `sherpad`;
8. points the guest's `sherpa` CLI at its own server, trusting the server's certificate file,
   and logs in as `dev`.

Nothing is prompted for. Login shells load `~/.env.d/`, so `sherpa login` works there without
prompts too.

Node images are not copied; import them as needed with `sherpa server image import`.

## GitHub token

`provision/github.env` holds only `GH_TOKEN=$GH_TOKEN`. The CLI fills it in from your
environment when it provisions, so the token is not in the repository and is never sent to the
Sherpa server. It lands in `~/.env.d/github.env` in the guest, mode `600`.

Bring the lab up with the token in the environment, for example with a `mise` task that loads it:

```
cd tests/environments/dev
mise run sherpa up
```

## After provisioning

Development happens in the guest's checkouts: build with
`cargo build --release -p sherpad`, install over `/opt/sherpa/bin/sherpad`, restart `sherpad`, and
push from there.
