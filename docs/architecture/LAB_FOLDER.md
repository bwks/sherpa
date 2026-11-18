# Lab Folder Files

The Sherpa CLI keeps every file it generates for a lab in a `.sherpa` folder beside the lab's
`manifest.toml`. Nothing it generates is left loose in the lab folder, so `sherpa destroy` can
remove all of it by deleting one folder.

## Layout

`shared::util::LabFolder` is the only place the paths are built (`crates/shared/src/util/lab_folder.rs`).
The folder name is `konst::LAB_STATE_DIR`; `shared` uses `LabFolder` too, to find a lab from the
`Include` lines in `~/.ssh/sherpa_lab_hosts`.

| Path | Written by | Mode |
|---|---|---|
| `.sherpa/` and every folder in it | `lab_files::ensure_lab_subdir` | 0700 |
| `.sherpa/lab-info.toml` | `lab_files::save_lab_files` | default |
| `.sherpa/ssh/config` | `lab_files::save_lab_files` | default |
| `.sherpa/ssh/known_hosts` | `lab_files::write_known_hosts`, also on a host key refresh | 0600 |
| `.sherpa/ca/lab-ca.crt` | `lab_files::write_lab_ca` | default |
| `.sherpa/vault/vault-<node>.toml` | `vault_credentials` | 0600 |
| `.sherpa/keycloak/keycloak-<node>.toml` | `keycloak_credentials` | 0600 |
| `.sherpa/surrealdb/surrealdb-<node>.toml` | `surreal_db_credentials` | 0600 |
| `.sherpa/talos/talosconfig-<cluster>` | `cluster_credentials` | 0600 |
| `.sherpa/k8s/kubeconfig-<cluster>` | `cluster_credentials` | 0600 |

Secret files are written by `secret_file::write_secret_file` (a new owner-only file renamed over
the old one; a current-user-only DACL on Windows). Folders are made by
`secret_file::ensure_private_dir`, which refuses a symlink or a file in the folder's place, so
nothing is ever written through a link. Folder modes are set on Unix only.

The server still sends the files' contents with its own names. Its SSH config names the known
hosts relatively (`UserKnownHostsFile sherpa_known_hosts`) and names no identity file, so a node
logs in with the client's own keys;
`lab_files::rewrite_ssh_config_for_client` replaces the known hosts path with the absolute path
of `.sherpa/ssh/known_hosts`. The config therefore works from anywhere:
through the `Include` in `~/.ssh/sherpa_lab_hosts`, with `ssh -F .sherpa/ssh/config <node>` from
the lab folder, for `sherpa ssh` and for the provisioners. The server's own lab folder
(`/opt/sherpa/labs/<lab-id>`) keeps the `sherpa_*` names.

The server's lab folder holds no copy of the manifest, the lab info or the SSH files: the lab's
manifest is its definition in the database (see [`DATABASE.md`](DATABASE.md#lab-definitions)),
and `lab-info.toml`, the SSH config and `known_hosts` the server sends are built from the lab's
and its nodes' records when `up`, `download` or `node.host_keys` returns them
(`lab_info::load_lab_info`, `ssh_config::lab_ssh_files`). Their format is unchanged, so every
client reads them as before. Each Talos cluster's plan is in the database too. The
`manifest.toml`, `lab-info.toml`, SSH files and `clusters/<cluster>/cluster.toml` older releases
saved there were imported and deleted at startup by 0.6.4 and 0.6.5; 0.7.0 ignores any left
behind (see [`SERVER.md`](SERVER.md#files-older-releases-left-in-lab-folders)). What the server's
folder keeps is the lab's CA and the container and VM certificates, Talos,
Vault, Keycloak and SurrealDB credentials (each 0600 in `vault/`, `keycloak/` and `surrealdb/`,
the application nodes' admins among them, see [`LAB_CA.md`](LAB_CA.md#keycloak)), and the files read while a node exists: the dnsmasq config
(`ztp/dnsmasq/dnsmasq.conf`, mounted in the boot container), the TFTP and HTTP files of the nodes
that boot by them (`ztp/tftp/`, `ztp/configs/`) and each container's config mounts (`<node>/`).
A lab from before 0.7.0 also still has its old SSH key pair, `sherpa_ssh_key` and `.pub`, until it
is destroyed (see [`SERVER.md`](SERVER.md#no-lab-ssh-key)). It does not keep:

- **A VM's boot files** (`vm-boot.d/<node>/`: its seed, config files, `ztp.iso`, config disks,
  Ignition file): they are read once, to copy the VM's media into the storage pool, and removed
  straight after (see [`ZTP_ISO.md`](ZTP_ISO.md#from-config-files-to-the-guest)). A failed copy
  keeps them until `destroy` or the node's next `redeploy`. Node names allow only letters,
  digits, `-` and `_`, so `vm-boot.d` is never a node's folder, and `vm-boot.d/<node>` never one
  of the lab's own folders, whatever the VM is called.
- **A dnsmasq leases file**: dnsmasq keeps its leases inside the boot container, outside its HTTP
  root (see [`ZTP.md`](ZTP.md#the-boot-server)).
- **Unikernel certificates**: none are issued (see [`LAB_CA.md`](LAB_CA.md#node-certificates)).
- **Another ZTP method's network boot files**: a `redeploy` that moves a VM off TFTP or HTTP
  removes that node's TFTP config or HTTP files (`node_ops::remove_stale_network_boot_files`),
  and no other node's. The boot server's config changes with it: see
  [`ZTP.md`](ZTP.md#redeploy-and-the-boot-server).

A node's own folder, `<node>/`, still holds a container's config mounts, and held a VM's boot
files in releases before 0.6.5. It is removed by name (the redeploy teardown) only through `node_ops::node_dir`, which gives none for a node named like one of the
lab's own folders (`certs`, `ztp`, `clusters`, `vault`, `keycloak`): that folder holds the lab
CA, the boot server's files or credentials, so it is kept, with a warning. A container named
like one writes its config files into it, as before.

Older releases left the first three; 0.6.5 removed them at startup, and 0.7.0 ignores any left
behind (see [`SERVER.md`](SERVER.md#files-older-releases-left-in-lab-folders)). The user's `manifest.toml` in
their own lab folder is the CLI's, and is never touched.

### Node config modes

Every config Sherpa renders for a node, and the folder it is in, gets its mode explicitly,
whatever `sherpad`'s umask: files through `shared::util::write_file_with_permissions`, folders
through `shared::util::create_dir_with_permissions`. The modes are constants in `server::konst`.

| Files | Folder | File mode | Folder mode | Read by |
|---|---|---|---|---|
| A VM's boot files: cloud-init, cloudbase-init and NoCloud seeds, CD-ROM, disk, USB and Ignition sources, the PAN-OS bootstrap | `vm-boot.d/<node>/` and its subfolders | `0600` | `0700` | `sherpad`, building and uploading the media |
| SONiC's HTTP files | `ztp/configs/`, the boot container's HTTP root | `0600` | `0700` | the boot container's HTTP server, as root |
| TFTP configs (the `arista_eos` and `juniper_evolved` VMs, `aruba_aoscx`) | `ztp/tftp/`, the TFTP root, not served over HTTP | `0604` | `0705` | dnsmasq in the boot container, as root |
| Container config mounts (cEOS, SR Linux, cJunosEvolved, IOL, FRR, Vault) | `<node>/` and its subfolders | `0600` | `0700` | the container's own processes, as root |

`ztp/` itself is `0700`. A container reads its mounts through Docker, which resolves the bind
mount's source as root, so the folders above a mount do not need to let the container in; only
the mounted file or folder's own mode matters to the process in the container.

- **TFTP is the one exception.** dnsmasq serves only files others can read: as root, as it
  runs in the boot container, it serves world-readable files only, and started otherwise it
  drops to the container's `dnsmasq` user, which owns nothing on the host and refuses a TFTP root
  it cannot list. The configs are readable by others, and the folder readable and searchable by
  them, so TFTP works whichever user dnsmasq runs as; the group, which on the host is the people
  in `sherpa`, gets nothing. On the host, others cannot reach the lab folder (`labs/` is
  `2750`).
- **The HTTP server serves all of `ztp/`'s mounts**, so a TFTP config can be fetched over HTTP
  too. Any node on the lab's management network can fetch any node's TFTP or HTTP config, without
  credentials: the modes protect the files on the host, not on that network.
- **Media are not rendered**: the `ztp.iso`, config disk and data disk copies in a VM's boot folder
  get the umask's mode, inside the `0700` folder, until they are in the storage pool and the
  folder is removed. An ISO's Rock Ridge modes are fixed (`ZTP_ISO.md`), whatever the source
  files'.
- Each container model's reader runs as root: cEOS (`/sbin/init`), SR Linux (`0:0`), FRR (the
  startup script, `frrinit.sh` and `vtysh -b`; the daemons run as `frr` but do not read
  `frr.conf`), Vault (Sherpa's command runs `vault` without the image's switch to the `vault`
  user), and IOL and cJunosEvolved, whose launchers need `NET_ADMIN` and a privileged container,
  which only root has.
- **SR Linux takes its config over.** Its startup, as root, gives `config.json` to its
  `srlinux` user (uid 1002 in the image) and opens it to everyone (`0777` and an ACL), for the
  processes that then run as that user. The mount reaches the host file, so on the host it is
  owned by uid 1002 and world-writable; the `0700` node folder is what keeps it private there.
  `sherpa` can still remove it, as it owns the folder.
- The dnsmasq config, `ztp/dnsmasq/dnsmasq.conf`, is not a node config and keeps the umask's
  mode; dnsmasq reads it as root before it drops root.

## Lab identity

`cmd::lab_identity` works out which lab a folder holds, for every per-lab command.
`.sherpa/lab-info.toml` holds the lab's ID as the server gave it, and is the lab's identity
whenever it is there and parses (`resolve_lab_identity`). The OS username plays no part, so a
folder made by `sherpa download`, or a lab started by another OS user, is found by it. When
`manifest.toml` is there too, its `name` must be the lab's name in `lab-info.toml`, or the
command is refused with a message to destroy that lab or remove `lab-info.toml` first. Names
are compared, not IDs, so a lab keeps the ID it was given even when today's rule would give
another.

A folder with only a manifest names a lab by its name. Its ID is the one `sherpa up` gives a
new lab: `get_id_for_user` of the Sherpa username in the login token (`token::username_from_token`)
and the lab's name, the rule the web UI uses for the labs it creates. Two Sherpa users with the
same OS name therefore get different IDs, and a user's ID is the same from every machine.
Older clients hashed the OS username (`USER`, or `USERNAME` on Windows) instead; their
labs keep those IDs through `lab-info.toml`.

| Command | Needs `manifest.toml` | Data from the server |
|---|---|---|
| `inspect`, `ssh`, `down`, `resume`, `destroy` | no; `resume` reads it when present for the Tailscale key, `ssh` to check the node name | |
| `console` | no | none: the server's console route takes the lab ID and node name, and refuses a container itself |
| `link impair` | no | the link's `index` from `inspect`, matched by either end (`node::interface`) |
| `up`, `redeploy`, `provision` | yes; `lab_identity::load_source_manifest` says so in a folder without one | |

`sherpa download` writes `lab-info.toml`, the SSH files and the lab's credentials, and no
manifest: the definition the server holds is not the user's source (it has no file contents
and no secrets).

A server older than link `index` sends none. `link impair` then falls back to the folder's
manifest, by the link's place in it, as clients before it did, and without a manifest says the
server is too old. Older clients ignore the field. A server older than the console route answers
`console` with a 404, and the client says the server is too old (see
[`CONSOLE_CLIENT.md`](CONSOLE_CLIENT.md#errors)).

## Destroy

After the server destroys the lab, `cmd::destroy::remove_local_lab_state` removes the lab's
`Include` from `~/.ssh/sherpa_lab_hosts` and deletes `.sherpa/` whole
(`destroy::remove_state_dir`). A symlinked `.sherpa` is refused, and nothing else in the lab
folder is touched. Failures only warn: the lab itself is gone.

`destroy` is the one command that does not stop on a failed migration. `cli::tolerate_failed_migration`
turns the migration's error into a warning (logged, and printed to the user) for `destroy` only;
every other command returns the error. The cleanup therefore handles either layout
(`destroy::remove_lab_files`): it removes the `Include` of both `.sherpa/ssh/config` and the old
`sherpa_ssh_config`, deletes `.sherpa/` whole when it is a real folder (a symlink is still
refused), and deletes the files an older client left loose by their exact generated names
(`lab_folder_migration::legacy_lab_files`, the same selection the migration uses, so a user's
own `vault-notes.toml` or `kubeconfig-prod` stays).

## Migration from the old layout

Clients before `.sherpa` saved `sherpa_ssh_config`, `sherpa_ssh_key`, `sherpa_known_hosts`,
`lab-info.toml`, `lab-ca.crt` and the credential files loose in the lab folder.
`client::lab_folder_migration` moves them, and is the only code that knows the old names. It is
temporary: remove it and its calls in a later release.

`cmd::cli::run` calls `migrate_current_lab_folder` before every command that uses the lab folder
(`uses_lab_folder`: all but `login`, `whoami`, `logout`, `new`, `init`, `validate`, `cert`,
`image` and `server`). `ssh-config inspect` and `clean` also migrate every lab folder an old
`sherpa_ssh_config` Include names (`migrate_registered_lab_folders`), warning on a failure. After
that, every reader knows only `.sherpa`.

`migrate_lab_folder`:

1. Refuses a symlinked `.sherpa`, or a file in its place.
2. Does nothing unless the folder is a lab's: a loose `lab-info.toml` or `sherpa_ssh_config`, or
   an existing `.sherpa` from an earlier, cut-short migration.
3. Renames each loose file into its new place, tightening secrets to 0600. Only files the
   client generated move: `vault-*.toml` and `keycloak-*.toml` whose top-level keys are exactly
   the client's, and the kubeconfig and talosconfig of clusters in the manifest or with both
   files present. A loose file replaces its copy in `.sherpa`, as an older client wrote it later.
4. Writes `sherpa_ssh_config` as `.sherpa/ssh/config` with its `UserKnownHostsFile` lines for
   the old known hosts pointed at `.sherpa/ssh/known_hosts`, then removes the old file. The old
   lab key, `sherpa_ssh_key`, and the `IdentityFile` lines naming it are left as they are:
   Sherpa has no lab key from 0.7.0, and the labs of clients that saved it loose had to be
   destroyed before the update to 0.6.0.
5. Moves `lab-info.toml` last, so a cut-short migration is still seen as a lab's.
6. Replaces the `Include` of the old config in `~/.ssh/sherpa_lab_hosts` with one of
   `.sherpa/ssh/config`, in place (`shared::util::replace_lab_ssh_include`). Nothing changes when
   the old one is not included.

Each step is idempotent, so a migrated folder, or one that is not a lab's, is left as it is.
