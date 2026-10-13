# Personal Linux VM account acceptance

The ignored `current_user_vm_login_sudo_resume_and_owner_redeploy` test in
`crates/server/tests/lab_lifecycle_tests.rs` requires a disposable Sherpa host
with Docker, libvirt/KVM, the normal Sherpa configuration and assets, a test
SurrealDB instance, and a downloaded Linux VM image. Use the existing server
test setup instructions in [`HOW-TO-RUN.md`](HOW-TO-RUN.md). This test creates and
destroys a VM lab through the existing APIs and uses SSH directly to its
management address from the server host.

Provide two distinct, unencrypted SSH private key files with matching `.pub`
files. Set `SHERPA_GUEST_TEST_KEY` and `SHERPA_GUEST_TEST_ROTATED_KEY` to their
paths. The test reads these keys; it does not modify them. Set
`SHERPA_GUEST_TEST_MODEL` to `ubuntu_linux`, `omarchy_linux` or `flatcar_linux`,
and optionally `SHERPA_GUEST_TEST_IMAGES_DIR` to the image directory. Run once
for each supported image under acceptance. The default model is Ubuntu.

```sh
cargo fmt
cargo clippy --workspace -- -D warnings
cargo test -p sherpad --test lab_lifecycle_tests current_user_vm_login_sudo_resume_and_owner_redeploy -- --ignored --nocapture --test-threads=1
```

The checks cover:

- Authenticated lab ownership despite a spoofed client `username`.
- Guest account creation, home directory, Bash, public-key login and passwordless sudo.
- Rejection of missing-key startup before a lab record is created, and missing-key redeploy while the original VM remains accessible.
- Resume retaining old keys after a profile change.
- Owner redeploy using the owner's newly registered keys and rejecting removed keys.
- Admin download returning refreshed personal SSH metadata without restricting local identities.

The test attempts lab cleanup on check failures. If transport or cleanup fails,
inspect and destroy the test lab on the disposable host before reusing it.
Unit tests cover portable usernames, malformed keys, key deduplication, password
omission, supported generators, custom ZTP rejection, legacy metadata, mixed SSH
configuration and legacy private-key permission enforcement without services.
