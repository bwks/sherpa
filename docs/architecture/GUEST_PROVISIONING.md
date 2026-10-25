# Guest Provisioning

How a VM node's login, manifest `environment_variables`, `text_files`, `startup_scripts`,
`user_scripts` and TLS certificates reach the guest at first boot. The generators are in
`crates/server/src/services/node_ops.rs` (`generate_cloud_init_ztp`, `generate_ignition_ztp`),
the guest login is in `crates/server/src/services/guest_account.rs`, and the documents are
rendered by `crates/template` (`cloud_init.rs`, `ignition.rs`). Talos uses NoCloud with its own
machine config; see [`TALOS.md`](TALOS.md).

## Delivery

| Method | Models | Delivered as |
|---|---|---|
| cloud-init | Linux cloud images, server appliances, Alpine, FreeBSD, OpenBSD | NoCloud ISO with `user-data`, `meta-data` and `network-config`, written to the node's lab directory |
| Ignition | Flatcar | Ignition JSON copied to the storage pool |
| cloudbase-init | Windows | NoCloud ISO with cloudbase-init `user-data` |

The generated files stay in the lab directory on the server, and the ISO is copied into the
storage pool, so everything below is readable there by `sherpa`, the `sherpa` group and QEMU
(see "Lab storage permissions" in [`SERVER.md`](SERVER.md)). None of it is for secrets: secrets
go through the client-side provisioners in [`PROVISIONERS.md`](PROVISIONERS.md), which the
server never sees.

Alpine has its own branch in `generate_cloud_init_ztp`; every other cloud-init model shares the
main branch. Both build `runcmd` in the same order: staged file moves, the environment loader,
`startup_scripts`, then `user_scripts`.

## Guest login

The login that owns guest-side work (`user_scripts`, the environment file) is the owner's personal
account when one is provisioned, otherwise `sherpa` (`guest_account::guest_username`).
`guest_account::supports_guest_account` lists the models that get a personal account: the Linux
cloud-init models, Alpine and Flatcar. The BSDs, Windows, containers, unikernels and network
devices use `sherpa` only.

Each account keeps its OS's native tools:

| OS | Shell | Admin rights | Where |
|---|---|---|---|
| Linux (cloud-init) | `/bin/bash` | cloud-init `sudo` rule, passwordless | `guest_account::cloud_init_user` |
| Alpine | `ash` (`/bin/sh`), `wheel` group | dedicated `konst::NODE_DOAS_CONF_DIR/<user>.conf`, `permit nopass` | `guest_account::alpine_cloud_init_user`, `doas_file` |
| Flatcar | `/bin/bash` | dedicated `/etc/sudoers.d/<user>` | `guest_account::ignition_user`, `sudoers_file` |
| FreeBSD, OpenBSD (`sherpa`) | `/bin/sh`, `wheel` group | cloud-init `sudo` rule | `cloud_init_user_settings` |

Nothing is installed at first boot, so accounts work without internet access. No password is
copied into the guest. Personal accounts are locked (`lock_passwd: true`), except on Alpine: its
sshd is built without PAM and refuses accounts whose shadow password starts with `!`, even for
key logins, so the Alpine account gets `*` (`konst::NODE_NO_PASSWORD_HASH`, with
`lock_passwd: false`), which matches no password but is not a lock.

`CloudInitUser` omits an empty `groups` list, because cloud-init's schema rejects it.

## Environment variables

Manifest `environment_variables` go to the guest login only, never system-wide. They are written
to `~/.sherpa_env` (`konst::NODE_USER_ENV_FILE`, mode 0600, owned by the login), and a line that
sources it (`user_env_loader`) is added to the login file bash reads: `~/.bash_profile`, otherwise
`~/.profile`. Non-login sessions, such as `ssh <node> <command>`, do not read it.

- cloud-init: the file is staged like a text file (`build_user_env_file`), and
  `user_env_loader_runcmd` appends the loader once the user exists.
- Flatcar: `build_ignition_user_env` writes the file and adds the loader with an Ignition `append`
  entry on `~/.bash_profile`, which Flatcar copies from the skeleton.
- Windows: values are set machine-wide (`build_windows_env_commands`).

## Files owned by guest users (cloud-init)

cloud-init's `write_files` runs before `users_groups`, so it cannot own a file by a user it is
about to create. `write_files` `defer` is not usable either: some images' packaged `cloud.cfg`
(openSUSE 15.6) never run `write_files_deferred`.

Instead, manifest `text_files` and the environment file are staged (`stage_cloud_init_files`).
`write_files` writes each as a root-only file under `konst::NODE_STAGING_DIR`, and one `runcmd`
line per file, run once the users exist:

1. looks up the user's home in `/etc/passwd` (`konst::NODE_PASSWD_FILE`, `passwd_home_lookup`),
   because FreeBSD homes are under `/usr/home`; a destination starting with `~/` is resolved
   against it, and a user missing from `passwd` skips the file;
2. creates missing parent directories, giving them to the user only when they are inside the
   user's home, so directories such as `/home` itself stay root-owned;
3. moves the file into place and sets its owner and mode.

The group defaults to the user's primary group (`id -gn`), because SUSE puts users in the shared
`users` group rather than a group of their own. Each file has its own command, so one bad owner
does not stop the others. The last command removes the staging directory. Values never appear in
`runcmd`.

Root-owned `write_files` entries (certificates, scripts, staged files) leave `owner` unset, so
cloud-init uses the distro default: `root:root`, or `root:wheel` on the BSDs, which have no `root`
group. A `root:root` owner there fails the whole `write_files` module.

## Scripts (cloud-init)

`build_cloud_init_scripts` writes `startup_scripts` to `konst::NODE_STARTUP_SCRIPTS_DIR` and
`user_scripts` to `konst::NODE_USER_SCRIPTS_DIR`, mode 0755. Startup scripts run as root; user
scripts are chowned to the guest login and its primary group, and run as it with `su -`, so they
get a login shell and the environment variables. Scripts are run directly, not through `bash`, so
each script's shebang picks its interpreter: Alpine and the BSDs ship no `bash`. A script without a
shebang runs with `/bin/sh`. Paths are shell-quoted. Flatcar does not run manifest scripts.

## Flatcar (Ignition)

`generate_ignition_ztp` writes text files as Ignition files with their own owner and mode, and adds
user-owned Ignition directories for each missing parent under the owner's `/home/<user>`
(`build_ignition_home_directories`); Ignition would otherwise create them as root, leaving the user
unable to write in, for example, their own `~/.config`.

Modes are written in code as octal digits (`644`) and converted to decimal on serialization
(`serialize_mode_as_decimal` in `crates/template/src/ignition.rs`), for files and directories
alike. Ignition reads modes as decimal numbers, so an unconverted `644` would be octal `1204`.

## Per-OS cloud-init adjustments

| OS | Problem | Adjustment |
|---|---|---|
| OpenBSD | cloud-init's OpenBSD renderer crashes on the IPv6 default route, leaving no network, and never writes IPv6 addresses | IPv4-only `network-config` (`cloud_init_ipv6_address`) |
| OpenBSD | cloud-init has no hosts template for OpenBSD, so `update_etc_hosts` fails | `manage_etc_hosts: false` (`cloud_init_manage_etc_hosts`); the name resolves through the lab DNS |
| Alpine | The ENI renderer leaves `dns-nameservers` to a `resolvconf` tool the image lacks, so `/etc/resolv.conf` is never written | `manage_resolv_conf` with `CloudInitResolvConf::lab_dns`, which runs in the init stage |
| FreeBSD, OpenBSD | No `root` group | Root-owned files omit `owner` |
| FreeBSD | Homes are under `/usr/home` | Homes are looked up in `/etc/passwd` at boot |
| SUSE, openSUSE | No per-user group; openSUSE 15.6 never runs deferred `write_files` | Primary group by default; staging instead of `defer` |
