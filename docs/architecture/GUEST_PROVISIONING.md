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
| cloud-init | Linux cloud images, NixOS, server appliances, Alpine, FreeBSD, OpenBSD | NoCloud ISO with `user-data`, `meta-data` and `network-config`, written to the node's lab directory |
| Ignition | Flatcar | Ignition JSON copied to the storage pool |
| cloudbase-init | Windows | NoCloud ISO with cloudbase-init `user-data` |

The generated files stay in the lab directory on the server; `user-data`, which can hold the node's key and, for Talos, its machine secrets, is written 0600. The ISO is built from them by
`iso::create_iso` (see [`ZTP_ISO.md`](ZTP_ISO.md)) and copied into the storage pool, so everything below is readable there by `sherpa`, the `sherpa` group and QEMU
(see "Lab storage permissions" in [`SERVER.md`](SERVER.md)). None of it is for secrets: secrets
go through the client-side provisioners in [`PROVISIONERS.md`](PROVISIONERS.md), which the
server never sees.

Alpine has its own branch in `generate_cloud_init_ztp`; every other cloud-init model shares the
main branch. NixOS shares it, with the adjustments below; the rest of what NixOS needs (the
`cloud-init` command, per-user groups, `eth` interface names, cloud-init without warnings) is
set in its image, `configuration.nix` in [bwks/nixos-image](https://github.com/bwks/nixos-image). Both build `runcmd` in the same order: staged file moves, the environment loader,
`startup_scripts`, then `user_scripts`.

## Guest login

The login that owns guest-side work (`user_scripts`, the environment file) is the owner's personal
account when one is provisioned, otherwise `sherpa` (`guest_account::guest_username`).
`guest_account::supports_guest_account` lists the models that get a personal account: the Linux
cloud-init models, `generic_vm`, Alpine, Flatcar, FreeBSD, OpenBSD, Windows, and the network
devices in [Network devices](#network-devices). Cisco FTDv, Talos, application containers and
unikernels have no way to add an account at boot and use `sherpa` or their own default login only.
A node with a custom ZTP config gets no personal account (`guest_account::provisions_owner`),
because the generated config it would be written into is replaced.

`generic_vm` is treated as a Linux cloud-init guest, like the main cloud-init branch.

Each account keeps its OS's native tools:

| OS | Shell | Admin rights | Where |
|---|---|---|---|
| Linux (cloud-init) | `/bin/bash` | cloud-init `sudo` rule, passwordless | `guest_account::cloud_init_user` |
| Alpine | `ash` (`/bin/sh`), `wheel` group | dedicated `konst::NODE_DOAS_CONF_DIR/<user>.conf`, `permit nopass` | `guest_account::alpine_cloud_init_user`, `doas_file` |
| Flatcar | `/bin/bash` | dedicated `/etc/sudoers.d/<user>` | `guest_account::ignition_user`, `sudoers_file` |
| NixOS | `konst::NODE_NIXOS_SHELL` (`/run/current-system/sw/bin/bash`), `wheel` group | `wheel`, passwordless in the image. No `sudo` rule (`node_ops::cloud_init_sudo_rule`) | `guest_account::nixos_cloud_init_user`; `sherpa`: `cloud_init_user_settings` |
| FreeBSD | `/bin/sh`, `wheel` group | cloud-init `sudo` rule, passwordless (neither `sudo` nor `doas` is in base; the image ships `sudo`) | `guest_account::bsd_cloud_init_user`; `sherpa`: `cloud_init_user_settings` |
| OpenBSD | `/bin/sh`, `wheel` group | `doas`, in base: `konst::NODE_DOAS_CONF_FILE` with the stock `/etc/examples/doas.conf` rule plus `nopass`, `permit nopass keepenv :wheel`. No `sudo` rule (`node_ops::cloud_init_sudo_rule`) | `guest_account::bsd_cloud_init_user`, `node_ops::cloud_init_doas_file` |

`CloudInitUser.groups` is written as a comma-separated string: cloud-init's BSD `add_user`
passes only string values to `pw`/`useradd`, so a list left BSD logins out of `wheel`.

Nothing is installed at first boot, so accounts work without internet access. No password is
copied into the guest. Personal accounts are locked (`lock_passwd: true`), except on Alpine and
the BSDs: Alpine's sshd is built without PAM and refuses accounts whose shadow password starts
with `!`, and OpenSSH on FreeBSD refuses `*LOCKED*` accounts, both even for key logins. Those
accounts get `*` (`konst::NODE_NO_PASSWORD_HASH`, with `lock_passwd: false`), which matches no
password but is not a lock. BSD accounts also leave the home to the OS, which is under
`/usr/home` on FreeBSD.

`CloudInitUser` omits an empty `groups` list, because cloud-init's schema rejects it.

## Owner keys per model

Many devices take only some key types, or a limited number of keys per user.
`guest_account::key_rules` holds both for each model, from the vendors' documentation;
`guest_account::node_account` applies them when `sherpa up` and `sherpa redeploy` check the lab,
before anything is created:

- Keys of a type the model does not take, and keys past its limit, are left out, and the CLI is
  told which (`Node '<node>' does not take these SSH keys of '<owner>'`).
- If none of the owner's keys fits, the operation stops with the types the model takes and the
  `ssh-keygen` command that makes one.

| Models | Key types | Keys |
|---|---|---|
| Linux, BSD, Flatcar, Windows, `generic_vm`, FRR, SONiC | every registered key | all |
| Cisco IOSv, IOSvL2, CSR1000v, Catalyst 8000v and 9000v | RSA | 2 |
| Cisco IOS XR | RSA | 4 (`keystring`, then `second`, `third`, `fourth`) |
| Cisco ASAv | Ed25519, ECDSA, RSA | 1 |
| Cisco NX-OS | ECDSA, RSA (NX-OS lists Ed25519 as unsupported) | 1 |
| Cisco ISE | RSA | 1 |
| Palo Alto PAN-OS | Ed25519, RSA | 1 |
| MikroTik RouterOS | Ed25519, RSA | all |
| Arista vEOS and cEOS | Ed25519, ECDSA, RSA | 2 (a key and a `secondary` key) |
| Aruba AOS-CX, Nokia SR Linux | Ed25519, ECDSA, RSA | 32 |
| Cumulus, Junos (vMX, vQFX, vSRX, vEvolved) | Ed25519, ECDSA, RSA | all |

## Network devices

The owner's account is rendered by each device's ZTP template next to `sherpa`, from
`shared::data::OwnerUser`. `guest_account::owner_user` writes each key the way the device's
config takes it (`guest_account::KeyFormat`): the key type and base64 data, a Cisco MD5 or
SHA-256 hash, a Junos statement with the whole key line, or the PAN-OS base64 of the key line.

Accounts have full administrator rights and log in by key. Devices that need a password, or that
would accept an empty one, get a random 20-character password nobody is told
(`OwnerUser.secret`; cloudbase-init takes at most 20 characters, and Cisco IOS 25). It is in
the rendered config in
the lab directory, which only `sherpa` and its group can read. Never use a `nopassword` form:
on IOS and ASA it accepts any password.

| Model | Account | Admin rights | Password |
|---|---|---|---|
| IOSv, IOSvL2, IOS XE | `username` and `ip ssh pubkey-chain` `key-hash` | privilege 15 | random `secret` |
| IOS XR | `username`, `ssh server username` `keystring` | `root-lr`, `netadmin` | random `secret 0` |
| ASAv | `username` and `attributes` with `hashed` keys | privilege 15, `service-type admin` | none (key-only is supported) |
| NX-OS | `username`, `sshkey` | `network-admin` | random |
| Arista EOS | `username`, `ssh-key`, `ssh-key secondary` | privilege 15 | random `secret` |
| Aruba AOS-CX | `user`, `authorized-key` | `administrators` | random (AOS-CX needs one before a key) |
| Junos | `system login user` | `super-user` | none |
| Cumulus | NVUE `system aaa user`, one named `authorized-key` per key | `system-admin` | random |
| SR Linux | AAA `user` with an `ssh-key` list | `srl-admin` (superuser) | random |
| PAN-OS | bootstrap `mgt-config` user with a `public-key` | superuser | a random, well-formed SHA-256 crypt hash (`guest_account::unusable_sha256_crypt_hash`), which no known password matches |
| MikroTik | `/user add`, `/user ssh-keys import` of one file per key on the config disk (`sata1`, `konst::MIKROTIK_CONFIG_DISK`) | `full` | random; RouterOS's default empty password must never be left |
| SONiC | `useradd`, `authorized_keys`, `/etc/sudoers.d` | `sudo`, `admin`, `docker` groups, passwordless sudo | none, locked |
| FRR | `adduser`, `authorized_keys`, `/etc/sudoers.d` | `frr`, `frrvty` groups, passwordless sudo | `*`, which Alpine's sshd without PAM accepts for key logins where it refuses a locked account |

The `sherpa` key on MikroTik is imported by the image itself: a scheduler imports
`sata1/sherpa_ssh_key.pub` for `sherpa`. The file is the lab's public key (see
[`SERVER.md`](SERVER.md#lab-ssh-keys)), written into the node's folder and copied to the disk.

### Cisco ISE

ISE's ZTP config has a single administrator, `sherpa`, and one `public_key`. With a personal
account it takes the owner's first RSA key instead of the lab's `sherpa` key, so the owner
logs in as `sherpa` with their own key, and the `ssh_usernames` entry for the node is `sherpa`.
Sherpa's own key no longer works on that node.

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

Nodes with a lab certificate also get the lab CA in the system trust store through cloud-init's
`ca_certs` module (`cloud_init_ca_certs`), so their clients trust lab services; see
[`LAB_CA.md`](LAB_CA.md).

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

Ignition files and directories take their mode as a `template::FileMode`, which holds the
permission bits and serializes them as the decimal number Ignition reads (`0o644` is `420`).
Fixed modes are written as Rust octal literals, `FileMode::new(0o644)`. A manifest's text file
mode, written as octal digits (`644`), is converted by `shared::validators::mode_from_octal_digits`,
which refuses digits other than 0 to 7 and values over 777. The same function converts container
secret file modes and backs every mode validator; `topology::validate::check_text_file_modes`
refuses bad text file modes in the client and in `up` before they get that far.

## Windows (cloudbase-init)

OpenSSH on Windows reads one shared `administrators_authorized_keys` file for every member of
Administrators, so any administrator key logs in as any administrator. Sherpa writes its own
`sshd_config` (`template::WindowsSshdConfigTemplate`, `konst::WINDOWS_SSHD_CONFIG`) with
`AuthorizedKeysFile __PROGRAMDATA__/ssh/authorized_keys/%u`: each account, `sherpa` and the
owner, has its own keys file under `konst::WINDOWS_AUTHORIZED_KEYS_DIR`, named after it and
private to Administrators and SYSTEM (mode `0600`, below). The file is outside the user's
profile, which Windows only creates at first logon. The owner is a local administrator
(`guest_account::cloudbase_user`); cloudbase-init needs a password, so it gets a random one
nobody is told.

cloudbase-init's `write_files` ignores `permissions`: a file inherits its folder's ACL, and
under `C:\` that lets `BUILTIN\Users` read it. `build_windows_private_file_acls` turns each
written file whose mode gives others no access (the node key at `0600`, a `text_files` entry at
`600` or `640`) into an `icacls` command in `runcmd` that removes inherited access and grants only
Administrators and SYSTEM. They are named by SID (`konst::WINDOWS_ADMINISTRATORS_SID`,
`WINDOWS_SYSTEM_SID`), so the command does not depend on the Windows display language, and no
other account is granted: an unknown name would make `icacls` fail and leave the file open.
cloudbase-init runs `write_files` before `runcmd`, and these commands come before the startup
scripts. `runcmd` lines run through cmd, so a path containing `"` or `%` is refused rather
than quoted. `administrators_authorized_keys` gets its own `icacls` command, which sshd needs.

## Per-OS cloud-init adjustments

| OS | Problem | Adjustment |
|---|---|---|
| OpenBSD | cloud-init's OpenBSD renderer crashes on the IPv6 default route, leaving no network, and never writes IPv6 addresses | IPv4-only `network-config` (`cloud_init_ipv6_address`) |
| OpenBSD | cloud-init has no hosts template for OpenBSD, so `update_etc_hosts` fails | `manage_etc_hosts: false` (`cloud_init_manage_etc_hosts`); the name resolves through the lab DNS |
| Alpine | The ENI renderer leaves `dns-nameservers` to a `resolvconf` tool the image lacks, so `/etc/resolv.conf` is never written | `manage_resolv_conf` with `CloudInitResolvConf::lab_dns`, which runs in the init stage |
| FreeBSD, OpenBSD | No `root` group | Root-owned files omit `owner` |
| FreeBSD | Homes are under `/usr/home` | Homes are looked up in `/etc/passwd` at boot |
| NixOS | No `/bin/bash`, so sshd refuses accounts given it | `konst::NODE_NIXOS_SHELL` for every account |
| NixOS | The generated `/etc/sudoers` does not read `/etc/sudoers.d`, so cloud-init `sudo` rules are ignored | Accounts join `wheel`, which the image gives passwordless sudo; no `sudo` rule |
| NixOS | `sshd_config` is generated and read-only, so cloud-init's set-passwords module fails on `ssh_pwauth` and reports an error | `ssh_pwauth` omitted (`cloud_init_ssh_pwauth`); the image allows password logins itself |
| SUSE, openSUSE | No per-user group; openSUSE 15.6 never runs deferred `write_files` | Primary group by default; staging instead of `defer` |
