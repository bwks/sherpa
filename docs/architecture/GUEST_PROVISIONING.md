# Guest Provisioning

How a VM's login, `environment_variables`, `text_files`, scripts and certificates reach the guest
at first boot: generated in `server::services::node_ops` and `guest_account`, rendered by
`crates/template`. Talos is in [`TALOS.md`](TALOS.md). The behaviour is in `tests/specs/`:
`provisioning.md`, `accounts-and-auth.md` (`owner_login`) and `ztp.md` (`device_accounts`).

## Delivery

| Method | Models | Delivered as |
|---|---|---|
| cloud-init | Linux cloud images, NixOS, appliances, Alpine, FreeBSD, OpenBSD | NoCloud ISO (`user-data`, `meta-data`, `network-config`) |
| Ignition | Flatcar | JSON in the storage pool, named by the domain's `<sysinfo type='fwcfg'>` entry, so libvirt labels it and adds it to the VM's AppArmor profile, which a raw `-fw_cfg` argument would not |
| cloudbase-init | Windows | NoCloud ISO with cloudbase-init `user-data` |

Each method carries only some of a manifest's files and scripts: cloud-init and cloudbase-init
take `text_files` and `startup_scripts`, and Ignition takes `text_files` and `systemd_units`.
`topology::validate::check_node_file_settings` refuses the others, and all three on any node whose
ZTP setting is another method or off, or that has its own `ztp.config`. The ZTP setting is the
node's or its image's (`effective_ztp`), so the client checks a node against its model's built-in
image at `sherpa validate`, `up` and `redeploy`, and the server checks it again against the image
it builds from, in `up`'s plan and before a redeploy tears the node down.

All of it is readable by `sherpa`, its group and QEMU in the storage pool (see [`SERVER.md`](SERVER.md#lab-and-database-storage-permissions)),
so none of it is for secrets: those go through the [provisioners](PROVISIONERS.md).

## Guest login

A node with a generated login has exactly one, the lab owner's, under their Sherpa username, with
their keys and their [device password](SERVER.md#device-passwords). There is no shared account and
no published password. `guest_account::provisions_owner` gives no login to a node with a custom ZTP
config or ZTP off, as the generated config it would go into is replaced or not made. Nodes that
cannot add an account at boot (Talos, application containers, unikernels) have none; FTDv's day-0
config sets its own `admin` password, so `admin` is its recorded login. Each account keeps its OS's
native tools (`sudo`, `doas`, `wheel`), and nothing is installed at boot, so nodes need no internet.

A login name or key a model cannot take is refused, or left out and reported, before anything is
created (`guest_account::node_account`, from `validate::validate_login_name` and
`guest_account::key_rules`, which hold each vendor's documented limits). A name is never shortened,
as the SSH config and what the person types would then disagree.

## Device password

`guest_account::device_password` writes the password in the form each device takes. A device gets
the plain password only when it takes nothing else, or does not verify a hash, and its config is
not served over the network; a config served over TFTP or HTTP (vEOS, Junos, SONiC) carries only
a SHA-crypt hash, as anything on the management network can fetch it. Hashes have no `rounds=`
field, which not every device reads. OpenBSD's crypt takes only bcrypt, so cloud-init hashes it on
the node. Aruba AOS-CX has no hashed form and serves its config over TFTP, so both its accounts are
locked with a password nobody keeps (`template::aruba_aos::LOCKED_PASSWORD`) and log in by key
only; an account with no password at all would take an empty one on its console and REST API.
A device password set later applies only to nodes built or redeployed after it.

## Files, environment and scripts (cloud-init)

cloud-init's `write_files` runs before its users exist, and its `defer` option is not run by every
image (openSUSE 15.6), so `stage_cloud_init_files` writes each user's file root-only under
`NODE_STAGING_DIR` and moves it into place from `runcmd` once the users exist, looking the home up
in `/etc/passwd` (FreeBSD homes are under `/usr/home`) and defaulting the group to the user's
primary group (SUSE shares `users`). Each file has its own command, so one bad owner does not stop
the others, and no value appears in `runcmd`. Root-owned entries leave `owner` unset, as the BSDs
have no `root` group and a `root:root` owner fails the whole module.

`runcmd` runs, in order: staged moves, the environment loader, `startup_scripts` as root. Scripts
that run as the owner are an ssh provisioner's (see [`PROVISIONERS.md`](PROVISIONERS.md)).
Environment variables go to the owner's login only (`~/.sherpa_env`, sourced from the login file
bash reads), never system-wide; Windows sets them machine-wide, each with a PowerShell command
passed as `-EncodedCommand` (base64 of its UTF-16LE bytes), so the `cmd` that runs `runcmd` never
parses a key or a value: a `"` cannot end an argument and `%NAME%` is not expanded. A Windows
startup script's file name does go into a `cmd` line, so one with `"` or `%` is refused at
validation, as a private file's path is. Scripts run through their shebang,
not `bash`, because Alpine and the BSDs ship none. Flatcar takes no `startup_scripts`; its
`systemd_units` are Ignition units. Ignition takes
file modes as decimal numbers, so `template::FileMode` holds the bits and serialises them that way.

## Windows (cloudbase-init)

OpenSSH on Windows reads one `administrators_authorized_keys` for every administrator, so any
administrator's key would log in as any other. Sherpa's `sshd_config` gives each account its own
keys file outside the profile, which Windows creates only at first logon. The images carry two more
administrators, the built-in `Administrator` and `Admin`, which cloudbase-init makes before the
user data runs and the `NoCloud` metadata cannot stop, so the user data runs
`WINDOWS_OWNER_ONLY_LOGIN_SCRIPT` to disable every enabled local account but the owner's, and only
once the owner's exists. cloudbase-init ignores `permissions`, so private files are narrowed with
`icacls` by SID (`build_windows_private_file_acls`).

## Per-OS adjustments

Each cloud-init quirk is handled in one place whose doc comment says why: in `node_ops`,
`cloud_init_ipv6_address`, `cloud_init_manage_etc_hosts`, `cloud_init_sudo_rule` and
`cloud_init_ssh_pwauth`; in `template`, `CloudInitResolvConf::lab_dns` and `CloudInitUser::groups`;
and `konst::NODE_NIXOS_SHELL`. The NixOS image's own part is its `configuration.nix` in
[bwks/nixos-image](https://github.com/bwks/nixos-image).
