# Provisioners

Provisioners are run by the `sherpa` CLI on lab nodes once they are ready, so secrets reach nodes
without passing through the server. The CLI connects with the lab's `sherpa_ssh_config`, through
the server as an SSH jump host; the server only forwards the encrypted stream. User-facing
behaviour is in sherpa-docs `architecture/provisioners.md`.

## Manifest

`topology::Node::provisioners` (`crates/topology/src/node.rs`) is a list of
`topology::Provisioner` (`crates/topology/src/provisioner.rs`), a `type`-tagged enum so more
types can be added. The only type is `Ssh(SshProvisioner)`: `files` (`ProvisionerFile { src,
dst, mode }`), `env_files` and `scripts`, all `deny_unknown_fields`.

The field is `#[serde(default, skip_serializing)]`: it is read from the user's `manifest.toml`
but never serialized, so it is absent from the JSON the client sends and from the
`manifest.toml` the server saves. Only `sherpa new` writes a manifest, and it writes a new one,
so nothing is lost by not serializing it.

## Validation

- `topology::validate::validate_provisioners` (client: `sherpa validate`, `sherpa up`, and before
  each run): Linux and BSD VMs only (`NodeConfig` kind and `OsVariant`); `dst` absolute or `~/`,
  with no empty, `.` or `..` parts and only `[A-Za-z0-9._+@-]` in each part, because it is used
  in `scp` paths and remote shell commands; modes as octal digits up to `777`; env file names
  unique per node; at least one action.
- `server::validate::reject_provisioners` (`up::up_lab`, `redeploy::redeploy_node`): any node with
  provisioners is refused. The CLI never sends them, so they only arrive from the web UI or REST,
  which cannot run them.

## Client

`crates/client/src/provision/`:

| File | Role |
|---|---|
| `run.rs` | `provision_lab`: selects nodes with provisioners (or `--node`), skips nodes missing from the ready list, runs each node, prints `shared::util::render_provision_table`, fails if any node was not provisioned |
| `ssh.rs` | The SSH provisioner: `wait_for_node` (retry the login until the account exists, then `cloud-init status --wait`), then files, env files and scripts. Client files are read before the node is changed |
| `remote.rs` | Pure builders of the POSIX `sh` commands run on the node |
| `transport.rs` | `SshTarget`: the only code that spawns `ssh`/`scp`, always with `-F sherpa_ssh_config`, `BatchMode=yes`, `ConnectTimeout` and `LogLevel=ERROR` |

`transport.rs` has two operations, run a command (optionally with stdin) and copy a file, which
match `lib_sshinto`'s `Session` and `Connection` (github.com/bwks/sshinto). Replacing the system
binaries with that library changes only this file.

Remote steps, as the node's login user:

- **Files:** `prepare_file_command` creates missing parents and an empty destination under
  `umask 077` and makes an existing file `0600`; `scp` then writes into it, so the contents are
  never readable by others; `finish_file_command` sets the final mode.
- **Env files:** copied like files to `~/.env.d/<name>` (`konst::NODE_USER_ENV_DIR`), then
  `env_loader_command` appends, once, a line that exports every readable file in `~/.env.d` to
  `~/.bash_profile`, else `~/.profile`: the same choice as `user_env_loader_runcmd` for
  `~/.sherpa_env` (see [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md)).
- **Scripts:** `script_wrapper_command` is the fixed remote command; the script arrives on stdin,
  is written to a `0700` temporary file, run through its shebang with `~/.sherpa_env` and
  `~/.env.d/*` exported, and removed. The command exits with the script's status. Secret data is
  never part of a command line.

## Lifecycle

- `cmd/up.rs`: after the SSH config, key and lab info are written and the results shown, with the
  nodes that came up with an SSH port as the ready list.
- `cmd/redeploy.rs`: after the `download` refresh, for the redeployed node.
- `cmd/provision.rs`: `sherpa provision [--node]`, on demand.

A failure is reported per node and makes the command exit non-zero; the lab is left running.
