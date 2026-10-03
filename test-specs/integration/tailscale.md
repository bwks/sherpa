# Tailscale Testing

Developer verification guide for the Tailscale gateway and lab lifecycle changes.
Run the commands below from the Sherpa source repository. For manifest setup,
credentials, and route approval, see [Tailscale setup](../../docs/TAILNET.md).

## Automated Regression Tests

Run these commands from the Sherpa Rust source repository, completing formatting
and linting successfully before running tests:

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test -p shared -p container -p validate -p sherpad --lib
```

The tests use mock Docker API responses and do not require a live Docker daemon,
SurrealDB, or a Tailscale credential. They cover ordinary containers with
Tailscale-like names, gateway ownership protection, failed gateway and volume
cleanup with retained metadata and successful retries, and status checks when the
full tailnet peer inventory exceeds the Docker exec output limit. They also cover
configuration defaults, partial overrides, rejected settings, configurable gateway commands,
and command/stop timeout overrides.

## Docker Smoke Test

On a Linux host with Docker, TUN, and nftables support, run this from the Rust
source repository after the formatting and linting checks above:

```bash
cargo test -p container live_gateway_daemon_smoke -- --ignored --nocapture
```

No Tailscale credential is needed. The test creates and removes a disposable
network, gateway, and volume, and checks IPv4/IPv6 forwarding restrictions,
credential delivery, stop, restart, and cleanup. The configured image is pulled if absent.

Both Docker tests use the standard server configuration defaults unless
`SHERPA_TEST_SERVER_CONFIG` names a complete server `sherpa.toml` file. To verify
overrides, point it at a disposable configuration with a different `socket_path`
and run the smoke test again. The test requires the chosen image to include
`tailscaled`, `tailscale`, and the existing nftables tools.

## Live Enrollment Test

Use a disposable test tailnet and prepare a disposable Docker management network.
Set these environment variables in the shell running the test:

| Variable | Value |
| -------- | ----- |
| `SHERPA_TEST_SERVER_CONFIG` | Optional server TOML file containing gateway setting overrides |
| `SHERPA_TEST_TAILSCALE_AUTH_KEY` | Auth key for the test tailnet, supplied through a secret manager or secure prompt |
| `SHERPA_TEST_TAILSCALE_NETWORK` | Existing disposable Docker management network name |
| `SHERPA_TEST_TAILSCALE_IPV4` | Unused gateway IPv4 address within that network |
| `SHERPA_TEST_TAILSCALE_IPV6` | Optional unused gateway IPv6 address within that network |
| `SHERPA_TEST_TAILSCALE_ROUTES` | Comma-separated management prefixes to advertise |

After formatting and linting pass, run:

```bash
cargo test -p container live_tailnet_enrollment_stop_resume_cleanup -- --ignored --nocapture
```

The test checks enrollment, repeated stop, resume with unchanged Tailscale
addresses, and repeated cleanup. It removes its gateway and identity volume;
the supplied Docker network remains. Use this only with disposable test resources.

## Manual Lab Acceptance

Run rebuilt Sherpa server and CLI binaries against a disposable lab containing
both VM and container nodes. Enable `[tailscale]` and load the client auth key as
described in the [setup guide](../../docs/TAILNET.md#manifest). Run each command
separately from the lab directory:

| Step | Expected result |
| ---- | --------------- |
| `sherpa up`, then `sherpa inspect` | Gateway connects; inspect shows its addresses and allocated management routes |
| Approve routes and inspect again | Approval reflects the update; remote SSH/HTTPS to node management addresses works over IPv4 and configured IPv6 |
| `sherpa down` | Gateway stops and remote management access through it stops |
| `sherpa resume`, then `sherpa inspect` | Gateway has the same Tailscale addresses; management access works again |
| `sherpa destroy` | Test nodes, gateway, identity volume, database records, and server-side lab directory are removed |

Repeat with automatic route approval, manual approval, an invalid enrollment key,
and Tailscale disabled. Create another lab and confirm that its management
addresses cannot be reached through the first lab's gateway. Include an ordinary
container node named `sherpa-tailnet-app` and confirm destroy removes it.

To verify cleanup retry against real Docker, temporarily keep the test lab's
existing identity volume mounted in a separate disposable container. Use a
read-only mount; the holder needs no Tailscale credentials. Destroy should report
a volume-removal failure while preserving database ownership and the server's
saved lab files. Remove the holding container and retry destroy as the same owner;
cleanup should succeed. Repeat with a fresh test lab using
`sherpa server clean <lab_id>` as an administrator.
