# Zero touch provisioning (ZTP)

How a node gets its first configuration: the one setting, where it is checked, who reads it, the
boot server, containers with ZTP off, and blank disks. The behaviour is in
[`tests/specs/ztp.md`](../../tests/specs/ztp.md). How a config reaches a VM guest is in
[`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md), the ISO it is written to in
[`ZTP_ISO.md`](ZTP_ISO.md), and the modes of the rendered files, and why, on the mode constants
in `server::konst` and in `config_privacy` in the spec.

## The setting

One enum, `shared::data::Ztp`: `Disabled`, or the one method Sherpa provisions the node by. There
is no separate "enabled" flag, so "off with a method" cannot be stored. The enum's word is the
same everywhere it appears: serde, `Display`, the `node_image.ztp` column, the API, the manifest's
`ztp.method`, the web form and the CLI table.

`ModelKind::ztp_methods` lists the methods each model and kind take. It is a `match` with no
wildcard arm, so a new model does not compile until it has an entry. `Disabled` is allowed for
every pair and is never listed.

A manifest node may set `ztp.method`; `shared::validators::effective_ztp` resolves it against the
image's. Every reader takes that result, and none reads the image's setting directly.

## Validation

`shared::validators::check_ztp` is the one rule: `Disabled` for every model, a method only when
the model lists it. It runs at every layer that can introduce a setting, so no path reaches a
reader with one the model does not take:

| Caller | When |
|---|---|
| `db::upsert_node_image`, `db::update_node_image` | every image write: import, scan, download, pull, set-default, the web form |
| `server::validate::validate_node_image_update` | the web form, before the image lock, for its error message |
| `topology::validate::check_node_ztp` | `sherpa validate` |
| `server::validate::validate_node_ztp` | `up` while planning, before anything is created; redeploy before the node is torn down |

`topology::validate::check_node_ztp_config` checks the node's `ztp.config` against the setting
it runs with. `sherpa validate` runs it with the model's built-in setting and
`validate_node_ztp` with the node's image, so all three refuse with one message. The rules are
`custom_config` and `ztp_setting_014` in the spec.

## Readers

Each reader is one `match` on `Ztp` with `Disabled` as a case in it, so adding a method forces
every reader to decide what it means:

- VM media, `node_ops::generate_vm_ztp_media`;
- container setup, `node_ops::container_model_setup`;
- DHCP boot options, `DnsmasqTemplate::boot_options`, the only lines the dnsmasq template loops over;
- the owner account, `guest_account::supports_guest_account`.

The VM and container generators each keep one arm for the values the validator excludes
(`volume` on a VM, the other methods on a container); it fails as "not validated".

## The boot server

Each lab has one boot container, `sherpa-router-<lab-id>`, on its management network. It is the
lab's NAT router and runs dnsmasq (DHCP, DNS, TFTP) and `webdir`, which serves everything under
`/opt/ztp`, listings included. So `boot_server::boot_container_spec` mounts there only the lab's
`ztp/configs/`, which holds only the files of nodes that boot by HTTP. The TFTP root
(`ztp/tftp/`), the dnsmasq config and the leases file sit outside it, so a node on the management
network can fetch the TFTP configs only by name, through TFTP, and nothing else. What is served
is open to the whole management network by design, so it carries password hashes only.

dnsmasq answers the names of the lab's nodes and forwards other names to the lab's gateway,
libvirt's dnsmasq, which asks the server's resolver. A name without a domain that is not a node's
is answered as not found and never forwarded (`domain-needed`): on a server with
systemd-resolved, which does not look up such a name, the gateway never answers it, and a node's
lookup would wait out its resolver's timeouts.

## Redeploy and the boot server

dnsmasq reads its config only when it starts (`SIGHUP` rereads host files, not the config). A VM
redeploy can change the node's boot options, so after the node's ZTP is generated and before its
domain is created it calls `boot_server::refresh_boot_server`: it renders every node's records as
`up` does and compares the result with the file on disk. Addresses and MACs survive a redeploy,
so it differs only when the node moved onto or off TFTP or HTTP; only then is the file written
and the boot container recreated, pausing the lab's DHCP, DNS, TFTP, HTTP and NAT. The empty
leases are harmless, as every address is reserved by MAC. A failed recreate fails the redeploy.

## Containers with ZTP off

`container_model_setup` applies the model's start settings for `Disabled`, and adds the config
files, mounts and logins for `Volume`. Off means the image as shipped: no generated config, no
Sherpa config mount and no owner login, because the login is written into the generated config.
The management address is still reserved and Docker still gives it to `eth0`, but most network
OSes only use it when the generated config says so. A container is ready when it is running, so
readiness does not depend on the setting.

`add_container_volumes` drops the manifest's `volumes` with `volume`, and keeps them with ZTP
off: they are how a user mounts a config of their own.

### SONiC container

The `sonic_linux` container runs upstream's `docker-sonic-vs`, whose `start.sh` reads the
container's `ethN` interfaces once and maps them to ports. Sherpa attaches the data interfaces
after the container starts, so the image cannot start as shipped. Its entrypoint, `supervisord`,
is given Sherpa's config, whose only program is Sherpa's start script: it waits for the data
interfaces, then hands over to the image's `supervisord`. These start files are mounted with ZTP
off too, so the wait always applies; with ZTP off the script makes no account and loads no
config, and still starts `sshd` last, as the readiness signal. Nothing is mounted at
`/etc/sonic/config_db.json`, because `start.sh` renames a file over that path; the script merges
Sherpa's config on the first start instead (see the templates in `template::sonic_linux`).

## Blank disks

The `disk` and `usb` methods, and Flatcar's data disk, start from a blank disk template in
`konst::SHERPA_BLANK_DISK_DIR`. `node_ops` copies the template into the node's folder and writes
the config onto the copy with `mcopy` (FAT) or `e2cp` (ext4), so the template is never changed. A
missing template fails the node's ZTP.

`sherpad init` makes the folder but no disk: `ensure_blank_disks` in the install and update
scripts makes them, the update on every run, so a server gets any disk added later. It never
changes a file that exists, since node disks may have been copied from it, and builds each disk
as `sherpa`, never as root, under a temporary name and links it into place, so a disk is either
whole or absent and a link put in the folder, which the `sherpa` group can write, gets nothing
of root's. Each disk is `0400`: only sherpad, as `sherpa`, its owner, opens a template, to copy
it, and QEMU only ever opens the copies. `sherpad doctor --images` sets the templates back to
`0400` (`doctor::fix_image_permissions`), working through open folder descriptors with
`O_NOFOLLOW` and `fchmodat(AT_SYMLINK_NOFOLLOW)`, so a link a group member puts in the images
folder is skipped and named, and root changes nothing through it. The section
between `# BEGIN blank disks` and `# END blank disks` is copied in both scripts, because each is
fetched alone with `curl` and has nothing beside it to source; the BATS tests run against each
copy.
