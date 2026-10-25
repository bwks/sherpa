# Provisioners

Provisioners get secrets onto lab nodes without the server storing them. There are two types:

- `ssh`, for Linux and BSD VMs: the `sherpa` CLI runs it on the node once it is ready, with the
  lab's `sherpa_ssh_config`, through the server as an SSH jump host. The server only forwards the
  encrypted stream.
- `container`, for containers: the CLI sends the files to the server outside the manifest, and
  the server keeps them in RAM and mounts them read-only at `/run/secrets` before the container
  starts. See [Container secret files](#container-secret-files).

User-facing behaviour is in sherpa-docs `architecture/provisioners/` (`overview.md`, `ssh.md`,
`container.md`).

## Manifest

`topology::Node::provisioners` (`crates/topology/src/node.rs`) is a list of
`topology::Provisioner` (`crates/topology/src/provisioner.rs`), a `type`-tagged enum:
`Ssh(SshProvisioner)` with `files` (`ProvisionerFile { src, dst, mode }`), `env_files` and
`scripts`, and `Container(ContainerProvisioner)` with `files` (`ContainerSecretFile { src, name,
mode }`, mode default `konst::CONTAINER_SECRET_DEFAULT_MODE`, `444`). All are
`deny_unknown_fields`.

The field is `#[serde(default, skip_serializing)]`: it is read from the user's `manifest.toml`
but never serialized, so it is absent from the JSON the client sends and from the
`manifest.toml` the server saves. Only `sherpa new` writes a manifest, and it writes a new one,
so nothing is lost by not serializing it.

## Validation

- `topology::validate::validate_provisioners` (client: `sherpa validate`, `sherpa up`,
  `sherpa redeploy`, `sherpa provision`): `ssh` on Linux and BSD VMs only (`NodeConfig` kind and
  `OsVariant`), `container` on containers only. Container files: names and modes checked by
  `shared::validators::validate_secret_file` (one path part of `[A-Za-z0-9._+@-]`), unique per
  node. ssh files: `dst` absolute or `~/`,
  with no empty, `.` or `..` parts and only `[A-Za-z0-9._+@-]` in each part, because it is used
  in `scp` paths and remote shell commands; modes as octal digits up to `777`; env file names
  unique per node; at least one action.
- `server::validate::reject_provisioners` (`up::up_lab`, `redeploy::redeploy_node`): any node with
  provisioners is refused. The CLI never sends them, so they only arrive from the web UI or REST,
  which cannot run them.
- `server::validate::validate_node_secrets` (`up_lab`, `redeploy_node`, before anything is
  created): secret files are only for container nodes in the manifest, with valid, unique names,
  valid modes and base64 content (`validate_secret_files`, also run by every write).

## Client

`crates/client/src/provision/`:

| File | Role |
|---|---|
| `run.rs` | `provision_lab`: selects nodes with ssh provisioners (or `--node`), skips nodes missing from the ready list, runs each node, prints `shared::util::render_provision_table` with any results passed in for container nodes, fails if any node was not provisioned |
| `container.rs` | `container_secrets`/`node_secrets`: reads container provisioner files into `shared::data::SecretFile`s; `container_results` reports them |
| `client_files.rs` | Client path rules shared by both types: `~` expanded, relative paths from the manifest's folder |
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

## Container secret files

`shared::data::SecretFile { name, mode, content }` carries a file; `SecretContent` is its base64
content, with a `Debug` that prints `[REDACTED]`. They travel outside the manifest, which is
saved:

| Request | Field | Meaning |
|---|---|---|
| `UpRequest` (`up` RPC, `POST /api/v1/labs`) | `node_secrets: BTreeMap<node, Vec<SecretFile>>` | Files for each container node |
| `RedeployRequest` (`redeploy` RPC, REST) | `secrets: Option<Vec<SecretFile>>` | `Some` replaces the node's files and an empty list removes them; `None` keeps them. The CLI always sends `Some`; the web UI sends `None`, as it has no files |
| `NodeSecretsRequest` (`node.secrets` RPC) | `files` | Rewrite a running container's files, for `sherpa provision` |

`server::services::node_secrets`:

- `write_node_secrets` writes `konst::SHERPA_SECRETS_DIR/<lab>/<node>/` (see `SERVER.md`), checks
  every file before writing, writes each one under a temporary name, `0600`, then sets its mode
  and renames it, so the container never sees a partial file, and removes files no longer sent.
- `up_lab` writes every node's files once, before the readiness loop, and adds
  `<dir>:/run/secrets:ro` (`konst::CONTAINER_SECRETS_DIR`) to that container's binds. A failure
  rolls the lab back.
- `redeploy_node` uses `secrets_for_redeploy` for the `Option` above.
- `update_node_secrets` (`node.secrets`) requires the lab owner and a container node, and rewrites
  the files only if the container mounts `/run/secrets` from that node's directory
  (`container::get_container_mount_source`): the mount is fixed when the container is created,
  so a container created without one must be redeployed.
- `destroy_lab` and `clean_lab_resources` remove the lab's directory after its containers
  (`destroy::destroy_lab_secrets`).

Files only: values put in a container's environment are stored by Docker in its container
config on disk, so images read secrets through their `*_FILE` variables instead.

## Lifecycle

- `cmd/up.rs`: container secret files go in the `up` request. ssh provisioners run after the SSH
  config, key and lab info are written and the results shown, with the nodes that came up with an
  SSH port as the ready list.
- `cmd/redeploy.rs`: the node's secret files go in the request; ssh provisioners run after the
  `download` refresh, for the redeployed node.
- `cmd/provision.rs`: `sherpa provision [--node]`, on demand: `node.secrets` for container nodes,
  then ssh provisioners.

A failure is reported per node and makes the command exit non-zero; the lab is left running.
