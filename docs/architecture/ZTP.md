# Zero touch provisioning (ZTP)

How a node gets its first configuration: the one setting that decides it, which methods each
model takes, where that is checked, who reads it, the blank disk templates the disk and USB media
start from, and what a container gets with ZTP off. How
the config reaches a VM guest is in [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md); the media
it is written to is in [`ZTP_ISO.md`](ZTP_ISO.md). The modes of the rendered config files, and which process reads each,
are in [`LAB_FOLDER.md`](LAB_FOLDER.md#node-config-modes).

## The setting

One enum, `shared::data::Ztp` (`crates/shared/src/data/node.rs`): `Disabled`, or the one method
Sherpa provisions the node by. There is no separate "enabled" flag, so "off with a method" cannot
be stored.

| Value | Word | Delivery |
|---|---|---|
| `Disabled` | `disabled` | nothing is generated |
| `CloudInit` | `cloud-init` | NoCloud CD-ROM with generated cloud-init, or cloudbase-init on Windows |
| `Cdrom` | `cdrom` | config file on a CD-ROM ISO |
| `Disk` | `disk` | config file on a second disk |
| `Http` | `http` | config served by the lab's boot server, named in DHCP options 67 and 239 |
| `Ignition` | `ignition` | Ignition config as a libvirt firmware config entry (`<sysinfo type='fwcfg'>`) |
| `NoCloud` | `nocloud` | `cidata` CD-ROM carrying the node's own `user-data` |
| `Tftp` | `tftp` | config served over TFTP, named in DHCP option 67 |
| `Usb` | `usb` | config file on a USB disk |
| `Volume` | `volume` | config files mounted into a container |

The word is the same everywhere: the serde name, `Display` (derived with `strum`, held equal to
serde by `crates/shared/tests/enum_strings.rs`), the `node_image.ztp` column, the API field
`ztp.method`, the manifest node key `ztp.method`, the web form's option value and the CLI table.

`ZtpSettings`, the image's `ztp` group, holds it as `method`, its only field. In the
`node_image` table the setting is the `ztp` column. The image's ZTP username, password and
password auth, which nothing provisioned with, were removed in 0.7.0 (schema version 26; see
[`NODE_SETTINGS.md`](NODE_SETTINGS.md)).

## Methods per model

`ModelKind::ztp_methods` lists the methods each model and kind take (see
[Model and kind](NODE_SETTINGS.md#model-and-kind)). It is a `match` over every model with no
wildcard arm, so a new model does not compile until it has an entry; every model takes one kind
today, so the table is per model. `Disabled` is allowed for every pair and is never listed.

| Methods | Models |
|---|---|
| `cloud-init` | the Linux, BSD and Windows VMs, the application VMs, the `generic` VM |
| `tftp` | the `arista_eos` VM, `aruba_aoscx` |
| `tftp`, `usb` | the `juniper_evolved` VM |
| `cdrom` | the Cisco IOS XE, ASA, NX-OS, IOS XR and FTD VMs, `juniper_srxv3`, `juniper_router`, `juniper_switch`, `paloalto_panos` |
| `disk` | `cisco_ios`, `cisco_iosl2`, `cisco_ise`, `mikrotik_chr` |
| `usb` | `cumulus_linux` |
| `http` | the `sonic_linux` VM |
| `ignition` | `flatcar_linux` |
| `nocloud` | `talos_linux` |
| `volume` | the `arista_eos`, `juniper_evolved` and `sonic_linux` containers, `nokia_srlinux`, `cisco_iol`, `cisco_ioll2`, `frr_linux` |
| none | every other container, and the unikernels |

Each pair's default from `ModelKind::node_config` is `Disabled` or a listed method; unit tests in
`model_kind.rs` hold the defaults to that.

`juniper_srxv3` takes `cdrom` only, its default. It also took `tftp` up to 0.6.5, where a vSRX
on TFTP stalled at boot without its config (#676). Migration `0033_srx_ztp_cdrom_only.sql` moves a vSRX image stored on `tftp` to `cdrom`
and keeps `cdrom` and `disabled`. A manifest node that sets `tftp` is refused by
`check_node_ztp` and `validate_node_ztp` with the allowed values, `disabled, cdrom`; on
`redeploy` that is before the node is torn down. `resume` boots the node's existing domain and
generates no config, so a vSRX deployed on TFTP resumes as it was.

## Validation

`shared::validators::check_ztp(model, ztp)` passes `Disabled` for every model and a method only
when the model lists it. Its error names the model, the value and the allowed values.

| Caller | Where | When |
|---|---|---|
| Every image write | `db::upsert_node_image`, `db::update_node_image` | import, scan, download, pull, set-default and the web form all write through these two |
| Web form | `server::validate::validate_node_image_update` | before the image lock and the write, for the form's error message |
| Manifest | `topology::validate::check_node_ztp` | `sherpa validate`, for a node that sets `ztp.method` |
| `up` | `server::validate::validate_node_ztp`, from `plan_lab` | phase 2, before anything is created |
| Redeploy | `validate_node_ztp`, from `load_node` | before the node is torn down |

`validate_node_ztp` checks the setting the node runs with, and the node's `ztp.config` against
it:

- `tftp`, `cdrom`, `disk`, `usb`, `nocloud` and `volume` use a `ztp.config` in place of the
  generated config.
- `cloud-init`, `ignition` and `http` do not, and a node that sets one is refused.
- With `disabled` a `ztp.config` is refused, except on `hashicorp_vault`, whose `ztp.config` is
  its own `config.hcl`, a start setting.
- A Talos cluster member with `disabled` is refused: the machine config Sherpa generates for it
  would not reach it.

## The setting a node runs with

A manifest node may set `ztp.method` to any of the ten words (`topology::Node.ztp.method`,
carried to `NodeExpanded.ztp.method`). Unikernels take no `ztp` group
(`check_node_kind_settings`). `shared::validators::effective_ztp` resolves the two sources: the node's
when it sets one, or else its image's. Every reader below takes its result; none reads the image's
setting directly.

## Readers

Each reader is one `match` on `Ztp` with `Disabled` as a case in it.

| Reader | `Disabled` | Methods |
|---|---|---|
| VM media, `node_ops::generate_vm_ztp_media` | no media; one "ZTP is off for node" message | each VM method's generator |
| Container setup, `node_ops::container_model_setup` | start settings only | `volume`: start settings, then config files, mounts and logins |
| DHCP boot options, `DnsmasqTemplate::boot_options` | none | `tftp`: option 67. `http`: options 67 and 239. The others: none |
| Owner account, `guest_account::supports_guest_account` | none | every method, except on `cisco_ftd` and `talos_linux` |
| Custom config, `validate::ztp` | refused, Vault excepted | as above |

The dnsmasq template only loops over the option lines `boot_options` returns.

### IOS XE day-0 config and PnP

The CSR1000v, Cat8000v and Cat9000v read `iosxe_config.txt` from the CD-ROM with CVAC, which
applies it line by line and then saves it to NVRAM. Until the save, the node has no startup
config, so PnP discovery is running. If the management interface comes up in that window (after
the config's `no shutdown`, before the save, about ten seconds), PnP's Auto Installer takes the
interface for DHCP and gets a lease. When the save lands, discovery stops because a startup config
is present, releases the lease, and leaves the interface with `no ip address`: the running config
has lost the static address, the startup config still has it, and the node never becomes ready.
Whether it happens depends on boot timing; seen on a CSR1000v 17.3 (#649).

The template's first command is therefore `do pnpa service discovery stop`, which ends discovery
before anything else is applied. IOS XE has no configuration-mode command for this, hence `do`.
On CSR1000v 17.3, CVAC then reads one line of binary bytes after the final `exit` and logs a
rejected command (`%CVAC-3-CONFIG_ERROR: 1 error(s)`); everything before it is applied and saved.
Cat8000v 17.16 does not. The template test in `crates/template/tests/cisco_iosxe` holds the stop
to the first command.

## The boot server

Each lab has one boot container, `sherpa-router-<lab-id>` (`ghcr.io/bwks/sherpa-router`), on
the lab's management network at its boot server address. It is the lab's NAT router, and runs
dnsmasq for DHCP, DNS and TFTP, and `webdir`, an HTTP server on port 8080. The image starts
`webdir` on `/opt/ztp` and serves every file and folder under it, listings included; neither is
set by Sherpa. So `boot_server::boot_container_spec` mounts under `/opt/ztp` only what the
`http` method needs:

| Lab folder (host) | In the container | Served by |
|---|---|---|
| `ztp/configs/` | `/opt/ztp` (`konst::BOOT_SERVER_HTTP_ROOT`), the HTTP root | HTTP, `http://<boot server>:8080/<file>` |
| `ztp/tftp/` | `/srv/tftp` (`konst::BOOT_SERVER_TFTP_ROOT`), dnsmasq's `tftp-root` | TFTP only |
| `ztp/dnsmasq/dnsmasq.conf` | `/etc/dnsmasq.conf` | nothing |
| none: the leases | `/var/lib/misc/dnsmasq.leases` (`konst::BOOT_SERVER_LEASES_FILE`), inside the container | nothing |

`ztp/configs/` holds only the files of the nodes that boot by HTTP, which today is `sonic_linux`
alone: its ZTP file map `<node>.conf`, named in DHCP options 67 and 239, and the two files the
map fetches, `<node>_config_db.json` and `<node>_ztp_user.sh`, the owner's account script. The
TFTP configs (the `arista_eos` and `juniper_evolved` VMs, `aruba_aoscx`), the dnsmasq config and the leases are not under the HTTP root, so a node on the
management network can fetch them only as TFTP allows: by name, the TFTP configs only. Up to
0.6.5 the whole `/opt/ztp` was the lab's ZTP folder, so HTTP also listed and served `tftp/` and
`dnsmasq/dnsmasq.leases`.

The modes each process needs are in [`LAB_FOLDER.md`](LAB_FOLDER.md#node-config-modes):
`webdir` and dnsmasq run as root in the container; dnsmasq serves only files others can read.

## Redeploy and the boot server

dnsmasq reads `dnsmasq.conf` only when it starts (`SIGHUP` rereads its host files, not the
config), and `up` writes it once. A VM redeploy calls `boot_server::refresh_boot_server` after the
node's ZTP is generated and before its domain is created:

1. It builds the DHCP host records of the lab's nodes as `up` does (`lab_ztp_records`):
   containers, VMs, then unikernels in index order, each from its record (management addresses,
   MAC) and image. A node's method is its stored definition's, else its image's. The redeployed
   node's record is the one its new ZTP made.
2. It renders the config (`render_dnsmasq_config`, the template `up` uses) and compares it with
   `ztp/dnsmasq/dnsmasq.conf`. Addresses and MACs are kept across a redeploy, so it differs only
   when the node's boot options changed: a move onto TFTP or HTTP adds its options 67 (and 239),
   a move off removes them. In a lab made before 0.7.0 it differs on the first VM redeploy
   too, since the TFTP root and the lease file moved.
3. When it differs, it writes the file, removes the lab's boot container and creates it again
   from `boot_container_spec` (`boot_server::run_boot_container`, which `up` uses too), so a
   lab made before 0.7.0 gets the mounts above along with the config that expects them. The
   leases were inside the old container and start again empty; every node's address is reserved
   by its MAC (`dhcp-host`), so none changes. DHCP, DNS, TFTP, HTTP and the lab's NAT pause
   while it is recreated. When it is the same, nothing is touched.

A failed recreate fails the redeploy before the domain is created. The node's old TFTP or HTTP
files are removed in the same stage (see [`LAB_FOLDER.md`](LAB_FOLDER.md#layout)).

A `match` must cover every variant, so the two generators each keep one arm for the values the
validator excludes: `volume` on a VM, and the eight other methods on a container. It returns an
error saying the setting was not validated. It is reached only if a write or a plan skipped
`check_ztp`.

## Containers: start settings and ZTP

`container_model_setup` applies `container_start_settings` for `Disabled`, and that followed by
`container_volume_ztp` for `Volume`.

| Model | Start settings (`disabled` and `volume`) | Added by `volume` |
|---|---|---|
| `arista_eos` container | image repo, privileged, environment, the init command | the startup config, its read-only mount, and the "copy into flash unless one is saved" prefix on the command |
| `nokia_srlinux` | image repo, privileged, user `0:0`, environment, command | `config.json` and its mount |
| `juniper_evolved` container | image repo, privileged, the `CPTX_*` environment its entrypoint reads | `juniper.conf` in `configdisk` and its mount |
| `cisco_iol`, `cisco_ioll2` | image repo, not privileged, `NET_ADMIN`, the `IOL_ID`, `IOL_MEMORY_MB` and `IOL_ETH_PORTS` environment its entrypoint writes the launcher config from, and IOL's NVRAM directory `run/` mounted at `/tmp/run` | `ios_config.txt` in `config/` and its read-only mount at `/config` |
| `frr_linux` | image repo, privileged, environment | the `daemons` file, `frr.conf`, `sherpa-init.sh`, their mounts, and the command that runs the script |
| `sonic_linux` container | image repo, privileged, `supervisord.conf` and `sherpa-init.sh` (no account, no config) with their read-only mounts, and the command `-c /mnt/sherpa-ztp/supervisord.conf` | `config_db.json` and its read-only mount, and `sherpa-init.sh` written again with the owner's account and the config load |
| `hashicorp_vault` | image, `config.hcl` (generated, or the node's `ztp.config`), its mount, environment, command, capabilities | nothing; its only setting is `disabled` |
| `keycloak_server` | image, user, environment with the node's own admin, command; needs the lab certificate | nothing |
| `gitlab_ce` | image, command, `container.shm_size` | nothing |
| `surreal_db` | image, environment with the node's own root user, command | nothing |
| `forgejo_forge`, `mongo_db`, `nats_bus` | image, command | nothing |
| `mysql_db`, `postgresql_db`, the `generic` container | the manifest's settings | nothing |

With ZTP off, a node of the first five has no generated config, no Sherpa config mount and no
logins (IOL keeps its NVRAM mount):
the owner's login is written into the generated config or script. FRR then runs
its image's own entrypoint and `daemons` file, with no SSH server. A container is ready when it
is running, so readiness does not depend on the setting.

Off means the image as shipped, so the node's management address is usually not configured
either. The address is still reserved for the node and Docker still gives it to the container's
`eth0`, but it is the generated config that makes most network OSes use it. Once booted, as
observed with `tests/labs/container-ztp` (cEOS 4.35.1F, SR Linux 26.7.2, cJunosEvolved 26.2R1.7,
FRR 10.5.2):

| Model | Management address | SSH | Reached with |
|---|---|---|---|
| `nokia_srlinux` | none: SR Linux takes the interface into its own network instance without it | not reachable | `docker exec` |
| `arista_eos` container | none: EOS takes `eth0` as `Management0` and leaves it unassigned; its own ZTP is off | enabled, not reachable | `docker exec ... Cli` |
| `juniper_evolved` container | the node's: the image's entrypoint puts it on `re0:mgmt-0` from the image's own `juniper.conf` | answers, refuses the owner: the image's config has only its own `admin` user | `docker exec` (SSH as `admin` was not tried) |
| `frr_linux` | the node's: a plain Linux container | none | `docker exec` |
| `cisco_iol`, `cisco_ioll2` | none: the entrypoint removes Docker's address from `eth0`, and IOL boots to its factory state and runs its own autoinstall | none | `docker attach`, the IOL console |

While SR Linux and cEOS boot, the container still holds the address: SR Linux's SSH server
answers there for about a minute, refusing the owner, and on cEOS nothing listens yet. The
containers stay running with no restarts. The lab test asserts each booted state. The node's
entry in the lab's SSH config is still written.

`add_container_volumes` adds the manifest's `volumes` to the mounts the model's setup made,
unless the node's setting is `volume`, or the model is `hashicorp_vault`, `gitlab_ce` or
`surreal_db`. So a node of the first five with ZTP off takes the manifest's volumes, which is how
a user mounts a config of their own.

### SONiC container

The `sonic_linux` container runs upstream's `docker-sonic-vs`. Its entrypoint is
`supervisord`, which runs the image's `start.sh`: it reads the container's `ethN` interfaces
**once**, maps them to ports through `lanemap.ini`, and generates the switch config. Sherpa
attaches the data interfaces after the container starts, so the image cannot be started as it
is. The command passes `supervisord` Sherpa's own config (`supervisord.conf`), whose only
program is Sherpa's start script (`sherpa-init.sh`). The script, as root:

1. with `volume`, makes the owner's account if `id <owner>` fails: uid 1000 from `useradd`,
   `/bin/bash`, group `frrvty` (so `vtysh` works without `sudo`), `chpasswd -e` with the `$6$`
   hash and every owner key in `authorized_keys`. If the image has `sudo`, it writes
   `/etc/sudoers.d/90-sherpa-owner` (`0440`, passwordless); if not, it logs that the owner has no
   root access. There is no fallback: root gets no password and no fake `sudo` is written;
2. on the first start only (marker `/etc/ssh/.sherpa-host-keys`), deletes the image's
   `/etc/ssh/ssh_host_*` keys and makes the node's own with `ssh-keygen -A`: every container
   from one image ships with the same set. The container keeps them across a restart and a
   `down`/`resume`; a redeploy makes a new container, so new keys, which its readiness probe
   records as for any node with an SSH ready port;
3. locks root's password and links `/usr/local/bin/rvtysh` to `vtysh`, which `show ip route`
   runs and the image lacks;
4. waits for the node's data interfaces, as many as it has, up to 300 seconds, then `exec`s the
   image's `supervisord`;
5. in the background, once `CONFIG_DB_INITIALIZED` is set and `start.sh` has finished: with
   `volume`, on the first start only (marker `/etc/sonic/.sherpa-config-loaded`), merges
   `config_db.json` into the running config with `sonic-cfggen -j … --write-to-db` and saves the
   result to `/etc/sonic/config_db.json`. Then it turns IPv6 forwarding on, starts `bgpd`, which
   the image defines but does not start, and starts `sshd` with `PermitRootLogin=no`, last.

So port 22 answering means the node is ready, and `ModelKind::ready_port` gives the container
SSH. Nothing is mounted at `/etc/sonic/config_db.json`: `start.sh` renames a file over that path,
which a bind mount cannot take, and a partial file there would stop it generating the `PORT`
table. `config_db.json` is what `SonicLinuxZtp::config` builds for the VM (hostname, local AAA,
all 32 ports admin up) without the management entries, as Docker addresses `eth0`. A custom
`ztp.config` replaces `config_db.json` only; the account is in the start script, so the owner
still gets it (`guest_account::provisions_owner`).

SONiC's hostname (`DEVICE_METADATA|localhost`) comes from that `config_db.json` only: the
image's `start.sh` does not read the Linux hostname, which Sherpa sets on the container itself
(`ContainerSpec::hostname`, the node name). The two agree, so the prompt and
`show runningconfiguration all` show the same name, and nothing in the start script renames the
host.

With ZTP off the start files are still mounted (#679 decision 4), so the interface wait always
applies; the node gets no account and no config, and its SSH server still starts, as the
readiness signal.

The container keeps its filesystem across a restart, so each step is safe to run again. `down`
pauses the container and `resume` unpauses it, keeping everything, saved or not. A container
that was stopped is started again by `resume` (a cold boot): the script waits for the
re-attached links, does not load Sherpa's config again, and `start.sh` loads the saved one.
`redeploy` makes a new container, which loads Sherpa's config again.

Observed with the 202511 image (build `1278069`) with `sudo` added: 50 seconds from start to
ready with 32 data interfaces; `show` and `vtysh` work without `sudo`, `config …` needs `sudo`,
and `show version` fails, as it calls `docker`. Every container from one image has the image's
SSH host keys.

### Cisco IOL

The image is the user's CML image rebuilt by
[docker-cisco-iol](https://github.com/bwks/docker-cisco-iol): Cisco's launcher, `/iol-runner`,
starts IOL, and a bridge relays each container interface `eth<N>` to IOL port `N`,
`Ethernet<N / 4>/<N % 4>`. The launcher's config is the image's business: Sherpa passes three
environment variables and the entrypoint writes it.

- `IOL_ID` is IOL's application ID, the node's index in the lab (`node_ops::iol_app_id`), 1 to
  `node_ops::CISCO_IOL_MAX_APP_ID` (999; the launcher takes 1000). IOL derives its MAC addresses
  from it, so two IOL nodes of one lab never share one.
- `IOL_MEMORY_MB` is the node's `memory`, else the image's: IOL's `-m`, the whole IOL process.
- `IOL_ETH_PORTS` is the data interface count plus management.

`cisco_iol.jinja` gives `Ethernet0/0` the node's management address, static, and the default
route; the L2 model makes the port routed (`no switchport`) and turns `ip routing` on. The
launcher copies `/config/ios_config.txt` into `/tmp/run` at every start, but IOL boots from its
NVRAM once a config has been saved there with `write memory`. `/tmp/run` is `<lab>/<node>/run`,
so a saved config survives `down` and `resume`; a redeploy makes the node's folder again, which
clears it. The owner's login uses an MD5 `key-hash` RSA key, as on IOSv.

## Blank disks

The `disk` and `usb` methods, and Flatcar's data disk, start from a blank disk template in
`/opt/sherpa/images/blank_disk/` (`konst::SHERPA_BLANK_DISK_DIR`). `node_ops` copies the template
into the node's folder and writes the config onto the copy with `mcopy` (FAT) or `e2cp` (ext4);
the template itself is never changed. A missing template fails the node's ZTP with
`Error loading src file`.

| File | Size | File system | Label | Used by |
|---|---|---|---|---|
| `fat32.img` | 64 MiB | FAT32 | none | `cumulus_linux` USB, `mikrotik_chr` config disk |
| `junos.img` | 64 MiB | FAT16 | `vmm-data` | `juniper_evolved` VM USB |
| `iosv.img` | 131040 sectors | FAT16 | none | `cisco_ios`, `cisco_iosl2` config disk |
| `ise.img` | 10 MiB | ext4 | `ISE-ZTP` | `cisco_ise` config disk |
| `ext4-500mb.img` | 500 MiB | ext4 | `data-disk` | `flatcar_linux` data disk; Ignition mounts it by label |

The same folder also gets `base.img` (64 MiB, no file system), `fat16.img` (64 MiB) and ext4
`data-disk` disks of 100 MiB, 200 MiB and 1 to 5 GiB, which no model uses today.

`sherpad init` creates the folder but no disk. The disks are made by `ensure_blank_disks` in
`scripts/sherpa_install.sh`, after the directories are set up, and in `scripts/sherpa_update.sh`,
before `sherpad` stops, on every update including one with nothing to install, so a server
installed before the installer made them gets the missing ones. The function:

- creates `images` and `blank_disk` `0775 sherpa:sherpa` when they are missing, and leaves
  existing ones as they are;
- creates each missing disk `0644 sherpa:sherpa`, with `qemu-img create -f raw` and then
  `mkfs.fat`, `mkfs.vfat` or `mkfs.ext4`;
- never changes a file that exists, whatever its contents, since node disks may have been copied
  from it, and leaves a symlink of a disk's name alone;
- builds each disk under a temporary name in the same folder and links it to its own name with
  `ln`, which fails rather than replace a file made meanwhile. A disk that fails to build leaves
  no file; the script stops with the disk's name.

The disk table and the functions are one section, between `# BEGIN blank disks` and
`# END blank disks`, copied in both scripts, because each script is fetched alone with `curl`
and has nothing next to it to run. `tests/bats/install_tests.bats` fails when the two copies
differ. The tools are in `qemu-utils`, `dosfstools` and `e2fsprogs`: the installer installs them
with its other packages, and the update script installs them when one is missing.

`iosv.img` was made by `create_iosv_disk.sh`, which wrote a hand-made boot sector and then ran
`mkfs.vfat -I` over it; mkfs replaces the whole boot sector, so the result is the same as
`mkfs.vfat -I` on a zeroed file of that size, apart from the random serial number.
`create_blank_disks.sh` stopped at its second `ext4-500mb.img` line, whose `500MB` size
`qemu-img` refuses, so the 1 to 5 GiB disks were never made by it. Both scripts are gone.

## Storage and migration

`node_image.ztp` is the table's last column, written with `enum_text` and read with
`decode_enum`. Migration `0016_ztp_setting.sql` adds it, fills it from the `ztp_enable` and
`ztp_method` columns it replaces, and drops those two. The first rule that matches decides:

| Stored before | `ztp` |
|---|---|
| `ztp_enable = 0`, any method | `disabled` |
| enabled, one of the four mounted-config containers | `volume` |
| enabled, any other container or a unikernel | `disabled` |
| enabled, a VM with a method its model takes | the same method |
| enabled, a VM with `none`, `volume` or `ipxe` | `disabled` |
| enabled, a VM with another method its model does not take | the model's default method |
| a model this build does not know | `disabled` |

`db::schema::apply`'s tests insert every model with every one of the 22 old pairs at version 15
and check each migrated row against this table and `check_ztp`.

## Tests

- Unit: the table, defaults and validator (`shared`), the migration and write refusals (`db`),
  DHCP options (`template`), the generators, owner account and plan-time check (`server`), the
  manifest field (`topology`, `client`).
- Integration: `lab_lifecycle::frr_gets_its_config_mounts_and_command_only_with_ztp_on` and
  `up_refuses_an_unsupported_ztp_setting_before_creating_anything`, against real Docker.
- Lab: `tests/labs/container-ztp`.
- Blank disks: `tests/bats/install_tests.bats` makes every disk in a temporary folder and checks
  its size, file system and label with `blkid` and `file`, that existing files and folders are
  left alone, and, after a real install, the disks under `/opt/sherpa`.
