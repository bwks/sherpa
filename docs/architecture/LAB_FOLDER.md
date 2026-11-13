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
| `.sherpa/ssh/key` | `lab_files::save_lab_files` (only when the SSH config uses the lab's key) | 0600 |
| `.sherpa/ssh/known_hosts` | `lab_files::write_known_hosts`, also on a host key refresh | 0600 |
| `.sherpa/ca/lab-ca.crt` | `lab_files::write_lab_ca` | default |
| `.sherpa/vault/vault-<node>.toml` | `vault_credentials` | 0600 |
| `.sherpa/keycloak/keycloak-<node>.toml` | `keycloak_credentials` | 0600 |
| `.sherpa/talos/talosconfig-<cluster>` | `cluster_credentials` | 0600 |
| `.sherpa/k8s/kubeconfig-<cluster>` | `cluster_credentials` | 0600 |

Secret files are written by `private_key::write_secret_file` (a new owner-only file renamed over
the old one; a current-user-only DACL on Windows). Folders are made by
`private_key::ensure_private_dir`, which refuses a symlink or a file in the folder's place, so
nothing is ever written through a link. Folder modes are set on Unix only.

The server still sends the files' contents with its own names. Its SSH config names the key and
known hosts relatively (`IdentityFile sherpa_ssh_key`, `UserKnownHostsFile sherpa_known_hosts`);
`lab_files::rewrite_ssh_config_for_client` replaces them with the absolute paths of
`.sherpa/ssh/key` and `.sherpa/ssh/known_hosts`. The config therefore works from anywhere:
through the `Include` in `~/.ssh/sherpa_lab_hosts`, with `ssh -F .sherpa/ssh/config <node>` from
the lab folder, for `sherpa ssh` and for the provisioners. The server's own lab folder
(`/opt/sherpa/labs/<lab-id>`) keeps the `sherpa_*` names.

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
4. Writes `sherpa_ssh_config` as `.sherpa/ssh/config` with its `IdentityFile` and
   `UserKnownHostsFile` lines for the old key and known hosts pointed at `.sherpa/ssh/`, then
   removes the old file.
5. Moves `lab-info.toml` last, so a cut-short migration is still seen as a lab's.
6. Replaces the `Include` of the old config in `~/.ssh/sherpa_lab_hosts` with one of
   `.sherpa/ssh/config`, in place (`shared::util::replace_lab_ssh_include`). Nothing changes when
   the old one is not included.

Each step is idempotent, so a migrated folder, or one that is not a lab's, is left as it is.
