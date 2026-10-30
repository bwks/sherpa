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
  in SFTP paths and remote shell commands; modes as octal digits up to `777`; env file names
  unique per node; at least one action. `provision_concurrency`, when set, at least 1.
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
| `run.rs` | `provision_lab`: selects nodes with ssh provisioners (or `--node`), skips nodes missing from the ready list, runs the nodes concurrently (see [Concurrency](#concurrency)), prints `shared::util::render_provision_table` with any results passed in for container nodes, fails if any node was not provisioned |
| `container.rs` | `container_secrets`/`node_secrets`: reads container provisioner files into `shared::data::SecretFile`s; `container_results` reports them |
| `client_files.rs` | Client path rules shared by both types: `~` expanded, relative paths from the manifest's folder |
| `env_file.rs` | Resolving `KEY=$NAME` lines of env files from the client's environment |
| `ssh.rs` | The SSH provisioner: `wait_for_node` (connect, retrying failures at the node until the account exists, then `cloud-init status --wait`), then files, env files and scripts over that one connection. Client files are read before the node is changed |
| `output.rs` | `NodeOutput`: one node's progress lines and script output, each line prefixed with the node's name and written whole |
| `remote.rs` | Pure builders of the POSIX `sh` commands run on the node |
| `transport.rs` | `SshTarget` (a node's route in `sherpa_ssh_config`) and `NodeSession` (one Jato connection): run a command, run one with stdin and its output shown, copy a file over SFTP |

`transport.rs` connects with the Jato library through `crate::ssh_client`, the same code as
`sherpa ssh` (see [`SSH_CLIENT.md`](SSH_CLIENT.md)); no `ssh` or `scp` program is run.

- **One connection per node.** `wait_for_node` connects with a 15 s connection limit
  (`konst::PROVISIONER_SSH_CONNECT_TIMEOUT`) and retries every 5 s until the ready timeout,
  but only failures at the node itself (`ssh_client::retryable`): refused connections, and the
  login refused because cloud-init has not created the account yet. A host key problem or the
  Sherpa server refusing the login fails at once. All steps then share the connection.
- **Unknown nodes fail at once.** A `<node>.<lab-id>` alias with no `Host` block in
  `sherpa_ssh_config`, such as a node added to the manifest after the lab was created, would
  resolve as in `ssh` to a direct connection to the alias as a hostname and fail its DNS
  lookup until the ready timeout. `SshTarget::new` refuses a route with no jump host instead,
  as every lab node is reached through the server.
- **Prompts.** Only the jump to the Sherpa server may ask for a key passphrase or password
  (`PromptScope::JumpHostsOnly`); a node is never asked about, as its login fails while it
  boots. One `jato::PromptCache` serves the whole run, so provisioning many nodes asks once. An
  unknown server host key is asked about as in `sherpa ssh`. The authentication limit is
  `konst::SSH_PROMPT_AUTH_TIMEOUT` (300 s) so typing is not cut off. Jato's `PromptCache`
  (from v0.4.3) asks one question at a time across all its prompters and re-checks the cache
  once it has its turn, so nodes connecting at once ask once.
- **Commands** run through `Session::exec` with no time limit, as the old `ssh` had none;
  Jato's keepalives end the command if the node stops answering (about 90 s). Plain commands
  keep their stderr for the error; scripts, sent on stdin, show their output as it arrives.

Remote steps, as the node's login user:

- **Files:** `prepare_file_command` creates missing parents and an empty destination under
  `umask 077` and makes an existing file `0600`; Jato's SFTP upload then writes the contents
  to a new `0600` temporary file beside it and renames it over the destination, so they are
  never readable by others; `finish_file_command` sets the final mode.
- **Env files:** read as text and resolved by `env_file::resolve_env_file`: a line whose
  value is exactly `$NAME` or `${NAME}` (optionally indented or after `export`) gets the
  client's value of `NAME` (`client_variable`), single-quoted with `shell_quote`; other lines
  and line endings are kept, so `$HOME/bin:$PATH` still expands on the node. An unset variable
  fails before the node is changed, naming the file, line and variable but never a value. The
  resolved text is uploaded (`NodeSession::copy_to` takes the content read up front), like
  files, to `~/.env.d/<name>` (`konst::NODE_USER_ENV_DIR`), then
  `env_loader_command` appends, once, a line that exports every readable file in `~/.env.d` to
  `~/.bash_profile`, else `~/.profile`: the same choice as `user_env_loader_runcmd` for
  `~/.sherpa_env` (see [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md)).
- **Scripts:** `script_wrapper_command` is the fixed remote command; the script arrives on stdin
  and is written to a `0700` temporary file. The user's shell is started as a login shell
  (`"${SHELL:-/bin/sh}" -l -c`), so `/etc/profile`, `/etc/profile.d` and the user's login file
  set up `PATH` and tools as for a login (mise's shims, for example, come from
  `/etc/profile.d` on devbox images). FreeBSD's `sh`, and `csh`/`tcsh` with `-c`, refuse `-l`
  (they are login shells only when started as `-sh`), so the wrapper first tries
  `"$SHELL" -l -c :` and, if that fails, runs `/bin/sh -c` reading `/etc/profile` and
  `~/.profile` itself, as a `sh` login does. Either way it then exports `~/.sherpa_env` and `~/.env.d/*`, so those
  win, and `exec`s the script through its shebang. The outer shell removes the file afterwards.
  The command exits with the script's status. Secret data is never part of a command line.

## Concurrency

`provision_lab` builds one future per node (`provision_one`: skip check, `SshTarget`,
`wait_for_node`, then the provisioners) and runs them with `buffer_unordered`, up to
`Manifest::provision_concurrency` at once (`konst::PROVISION_CONCURRENCY`, 8, when unset). The
field is client-only and `#[serde(default, skip_serializing)]` like `provisioners`, so servers
never see it; `validate_provisioners` refuses `0`. All futures run on the caller's task, so
nothing needs to be `'static` or `Send`.

- **Readiness and work together.** Each node's readiness wait is part of its future, so its
  ready timeout starts when it gets a place, and a finished node frees its place at once
  (unordered, so a slow node does not hold up later ones). `in_manifest_order` puts the
  results back in manifest order for the table.
- **Output.** `NodeOutput` prefixes every line with the node name, padded to the longest name
  (`<node> | <line>`), keeps a partial line until it ends, and writes each batch of lines with
  one `write_all` on a locked stdout or stderr, so lines from different nodes never mix. A
  last line without a newline is shown when the command ends. Each node ends with a
  `provisioned` or `failed` line.
- **Host key refresh.** `HostKeyRefresh` clones share a `tokio::sync::Mutex`, so nodes whose
  key is unknown at once refresh `sherpa_known_hosts` one at a time and never write it
  together. A connection that reads the file while it is rewritten sees an unknown key and
  refreshes again.

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
