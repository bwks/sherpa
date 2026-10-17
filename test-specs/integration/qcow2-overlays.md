# QCOW2 overlay verification

Production provisioning uses the existing libvirt storage-volume API. It never
executes QEMU commands. Header eligibility follows the [QEMU QCOW2 format](https://www.qemu.org/docs/master/interop/qcow2.html)
and volume XML uses [libvirt backing-store elements](https://libvirt.org/formatstorage.html#backing-store-elements).

Run formatting and workspace Clippy before any tests:

```sh
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

The unit tests cover node-only settings with VM overlay defaults and explicit copy requests, client expansion and
the expansion shared by startup/redeploy preserving optional node settings,
VM-only provisioning defaults, rejection of lab-level settings,
QCOW2 eligibility/capacity, XML escaping,
database rejection before filesystem deletion, orphaned backing references,
repeated dependency discovery without runtime state, concurrent provisioning and
deletion, and metadata retention until VM/disk cleanup succeeds.

The gated libvirt test needs a local QEMU/KVM connection, the existing
`sherpa-pool`, readable pool files, and `qemu-io`:

```sh
SHERPA_OVERLAY_LIVE=1 cargo test -p libvirt live_overlay_disk_isolation_resize_and_reset -- --ignored --nocapture
```

It creates disposable volumes, writes guest blocks on temporary user-owned
QCOW2 files and uploads those through libvirt. It verifies matching base reads,
independent writes, unchanged base contents, stop/start persistence, resizing only
the overlay, replacement after disk deletion, refusal to overwrite existing
disks, ordinary QCOW2 copies, and byte-identical auxiliary media. Cleanup removes
only volumes and domains created by the test. A failed cleanup reports the
remaining resource and preserves its backing volumes.

For the image reference integration tests, start `dev/testdb` with the `sherpa`
root login expected by `db::connect`, and provide its password through
`SHERPA_DB_PASSWORD`:

```sh
SHERPA_DEV_DB_USER=sherpa dev/testdb start
cargo test -p db node_image::delete_tests -- --ignored
```

These tests use isolated database namespaces, never the server's application
namespace. Referenced image deletion must fail and leave the image record intact.

Full Sherpa lab acceptance additionally exercises `up`, `down`, `resume`,
`redeploy`, and `destroy` through the authenticated application API: resume must
preserve writes, redeploy must reset them against the pinned image version, and
destroy must remove the node overlay while preserving the imported base. Inject
VM/disk deletion failures and verify ownership, image references, and saved
configuration remain available for a successful cleanup retry.

## Local measurement (2026-10-10)

Omarchy 4.0.4 was measured with a disposable benchmark test, since removed:

| Measurement | Copy | Overlay |
| --- | ---: | ---: |
| Disk creation | 15.560591 s | 0.025956 s |
| Domain startup API | 1.420619 s | 1.677043 s |
| Guest-agent polling | 90.063970 s (timeout) | 90.062449 s (timeout) |

Overlay disk creation was approximately 600 times faster (99.83% less time).
Neither isolated guest answered the guest agent within the limit. These results
establish disk-creation performance only, not time to a ready guest or Sherpa SSH
readiness. Disposable benchmark domains and disks were removed after the run.

The workspace test suite, four live image-deletion database tests, and the gated
libvirt isolation/resize/reset/copy/media test passed locally. Full authenticated
application lifecycle acceptance remains a separate environment-dependent check.
