# Provisioners

Provisioners get secrets onto nodes without the server holding them: it stores their paths and
settings, never a file's contents. See `tests/specs/provisioning.md`
and sherpa-docs `architecture/provisioners/`.

- `ssh`, for Linux and BSD VMs: the CLI runs it itself over SSH once the node is ready; with
  `--ssh-jump` the server is only a jump host forwarding the encrypted stream.
- `container`, for containers: the CLI sends the files beside the manifest, and the server keeps
  them in RAM and mounts them read-only at `/run/secrets` before the container starts.

## Manifest

`topology::Node::provisioners` and `Manifest::provision_concurrency` are read from
`manifest.toml` but never serialized, so the manifest the CLI sends has none. The CLI sends them
in `ManifestAsWritten` instead, only so the server can store them in the lab's definition (see
[`DATABASE.md`](DATABASE.md#lab-definitions)); the server runs none of them. A manifest that
carries provisioners itself can only have come from the web UI or REST, which cannot run them, so
`server::validate::reject_provisioners` refuses it.

## Client

`crates/client/src/provision/` connects through Jato, as `sherpa ssh` does; no `ssh` program runs.

- **One connection per node**, kept for every step. Only failures at the node are retried until
  the ready timeout (refused or dropped connections, the login refused before cloud-init makes the
  account). A host key problem, the Sherpa server refusing the login, or a local file that cannot
  be read fails at once, as no wait would fix them.
- **Prompts.** Any hop may ask for a key's passphrase; only the jump to the Sherpa server may ask
  for a password, as a node's refused login while it boots must be retried, not answered. One
  `jato::PromptCache` serves the run, so many nodes ask once.
- **Commands** have no time limit; Jato's keepalives end one whose node stops answering.
- **Concurrency.** Each node is one future, readiness wait included, so its timeout starts when it
  gets a place, run with `buffer_unordered` on the caller's task: nothing needs to be `Send`.
  Lines are written whole, so nodes' output never mixes, and `HostKeyRefresh` makes nodes whose
  key is unknown rewrite `.sherpa/ssh/known_hosts` one at a time.

## Container secret files

`server::services::node_secrets` writes them under `/run/sherpa`, the daemon's systemd
`RuntimeDirectory` (a tmpfs that `RuntimeDirectoryPreserve=yes` keeps across restarts), so a
reboot clears them and `sherpa provision` sends them again. They are files, not environment
values, because Docker writes a container's environment into its config on disk. The bind mount is
fixed when the container is created, so `node.secrets` rewrites only a container that already
mounts its node's folder, and any other must be redeployed.
