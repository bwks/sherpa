# How to Run Integration Tests

Integration tests require external services (SurrealDB, Docker, libvirt) and some crates
need elevated privileges. Tests run inside a `devbox_linux` VM managed by sherpa, which
provides passwordless sudo and all dependencies pre-installed.

## Prerequisites

- A running `devbox_linux` VM built from `dev/integration-test/manifest.toml`
- SSH access to the VM
- Code synced to `/home/sherpa/sherpa/` inside the VM

## VM Setup

Build the test VM from the host:

```bash
cd dev/integration-test && sherpa up
```

SSH into the VM:

```bash
ssh -F dev/integration-test/sherpa_ssh_config testbox.<LAB_ID>
```

The lab ID can be found in `dev/integration-test/lab-info.toml`.

Sync code to the VM (from the host, in the project root):

```bash
rsync -az --delete \
  --exclude='target/' \
  --exclude='.git/' \
  --exclude='dev/integration-test/' \
  -e "ssh -F dev/integration-test/sherpa_ssh_config" \
  ./ testbox.<LAB_ID>:/home/sherpa/sherpa/
```

## Using the `/test-integration` Skill (Recommended)

Run the skill from within the VM (requires Claude Code installed in the VM):

```
/test-integration              # Run all integration tests
/test-integration db           # Run only db crate tests
/test-integration container    # Run only container crate tests
/test-integration network      # Run only network crate tests
/test-integration libvirt      # Run only libvirt crate tests
/test-integration server       # Run only server (sherpad) crate tests
```

### What the Skill Does

1. **Test DB restart** — Restarts SurrealDB inside the VM for a clean state
2. **Format + lint** — Runs `cargo fmt --check` and `cargo clippy` to catch issues early
3. **Test execution** — Runs integration tests sequentially per crate
4. **Report** — Shows pass/fail summary per crate

## Manual Execution

### Using the Script

```bash
./scripts/run-integration-tests.sh
```

This runs all integration tests sequentially (requires sudo for network/server tests).

### Per-Crate Commands

Start the test database first:

```bash
SHERPA_DEV_DB_USER=sherpa SHERPA_DEV_DB_PASS='Everest1953!' ./dev/testdb restart
```

Then run individual crate tests:

```bash
# Database tests
cargo test -p db -- --ignored --test-threads=1

# Container tests (requires Docker daemon)
cargo test -p container -- --ignored --test-threads=1

# Network tests (requires sudo for rtnetlink)
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p network -- --ignored --test-threads=1

# Libvirt tests (requires libvirtd)
cargo test -p libvirt -- --ignored --test-threads=1

# Server tests (requires sudo)
sudo -E env PATH="$HOME/.cargo/bin:$PATH" cargo test -p sherpad -- --ignored --test-threads=1
```

### Test Database Management

```bash
./dev/testdb start     # Start SurrealDB container (in-memory)
./dev/testdb stop      # Stop the container
./dev/testdb restart   # Destroy and recreate (fresh database)
./dev/testdb status    # Check if running
./dev/testdb logs      # Tail container logs
./dev/testdb destroy   # Stop and remove container
```

Default connection: `ws://localhost:42069` (user: root, pass: root)

## Test Conventions

- All integration tests use the `#[ignore]` attribute and run with `-- --ignored`
- Tests requiring shared state use `--test-threads=1` to avoid contention
- Database tests use per-test namespace isolation (timestamp + thread ID)
- Container tests prefix resources with `sherpa-test-` for safe identification
- Network and server tests require `sudo` for capability-dependent operations
