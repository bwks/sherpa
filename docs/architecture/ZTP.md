# Zero touch provisioning (ZTP)

How a node gets its first configuration: the one setting that decides it, which methods each
model takes, where that is checked, who reads it, and what a container gets with ZTP off. How
the config reaches a VM guest is in [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md); the media
it is written to is in [`ZTP_ISO.md`](ZTP_ISO.md).

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
| `Ignition` | `ignition` | Ignition config passed to QEMU |
| `NoCloud` | `nocloud` | `cidata` CD-ROM carrying the node's own `user-data` |
| `Tftp` | `tftp` | config served over TFTP, named in DHCP option 67 |
| `Usb` | `usb` | config file on a USB disk |
| `Volume` | `volume` | config files mounted into a container |

The word is the same everywhere: the serde name, `Display` (derived with `strum`, held equal to
serde by `crates/shared/tests/enum_strings.rs`), the `node_image.ztp` column, the API field
`ztp`, the manifest node field `ztp`, the web form's option value and the CLI table.

`ZtpSettings` holds it as `mode`, serialised as `ztp`, beside `ztp_username`, `ztp_password` and
`ztp_password_auth`, which are only displayed.

## Methods per model

`NodeModel::ztp_methods` lists the methods each model takes. It is a `match` over every model
with no wildcard arm, so a new model does not compile until it has an entry. `Disabled` is
allowed for every model and is never listed.

| Methods | Models |
|---|---|
| `cloud-init` | the Linux, BSD and Windows VMs, the application VMs, `generic_vm` |
| `tftp` | `arista_veos`, `aruba_aoscx` |
| `tftp`, `cdrom` | `juniper_vsrxv3` |
| `tftp`, `usb` | `juniper_vevolved` |
| `cdrom` | the Cisco IOS XE, ASA, NX-OS, IOS XR and FTD VMs, `juniper_vrouter`, `juniper_vswitch`, `paloalto_panos` |
| `disk` | `cisco_iosv`, `cisco_iosvl2`, `cisco_ise`, `mikrotik_chr` |
| `usb` | `cumulus_linux` |
| `http` | `sonic_linux` |
| `ignition` | `flatcar_linux` |
| `nocloud` | `talos_linux` |
| `volume` | `arista_ceos`, `nokia_srlinux`, `juniper_cevolved`, `frr_linux` |
| none | every other container, and the unikernels |

Each model's default from `NodeConfig::get_model` is `Disabled` or a listed method; unit tests in
`node.rs` hold the table and the defaults to that.

## Validation

`shared::validators::check_ztp(model, ztp)` passes `Disabled` for every model and a method only
when the model lists it. Its error names the model, the value and the allowed values.

| Caller | Where | When |
|---|---|---|
| Every image write | `db::upsert_node_image`, `db::update_node_image` | import, scan, download, pull, set-default and the web form all write through these two |
| Web form | `server::validate::validate_node_image_update` | before the image lock and the write, for the form's error message |
| Manifest | `topology::validate::check_node_ztp` | `sherpa validate`, for a node that sets `ztp` |
| `up` | `server::validate::validate_node_ztp`, from `plan_lab` | phase 2, before anything is created |
| Redeploy | `validate_node_ztp`, from `load_node` | before the node is torn down |

`validate_node_ztp` checks the setting the node runs with, and the node's `ztp_config` against
it:

- `tftp`, `cdrom`, `disk`, `usb`, `nocloud` and `volume` use a `ztp_config` in place of the
  generated config.
- `cloud-init`, `ignition` and `http` do not, and a node that sets one is refused.
- With `disabled` a `ztp_config` is refused, except on `hashicorp_vault`, whose `ztp_config` is
  its own `config.hcl`, a start setting.
- A Talos cluster member with `disabled` is refused: the machine config Sherpa generates for it
  would not reach it.

## The setting a node runs with

A manifest node may set `ztp` to any of the ten words (`topology::Node.ztp`, carried to
`NodeExpanded.ztp`). `shared::validators::effective_ztp` resolves the two sources: the node's
when it sets one, or else its image's. Every reader below takes its result; none reads the image's
setting directly.

## Readers

Each reader is one `match` on `Ztp` with `Disabled` as a case in it.

| Reader | `Disabled` | Methods |
|---|---|---|
| VM media, `node_ops::generate_vm_ztp_media` | no media; one "ZTP is off for node" message | each VM method's generator |
| Container setup, `node_ops::container_model_setup` | start settings only | `volume`: start settings, then config files, mounts and logins |
| DHCP boot options, `DnsmasqTemplate::boot_options` | none | `tftp`: option 67. `http`: options 67 and 239. The others: none |
| Owner account, `guest_account::supports_guest_account` | none | every method, except on `cisco_ftdv` and `talos_linux` |
| Custom config, `validate::ztp` | refused, Vault excepted | as above |

The dnsmasq template only loops over the option lines `boot_options` returns.

A `match` must cover every variant, so the two generators each keep one arm for the values the
validator excludes: `volume` on a VM, and the eight other methods on a container. It returns an
error saying the setting was not validated. It is reached only if a write or a plan skipped
`check_ztp`.

## Containers: start settings and ZTP

`container_model_setup` applies `container_start_settings` for `Disabled`, and that followed by
`container_volume_ztp` for `Volume`.

| Model | Start settings (`disabled` and `volume`) | Added by `volume` |
|---|---|---|
| `arista_ceos` | image repo, privileged, environment, the init command | the startup config, its read-only mount, and the "copy into flash unless one is saved" prefix on the command |
| `nokia_srlinux` | image repo, privileged, user `0:0`, environment, command | `config.json` and its mount |
| `juniper_cevolved` | image repo, privileged, the `CPTX_*` environment its entrypoint reads | `juniper.conf` in `configdisk` and its mount |
| `frr_linux` | image repo, privileged, environment | the `daemons` file, `frr.conf`, `sherpa-init.sh`, their mounts, and the command that runs the script |
| `hashicorp_vault` | image, `config.hcl` (generated, or the node's `ztp_config`), its mount, environment, command, capabilities | nothing; its only setting is `disabled` |
| `keycloak_server` | image, user, environment, command; needs the lab certificate | nothing |
| `gitlab_ce` | image, command, `shm_size` | nothing |
| `forgejo_forge`, `surreal_db`, `mongo_db`, `nats_bus` | image, command | nothing |
| `mysql_db`, `postgresql_db`, `generic_container` | the manifest's settings | nothing |

With ZTP off, a node of the first four has no generated config, no Sherpa mount and no logins:
the `sherpa` login and the owner's are written into the generated config or script. FRR then runs
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
| `arista_ceos` | none: EOS takes `eth0` as `Management0` and leaves it unassigned; its own ZTP is off | enabled, not reachable | `docker exec ... Cli` |
| `juniper_cevolved` | the node's: the image's entrypoint puts it on `re0:mgmt-0` from the image's own `juniper.conf` | answers, refuses `sherpa`: the image's config has only its own `admin` user | `docker exec` (SSH as `admin` was not tried) |
| `frr_linux` | the node's: a plain Linux container | none | `docker exec` |

While SR Linux and cEOS boot, the container still holds the address: SR Linux's SSH server
answers there for about a minute, refusing `sherpa`, and on cEOS nothing listens yet. The
containers stay running with no restarts. The lab test asserts each booted state. The node's
entry in the lab's SSH config is still written.

`add_container_volumes` applies the manifest's `volumes` unless the node's setting is `volume`,
or the model is `hashicorp_vault`, `gitlab_ce` or `surreal_db`. So a node of the first four with
ZTP off takes the manifest's volumes, which is how a user mounts a config of their own.

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
