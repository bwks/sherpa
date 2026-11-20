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
| Ignition | Flatcar | Ignition JSON copied to the storage pool, named in the domain's `<sysinfo type='fwcfg'>` entry `opt/org.flatcar-linux/config` |
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

A node with a generated login has one: the lab owner's, named after their Sherpa username, with
their registered SSH keys the model accepts and their [device password](SERVER.md#device-passwords). There
is no shared `sherpa` account and no published password on the nodes Sherpa builds. The
application nodes that have an administrator of their own, Keycloak and SurrealDB, get one per
node with a generated password, kept on the server and handed to the owner (see
[`LAB_CA.md`](LAB_CA.md#keycloak)).
`guest_account::from_user` makes the owner's `GuestAccount` from the user record and the opened
device password. It refuses an owner whose name cannot be a login (a legacy account named `root`,
`sherpa` or `admin`, say) with `User '<name>' cannot start labs`; before 0.7.0 such an owner fell
back to `sherpa`.

The login that owns guest-side work (`user_scripts`, the environment file) is that account.
`guest_account::supports_guest_account` lists the models that get it: the Linux cloud-init models,
the `generic` VM, Alpine, Flatcar, FreeBSD, OpenBSD, Windows, and the network devices in
[Network devices](#network-devices). Talos, application containers and unikernels have no way to
add an account at boot and have no generated login. Cisco FTDv takes no account either, but its
day-0 config sets the password of its own `admin` (`guest_account::fixed_login`), so `admin` is
its login and no key is installed. A node with a custom ZTP config, or ZTP off, gets no account
(`guest_account::provisions_owner`), because the generated config it would be written into is
replaced or not made.

A `generic` VM is treated as a Linux cloud-init guest, like the main cloud-init branch.

Each account keeps its OS's native tools:

| OS | Shell | Admin rights | Where |
|---|---|---|---|
| Linux (cloud-init) | `/bin/bash` | cloud-init `sudo` rule, passwordless | `guest_account::cloud_init_user` |
| Alpine | `ash` (`/bin/sh`), `wheel` group | dedicated `guest_account::NODE_DOAS_CONF_DIR/<user>.conf`, `permit nopass` | `guest_account::alpine_cloud_init_user`, `doas_file` |
| Flatcar | `/bin/bash`, `wheel` and `docker` groups | dedicated `/etc/sudoers.d/<user>`, passwordless | `guest_account::ignition_user`, `sudoers_file` |
| NixOS | `server::konst::NODE_NIXOS_SHELL` (`/run/current-system/sw/bin/bash`), `wheel` group | `wheel`, passwordless in the image. No `sudo` rule (`node_ops::cloud_init_sudo_rule`) | `guest_account::nixos_cloud_init_user` |
| FreeBSD | `/bin/sh`, `wheel` group | cloud-init `sudo` rule, passwordless (neither `sudo` nor `doas` is in base; the image ships `sudo`) | `guest_account::bsd_cloud_init_user` |
| OpenBSD | `/bin/sh`, `wheel` group | `doas`, in base: `node_ops::NODE_DOAS_CONF_FILE` with the stock `/etc/examples/doas.conf` rule plus `nopass`, `permit nopass keepenv :wheel`. No `sudo` rule (`node_ops::cloud_init_sudo_rule`) | `guest_account::bsd_cloud_init_user`, `node_ops::cloud_init_doas_file` |

`CloudInitUser.groups` is written as a comma-separated string: cloud-init's BSD `add_user`
passes only string values to `pw`/`useradd`, so a list left BSD logins out of `wheel`.

Nothing is installed at first boot, so accounts work without internet access. Accounts are not
locked (`lock_passwd: false`): the device password is their password, so the console and
password SSH logins work as well as keys. The owner keeps passwordless admin rights, so the
password is for logging in, not for `sudo`. BSD accounts leave the home to the OS, which is under
`/usr/home` on FreeBSD.

`CloudInitUser` omits an empty `groups` list, because cloud-init's schema rejects it.

## Device password

The password of the owner's login is the owner's device password, opened from the database. `up`
(`set_up`, `services/up/lab.rs`) and `redeploy` (`redeploy/node.rs`) check the owner in this
order, before they create or tear down anything: an SSH key (`guest_account::require_ssh_key`),
then a device password (`guest_account::require_device_password`, which calls
`db::open_device_password` and fails with the message that says how to set one), then
`guest_account::from_user`. `resume`, `down`, `destroy` and `inspect` build no configuration and
do not need it. A new device password applies to nodes created or redeployed afterwards.

`guest_account::device_password` writes it in the form a device takes (`PasswordForm`):

- `Plain`: the password itself. For devices that take nothing else, or whose hashed form is not
  verified on them, and for configs that are not served over TFTP or HTTP: Cisco IOS family,
  IOS XR, NX-OS, ASAv, ISE, FTDv, SR Linux (hashes it on commit), MikroTik, Cumulus,
  Windows (cloudbase-init needs it) and OpenBSD.
- `Hashed`: a salted SHA-crypt hash from `template::crypt_hash` (`crates/template/src/crypt.rs`,
  the `sha-crypt` crate). `$6$` (SHA-512) for cloud-init `hashed_passwd` on Linux, NixOS, Alpine
  and FreeBSD, Flatcar's Ignition `passwordHash`, Arista EOS `secret sha512`, Junos
  `encrypted-password`, and FRR and SONiC through `chpasswd -e`. `$5$` (SHA-256) for PAN-OS
  `phash`. The salt is 16 random characters and 5000 rounds, and the hash has no `rounds=` field,
  which not every device reads. Each render has its own salt.

OpenBSD's crypt(3) takes only bcrypt, so cloud-init gets the plain password there
(`plain_text_passwd`) and hashes it on the node.

Configs served over TFTP or HTTP (vEOS, Junos, SONiC) carry only a hash, since anything on the
management network can fetch them. Fixed vendor accounts take the device password too:
`admin` on NX-OS, IOS XR, PAN-OS, SR Linux (`admin-user`) and RouterOS, `root` on Junos
(`root-authentication`), and the ASA `enable password`.

Aruba AOS-CX takes no device password. It has no hashed form, only `plaintext` and a
`ciphertext` encrypted under a key every switch shares, which Sherpa cannot make, and its config
is served over TFTP. So the owner logs in by key only (#676):

- the owner and `admin` get `password ciphertext` of one random password nobody keeps,
  `LOCKED_PASSWORD` in `crates/template/src/aruba_aos.rs`, made once on a 10.16 switch; how, and
  what it reveals, is in its doc comment. It loads on a fresh 10.16 switch;
- `no ssh password-authentication`, so SSH offers only `publickey`.

An AOS-CX account with no password at all takes an empty one on the serial console and on the
HTTPS REST API, which `ssh password-authentication` does not cover, so the owner is locked like
`admin` rather than left without one. `admin` cannot be deleted (`User admin could not be
deleted`). Neither account has a password login, the console included. AOS-CX nodes still need
the owner's keys; `up` and `redeploy` still require a device password for every lab, as for any
other model.

## Login names

A model may not take every Sherpa username. `server::validate::validate_login_name`
(`validate/login_name.rs`) holds each model's limit, from the vendor's documentation:

| Models | Limit |
|---|---|
| Windows Server, `devbox_windows` | at most 20 characters |
| Cisco NX-OS (`cisco_nexus9300`) | at most 28 characters, starting with a letter or digit |
| Arista vEOS and cEOS | starting with a letter or digit |
| PAN-OS, OpenBSD | at most 31 characters |
| every other model | any Sherpa username |

A name a model cannot take is refused, never shortened or changed, since a changed name would
make the SSH config and what the person types disagree. `guest_account::node_account` applies it
with the key rules, so it runs where they do: `up` planning (`up/plan.rs`, `node_logins`) and
`redeploy` (`redeploy/context.rs`, `node_login`), before anything is created or torn down.

## Owner keys per model

Many devices take only some key types, or a limited number of keys per user.
`guest_account::key_rules` holds both for each model, from the vendors' documentation;
`guest_account::node_account` applies them, with the login name rule, when `sherpa up` and
`sherpa redeploy` check the lab, before anything is created:

- Keys of a type the model does not take, and keys past its limit, are left out, and the CLI is
  told which (`Node '<node>' does not take these SSH keys of '<owner>'`).
- If none of the owner's keys fits, the operation stops with the types the model takes and the
  `ssh-keygen` command that makes one.

| Models | Key types | Keys |
|---|---|---|
| Linux, BSD, Flatcar, Windows, the `generic` VM, FRR, SONiC | every registered key | all |
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

The owner's account is rendered by each device's ZTP template, from `shared::data::OwnerUser`:
the name, the device password in the form above, and the keys. `guest_account::owner_user` writes
each key the way the device's config takes it (`guest_account::KeyFormat`): the key type and
base64 data, a Cisco MD5 or SHA-256 hash, a Junos statement with the whole key line, or the
PAN-OS base64 of the key line.

Accounts have full administrator rights and log in by key or by the device password, except on
AOS-CX, by key only. Never use a
`nopassword` form: on IOS and ASA it accepts any password.

| Model | Account | Admin rights | Password |
|---|---|---|---|
| IOSv, IOSvL2, IOS XE | `username` and `ip ssh pubkey-chain` `key-hash` | privilege 15 | `secret`, plain |
| IOS XR | `username`, `ssh server username` `keystring`; `admin` | `root-lr`, `netadmin` | `secret 0`, plain |
| ASAv | `username` and `attributes` with `hashed` keys; `enable password` | privilege 15, `service-type admin` | `password`, plain |
| NX-OS | `username`, `sshkey`; `admin` | `network-admin` | `password 0`, plain |
| Arista EOS | `username`, `ssh-key`, `ssh-key secondary` | privilege 15 | `secret sha512`, `$6$` |
| Aruba AOS-CX | `user`, `authorized-key`; `admin` | `administrators` | none: both locked with `password ciphertext` of a random password, SSH password login off |
| Junos | `system login user`; `root-authentication` | `super-user` | `$6$` `encrypted-password` |
| Cumulus | NVUE `system aaa user`, one named `authorized-key` per key | `system-admin` | plain |
| SR Linux | AAA `user` with an `ssh-key` list; `admin-user` | `srl-admin` (superuser) | plain; SR Linux hashes it on commit |
| PAN-OS | bootstrap `mgt-config` user with a `public-key`; `admin` | superuser | `phash`, `$5$` |
| MikroTik | `/user add`, `/user ssh-keys import` of one file per key on the config disk (`sata1`, `konst::MIKROTIK_CONFIG_DISK`); `/user set 0` for `admin` | `full` | plain |
| SONiC VM | `useradd`, `authorized_keys`, `/etc/sudoers.d` | `sudo`, `admin`, `docker` groups, passwordless sudo | `$6$` through `chpasswd -e` |
| SONiC container | `useradd` in its start script on the first start, `authorized_keys`, `/etc/sudoers.d/90-sherpa-owner` when the image has `sudo` | `frrvty` group, passwordless sudo; none without `sudo` in the image | `$6$` through `chpasswd -e` |
| FRR | `adduser`, `authorized_keys`, `/etc/sudoers.d` | `frr`, `frrvty` groups, passwordless sudo | `$6$` through `chpasswd -e` |

The MikroTik config disk carries the rendered `config.rsc` and the owner's key files only. Up to
0.6.5 it also carried the lab key's public half, `sherpa_ssh_key.pub`; Sherpa has no lab key
from 0.7.0 (see [`SERVER.md`](SERVER.md#no-lab-ssh-key)).

The seeded CHR image (7.20.8) has two schedulers, each run every minute:

- `sherpa-ztp` imports `sata1/config.rsc` if it exists, and removes itself once an import gets to
  the end. An import stops at its first error, so a run that comes too early, such as one that
  finds no `ether1` yet, is retried a minute later. RouterOS deletes a key file once it imports
  it, so each key import in the template is guarded on its file still being there
  (`:if ([/file find name=…] != "")`): a retry skips the keys the first run imported instead of
  stopping on them and never reaching the addresses.
- `sherpa-ssh-key` imports `sata1/sherpa_ssh_key.pub` for `sherpa` if the file exists, and then
  removes itself. With no such file it does nothing, every minute, and logs nothing: it fails
  harmlessly. It is dead in the image and can be dropped from the image build.

### Cisco ISE

ISE's ZTP config has a single administrator and one `public_key`. It is the owner: their name,
the device password and their first RSA key. The `ssh_usernames` entry for the node is the owner's
name.

### Cisco FTDv

FTDv takes no account at boot. Its day-0 JSON (`AdminPassword`) sets the password of its `admin`,
to the device password. `admin` is the node's recorded login, and no SSH key is installed.

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
`write_files` writes each as a root-only file under `node_ops::NODE_STAGING_DIR`, and one `runcmd`
line per file, run once the users exist:

1. looks up the user's home in `/etc/passwd` (`node_ops::NODE_PASSWD_FILE`, `passwd_home_lookup`),
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

`build_cloud_init_scripts` writes `startup_scripts` to `node_ops::NODE_STARTUP_SCRIPTS_DIR` and
`user_scripts` to `node_ops::NODE_USER_SCRIPTS_DIR`, mode 0755. Startup scripts run as root; user
scripts are chowned to the guest login and its primary group, and run as it with `su -`, so they
get a login shell and the environment variables. Scripts are run directly, not through `bash`, so
each script's shebang picks its interpreter: Alpine and the BSDs ship no `bash`. A script without a
shebang runs with `/bin/sh`. Paths are shell-quoted. Flatcar does not run manifest scripts.

## Flatcar (Ignition)

The owner's Ignition user has exactly the keys on their account, as on every other model, `wheel`
and `docker`, and a `passwordHash` of the device password (`guest_account::ignition_user`).

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
`sshd_config` (`template::WindowsSshdConfigTemplate`, `node_ops::WINDOWS_SSHD_CONFIG`) with
`AuthorizedKeysFile __PROGRAMDATA__/ssh/authorized_keys/%u`: each account has its own
keys file under `node_ops::WINDOWS_AUTHORIZED_KEYS_DIR`, named after it and
private to Administrators and SYSTEM (mode `0600`, below). The file is outside the user's
profile, which Windows only creates at first logon. The config also sets `VersionAddendum
sherpa-ready`, which readiness waits for: the image's `sshd` answers long before cloudbase-init
has made the owner's account and keys file and restarted it with this config (see "Readiness" in
[`SERVER.md`](SERVER.md)). The owner is a local administrator
(`guest_account::cloudbase_user`); cloudbase-init needs a password, so it gets the device
password as plain text.

The owner is the node's only enabled login. The images come with two more administrators:

- the built-in `Administrator` (the `-500` SID), which the Windows Server image's build unattend
  gives a build password and auto-logon, and which the Windows 11 devbox image disables at build;
- `Admin`, made by cloudbase-init's `CreateUserPlugin` from the image's `cloudbase-init.conf`
  (`username=Admin`) with a random password, before the user data runs. Its name comes from
  `service.get_admin_username() or CONF.username`, and the `NoCloud` metadata service has no
  admin username, so the user data cannot rename it or stop it being made.

So the user data writes `node_ops::WINDOWS_OWNER_ONLY_LOGIN_SCRIPT`
(`template::WindowsOwnerOnlyLoginTemplate`) and runs it from `runcmd`, after `sshd` restarts and
before the node's startup scripts: it disables every enabled local account but the owner's,
and nothing at all unless the owner's account exists and is enabled. cloudbase-init runs as
`LocalSystem`, so no service depends on the disabled accounts. Accounts a startup script makes
are left alone. A recovery login is the owner's, with the device password, on the console.

cloudbase-init's `write_files` ignores `permissions`: a file inherits its folder's ACL, and
under `C:\` that lets `BUILTIN\Users` read it. `build_windows_private_file_acls` turns each
written file whose mode gives others no access (the node key at `0600`, a `text_files` entry at
`600` or `640`) into an `icacls` command in `runcmd` that removes inherited access and grants only
Administrators and SYSTEM. They are named by SID (`node_ops::WINDOWS_ADMINISTRATORS_SID`,
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
| NixOS | No `/bin/bash`, so sshd refuses accounts given it | `server::konst::NODE_NIXOS_SHELL` for every account |
| NixOS | The generated `/etc/sudoers` does not read `/etc/sudoers.d`, so cloud-init `sudo` rules are ignored | Accounts join `wheel`, which the image gives passwordless sudo; no `sudo` rule |
| NixOS | `sshd_config` is generated and read-only, so cloud-init's set-passwords module fails on `ssh_pwauth` and reports an error | `ssh_pwauth` omitted (`cloud_init_ssh_pwauth`); the image allows password logins itself |
| SUSE, openSUSE | No per-user group; openSUSE 15.6 never runs deferred `write_files` | Primary group by default; staging instead of `defer` |
