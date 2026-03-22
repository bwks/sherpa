# Sherpa

Sherpa is a lab management platform for building and managing virtual network topologies. It provides a unified control plane for virtual machines, containers, and unikernels backed by libvirt/KVM/QEMU and Docker.

Interact with Sherpa via the `sherpa` CLI or the built-in web UI.

For installation, usage, and configuration see the [documentation](https://docs.sherpalabs.net/about/).

## Development

### Prerequisites

- Rust (stable)
- Docker (for containers and the SurrealDB dev database)
- libvirt/KVM/QEMU (for VM and unikernel support)

### Project Structure

The project is organized as a Cargo workspace with crates under `crates/`:

| Crate        | Purpose                                        |
|--------------|------------------------------------------------|
| `client`     | CLI tool (`sherpa`) for interacting with the server |
| `server`     | Server daemon (`sherpad`) — API, web UI, and WebSocket RPC |
| `db`         | SurrealDB schema, migrations, and CRUD operations |
| `libvirt`    | VM and unikernel management via libvirt        |
| `container`  | Container management via Docker/Bollard        |
| `network`    | Host network operations                        |
| `shared`     | Shared data models and utilities               |
| `template`   | Configuration template generation              |
| `topology`   | Lab topology data models and transformers      |
| `validate`   | Input validation                               |

### Building

```bash
# debug build
cargo build

 # release build
cargo build --release

  # release build for all workspace crates
cargo build --workspace --all-targets --release
```

### Code Quality

```bash
cargo fmt              # format code
cargo clippy           # lint
cargo test             # run tests
```

### Dev Scripts

Helper scripts are in the `dev/` directory:

```bash
dev/serve              # run the server in development mode
dev/testdb start       # spin up an in-memory SurrealDB test database
dev/testdb stop        # stop the test database
dev/testdb restart     # fresh database
dev/dbreset            # reset the database
dev/rebuild            # Rebuild the project and copy binaries to path
```

## Logging

### Log Levels

Sherpa uses the `RUST_LOG` environment variable to control log verbosity. If not set, it defaults to `info`.

| Level   | Description                                      |
|---------|--------------------------------------------------|
| `error` | Errors only                                      |
| `warn`  | Errors and warnings                              |
| `info`  | General operational messages (default)            |
| `debug` | Detailed diagnostics including timing information |
| `trace` | Very verbose, includes library-level output       |

### Viewing Logs

When running via **systemd** (foreground mode), logs go to journald:

```bash
# Follow logs in real time
journalctl -u sherpad -f

# View last 200 lines
journalctl -u sherpad --no-pager -n 200

# View logs since last boot
journalctl -u sherpad -b
```

When running in **background mode** (without `--foreground`), logs are written to:

```
/opt/sherpa/logs/sherpad.log
```

### Changing the Log Level

Set `RUST_LOG` in the environment file used by the systemd unit:

```bash
# /opt/sherpa/env/sherpa.env
RUST_LOG=debug
```

Then restart the service:

```bash
sudo systemctl daemon-reload && sudo systemctl restart sherpad
```

You can also scope the level to specific crates:

```bash
# Debug logs for sherpad only, info for everything else
RUST_LOG=info,sherpad=debug
```