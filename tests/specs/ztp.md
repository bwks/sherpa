# ZTP

How Sherpa gives each node its first configuration at boot: the ZTP setting an image and a node have, what a node with ZTP off gets, how a config reaches a node by each method, the config ISO, a node's own `ztp.config`, the config Sherpa generates for each model, the management addresses, the name server and the accounts in it, the lab's boot server, and who can read a node's config on the server.

What the owner's account is on every node, whatever its model (which keys, which name, the form of the password, its rights on each operating system), is `owner_login` in [accounts-and-auth.md](accounts-and-auth.md); here is only how each model's config writes it. That up refuses an unsupported setting is `lab_preflight_004` in [lab-lifecycle.md](lab-lifecycle.md), a container's start settings are `container_run` in [nodes.md](nodes.md), and which addresses a lab's nodes get is `lab_addressing`. The machine config of a Talos cluster member is made from the cluster's secrets and belongs to the Talos spec; how any `nocloud` config reaches its node is here. A manifest's text files, scripts and environment variables, which cloud-init and Ignition also carry, belong to Provisioning. Reading `ztp` out of a manifest belongs to Manifests.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `ztp_setting_004_a_method_the_model_does_not_take_is_refused_naming_what_it_takes`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

A config template is tested one case per model, not one table-driven case for all: each model has a template and a syntax of its own, so the expected text is the model's, and a failure names the model.

## `ztp_setting`: a node is built with one ZTP setting, which its model takes

**Statement:** every node is built with one ZTP setting, its manifest's `ztp.method` or else its image's, and it is `disabled` or a method its model and kind take.
**Relied on by:** the user, the admin who sets an image's. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa validate` refuses a `ztp.method` the node's model does not take, naming the node and the settings its model takes, and accepts the manifest without it | e2e |
| 002 | A node is built with its manifest's method, and with its image's when the manifest sets none | unit |
| 003 | `disabled` is taken for every model and kind | unit |
| 004 | A method the model and kind do not take is refused, naming the model, the method and every setting the model takes, which for a model with no method is `disabled` alone | unit |
| 005 | Up and redeploy check the setting the node will be built with, so an image stored with a method its model does not take is refused as a manifest's is, naming the node | unit |
| 006 | A redeploy whose manifest sets a method the node's model does not take is refused, naming what the model takes, and the node stays as it was | integration |
| 007 | The database refuses an image whose setting its model does not take, when it is recorded and when it is edited, and what was stored is unchanged | integration |
| 008 | The database stores `disabled` and a method the model takes, when an image is recorded and when it is edited | integration |
| 009 | The image edit page offers `disabled` and the model's methods and nothing else, with the image's own chosen | unit |
| 010 | The edit form's setting is read by its word, and a word that is not a setting is refused | unit |
| 011 | A saved edit form carries its setting to the image | unit |
| 012 | An edit to a method the model does not take is refused, naming what it takes | unit |
| 013 | The built-in setting of every model and kind is `disabled` or a method it takes | unit |
| 014 | A member of a Talos cluster with ZTP off is refused, naming the cluster; a Talos node in no cluster may have it off | unit |
| 015 | A VM whose manifest sets no method is given the media of its image's method | unit |
| 016 | After `ztp.method` is changed in the manifest, `sherpa redeploy` rebuilds the node with the new setting (no test yet) | e2e |

## `ztp_disabled`: a node with ZTP off boots as its image ships

**Statement:** a node whose ZTP setting is `disabled` boots as its image ships: Sherpa generates no config for it, gives it no config media or mount, and adds no login.
**Relied on by:** the user who brings a config of their own, or wants the image untouched. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container with `ztp.method = "disabled"` runs with none of its model's config mounts | e2e |
| 002 | A container with ZTP off refuses its owner's SSH login, for the reason its model has: no address, or no SSH server | e2e |
| 003 | A container redeployed with ZTP off is as it was after up: running, with no config mount and no owner login | e2e |
| 004 | Every node of the lab, the ones with ZTP off and an application container included, is still running and was never restarted once the others have booted | e2e |
| 005 | A VM with ZTP off is built with its boot disk alone, no config CD-ROM, config disk, USB disk or firmware config, and nothing is written for it | unit |
| 006 | A VM with ZTP off boots from its image with no config media and no owner login (no test yet) | e2e |
| 007 | A `sonic_linux` container with ZTP off still gets Sherpa's start script, which waits for its data interfaces and starts its SSH server, with no account and no config in it | unit |
| 008 | A `cisco_iol` container with ZTP off has no config mount and keeps its NVRAM folder and the manifest's mounts | unit |
| 009 | The manifest's `container.volumes` reach a container of a model whose config Sherpa mounts only with ZTP off; with ZTP on they are left out | unit |

## `config_delivery`: a node's config reaches it at first boot by its ZTP method

**Statement:** a node's config reaches it at first boot by its ZTP method: on a CD-ROM, a config disk or a USB disk, through the firmware, as mounted files, or from the lab's boot server over TFTP or HTTP.
**Relied on by:** the user, every node that is to come up configured. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container with ZTP on runs with every config mount of its model and takes its owner's SSH login by key, after up and after a redeploy | e2e |
| 002 | A `cisco_iol` node's running config has the name, the management interface and the owner's account Sherpa generated, read over the owner's SSH login (no test with its ID yet; `tests/labs/cisco-iol` shows it) | e2e |
| 003 | A cloud-init VM finishes cloud-init without an error (no test with its ID yet; `tests/labs/nixos-linux` shows it) | e2e |
| 004 | A VM of each other method, `cdrom`, `disk`, `usb`, `tftp`, `http`, `ignition` and `nocloud`, boots with its name and its management address from the config Sherpa delivered (no test yet) | e2e |
| 005 | An `frr_linux` container with ZTP on has its three config mounts and its start command, at up and after a redeploy | integration |
| 006 | A config medium is copied into the storage pool byte for byte | integration |
| 007 | `cloud-init`: `user-data`, `meta-data` and `network-config` are written to an ISO labelled `cidata`, which is the VM's CD-ROM (no test yet) | unit |
| 008 | `nocloud`: the node's own config is `user-data`, byte for byte, beside Sherpa's `meta-data` and a `network-config` with its static management address | unit |
| 009 | `nocloud` with no config: `meta-data` and `network-config` are written and no `user-data` | unit |
| 010 | `cdrom`: the config is written under the file name its model reads, and an ISO is made of it | unit |
| 011 | `cdrom` on PAN-OS: `init-cfg.txt` and `bootstrap.xml` are in the `config` folder of an ISO labelled `bootstrap`, beside empty `content`, `license` and `software` folders (no test yet) | unit |
| 012 | `disk`: the config is copied onto a blank disk image, FAT for IOSv, IOSvL2 and RouterOS and ext4 for ISE, attached as the VM's second disk, and RouterOS gets each of the owner's keys as a file beside it (no test yet) | integration |
| 013 | `usb`: the config is copied onto a FAT disk image attached as a USB disk, and Junos Evolved gets it also as the archive it reads (no test yet) | integration |
| 014 | `ignition`: the config is copied into the storage pool and given to the VM as a firmware config entry of its domain, not as a disk and not as a QEMU argument | unit |
| 015 | A domain with an Ignition config has the firmware config entry, and one without has none | unit |
| 016 | `tftp`: the config is `<node>.conf` in the lab's TFTP folder (no test yet) | unit |
| 017 | `http`: the node's ZTP file names its config database and its account script by their URLs on the boot server | unit |
| 018 | `volume` on cEOS: the startup config is mounted read-only and the start command copies it into flash; with ZTP off nothing is mounted and the image's plain init command runs | unit |
| 019 | `volume` on SR Linux: the config is mounted over the image's, and with ZTP off nothing is mounted | unit |
| 020 | `volume` on cJunosEvolved: the config directory holding `juniper.conf` is mounted read-only, and with ZTP off nothing is mounted | unit |
| 021 | `volume` on FRR: its `daemons` file, `frr.conf` and start script are mounted and the script is its command, and with ZTP off none is | unit |
| 022 | `volume` on IOL and IOL L2: the config directory holding the startup config is mounted read-only, beside the NVRAM folder | unit |
| 023 | `volume` on the SONiC container: the `supervisord` config, the start script and `config_db.json` are mounted read-only | unit |
| 024 | `volume` on a VM fails as not validated, and nothing is written | unit |
| 025 | Any method but `volume` on a container fails as not validated, and nothing is written | unit |
| 026 | `volume` on a container of a model Sherpa generates no config for fails as not validated, naming the model | unit |
| 027 | The image's own CD-ROM, such as vEOS's Aboot, is attached unless the ZTP method brings a CD-ROM (no test yet) | unit |
| 028 | At up, every container and every VM is issued a certificate and a key by the lab's CA, in the lab's certificates folder beside the CA's own, and a unikernel is issued none | integration |
| 029 | An Ignition config carries the lab CA's certificate and the node's certificate as files anyone on the node reads (0644), and the node's key as a file only root reads (0600) | unit |

The lab's CA and a node's certificate are here because they reach the node in its first config: as files of its cloud-init, cloudbase-init or Ignition config, or as mounts of a container. A unikernel has no boot config or mount to carry one.

## `config_iso`: a config ISO holds a node's files as every guest reads them

**Statement:** a config ISO holds the files of one folder, under their own names and with their contents unchanged, readable by Linux, BSD, Windows and network operating systems, and a folder it cannot hold whole is refused.
**Relied on by:** cloud-init, cloudbase-init, Talos and every `cdrom` model, which find their config by file name and volume label. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The image is whole sectors with a primary and a Joliet volume descriptor, each carrying the volume label | unit |
| 002 | The primary tree keeps each name whole and in its own case, hyphens included | unit |
| 003 | Rock Ridge entries carry each file's own name, with every file and folder read-only | unit |
| 004 | The Joliet tree holds the same files as the Rock Ridge tree | unit |
| 005 | Every file's contents read back unchanged, an empty file and a large one included | unit |
| 006 | A folder with more entries than one sector holds is written whole | unit |
| 007 | The path tables list every folder under its parent | unit |
| 008 | An ISO written into the folder it is made from leaves itself out | unit |
| 009 | A write that fails keeps the image that was there, and leaves no partial image | unit |
| 010 | A partial image left in the folder by an earlier failed write is left out of the next one | unit |
| 011 | A source folder that does not exist is refused, and no image is made | unit |
| 012 | A volume label that is empty, has a space or is too long is refused | unit |
| 013 | A name too long for the format, and two names that become one in it, are refused | unit |
| 014 | Folders nested more than eight deep are refused | unit |
| 015 | A symlink in the folder is refused, naming it | unit |
| 016 | A date the format cannot hold is refused, in a record and in a volume descriptor | unit |
| 017 | A config ISO Sherpa made is mounted by a Linux guest, which reads each file under its name (no test yet) | e2e |

## `custom_config`: a node's own `ztp.config` replaces the config Sherpa generates

**Statement:** a node's `ztp.config` file is delivered to it in place of the config Sherpa generates, byte for byte, by the methods that deliver a config file, and is refused on any other node.
**Relied on by:** the user who brings a device config of their own. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A node with a `ztp.config` boots with that config and not Sherpa's (no test yet) | e2e |
| 002 | The CLI reads `ztp.config` from the user's machine, relative to the manifest's folder, and sends its contents; a node without one is left alone | unit |
| 003 | A `ztp.config` file that is empty is refused, naming the node and the file | unit |
| 004 | A `ztp.config` file that cannot be read is refused, naming the node and the file | unit |
| 005 | A config is taken with `tftp`, `cdrom`, `disk`, `usb`, `nocloud` and `volume`, and refused with `cloud-init`, `ignition` and `http`, naming the method | unit |
| 006 | A config on a node with ZTP off is refused for every model but `hashicorp_vault` | unit |
| 007 | A `hashicorp_vault` node's `ztp.config` is Vault's own config file, mounted as it is with its ZTP off | unit |
| 008 | An IOL node's config is written as its startup config, as it is | unit |
| 009 | A config for a `tftp`, `cdrom`, `disk` or `usb` node is written under the file name its model reads, as it is (no test yet) | unit |
| 010 | A redeploy with a changed `ztp.config` rebuilds the node with the new one (no test yet) | e2e |
| 011 | A PAN-OS node's config is `init-cfg.txt` in the `config` folder of an ISO labelled `bootstrap`, beside empty `content`, `license` and `software` folders, with no generated `bootstrap.xml` | unit |

A node with a custom config gets no owner login, except FRR and the SONiC container: `owner_login_017` and `owner_login_018`. A Talos cluster member, and the Vault node a cluster names, may set no `ztp.config`: that belongs to the Talos spec.

## `generated_config`: the config Sherpa generates is its model's own

**Statement:** the config Sherpa generates for a node is written as its model reads it, and gives the node its name, its management address and default route, and an SSH server its owner can reach.
**Relied on by:** the user, SSH, provisioners. **Priority:** P0.

Cases 001 to 024 are each the whole config of one model, for a lab with IPv4 only, unless they say otherwise.

| Case | What it shows | Level |
|---|---|---|
| 001 | Arista vEOS | unit |
| 002 | Arista cEOS | unit |
| 003 | Aruba AOS-CX, with `admin` and the owner locked to key logins | unit |
| 004 | Cisco ASAv | unit |
| 005 | Cisco FTDv: day-0 JSON with the EULA accepted, a manual IPv4 address, gateway and mask, a name server, managed locally | unit |
| 006 | Cisco IOL | unit |
| 007 | Cisco IOL L2: the management port is a routed port with the address, and IP routing is on | unit |
| 008 | Cisco IOSv | unit |
| 009 | Cisco IOSvL2 (no test yet) | unit |
| 010 | Cisco IOS XE, which stops PnP discovery first | unit |
| 011 | Cisco IOS XE with a boot licence line, as Cat8000v and Cat9000v get | unit |
| 012 | Cisco IOS XR | unit |
| 013 | Cisco ISE | unit |
| 014 | Cisco NX-OS | unit |
| 015 | Cumulus Linux | unit |
| 016 | FRR: `frr.conf` | unit |
| 017 | FRR: the start script, which installs and starts the SSH server and makes the owner's account | unit |
| 018 | Junos, the one config vRouter, vSwitch, vSRX, vEvolved and cEvolved get | unit |
| 019 | MikroTik RouterOS | unit |
| 020 | Nokia SR Linux; with no IPv6 in the lab its management interface asks for an IPv6 address by DHCP | unit |
| 021 | Nokia SR Linux: the management network instance and its ACL | unit |
| 022 | Palo Alto PAN-OS: `init-cfg.txt` | unit |
| 023 | Palo Alto PAN-OS: `bootstrap.xml` | unit |
| 024 | SONiC: `config_db.json` with the name, every port up and the management port and address | unit |
| 025 | MikroTik RouterOS imports each of the owner's key files only if the file is there | unit |
| 026 | SONiC container: `supervisord` runs only Sherpa's start script | unit |
| 027 | SONiC container: the start script waits for the node's data interfaces, then hands over to SONiC | unit |
| 028 | SONiC container: the start script loads Sherpa's config on the first start only, and saves it | unit |
| 029 | SONiC container: with no config and no owner, the start script loads none, makes no account and still starts the SSH server | unit |
| 030 | SONiC container: the SSH server starts last, after the switch and its config | unit |
| 031 | SONiC container: the node's own SSH host keys are made on the first start only, before the SSH server starts | unit |
| 032 | cloud-init `user-data` carries the node's name, FQDN, packages and commands | unit |
| 033 | cloud-init trusts the lab's CA | unit |
| 034 | cloud-init leaves SSH password logins to the image when the model does not set them | unit |
| 035 | cloud-init writes the lab's domain and name server to `resolv.conf` when it manages it, as on Alpine | unit |
| 036 | cloud-init `meta-data` names the instance and the node | unit |
| 037 | A MAC address of digits only stays a string in cloud-init `network-config` | unit |
| 038 | Words YAML reads as booleans stay strings in cloud-init `user-data` | unit |
| 039 | A MAC address of digits only stays a string in cloudbase-init `network-config` | unit |
| 040 | cloudbase-init `user-data` carries the Windows node's name | unit |
| 041 | An Ignition config carries its users, files, links, units and directories | unit |
| 042 | An Ignition file that is only appended to carries no contents of its own | unit |
| 043 | An Ignition directory carries its owner and group | unit |

## `config_addressing`: a generated config gives the node its reserved management addresses

**Statement:** a generated config gives the node the management addresses its lab reserved for it, as static addresses with the lab's gateway: IPv4 always, and IPv6 when the lab has it and the model's config carries one.
**Relied on by:** the user, SSH, provisioners, the readiness check. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | cloud-init `network-config`: the interface with the node's MAC address gets its IPv4 address, the gateway and the lab's name server | unit |
| 002 | cloud-init `network-config` in a lab with IPv6 adds the IPv6 address, gateway and name server | unit |
| 003 | cloudbase-init `network-config`: the same for a Windows node, IPv4 | unit |
| 004 | cloudbase-init `network-config` in a lab with IPv6 | unit |
| 005 | Flatcar: the Ignition network file gives the management interface its IPv4 address, gateway and name server | unit |
| 006 | Flatcar: the same file in a lab with IPv6 | unit |
| 007 | Arista vEOS in a lab with IPv6 has its IPv6 address and default route | unit |
| 008 | Cisco IOSv in a lab with IPv6 | unit |
| 009 | Cisco IOL in a lab with IPv6 | unit |
| 010 | Nokia SR Linux in a lab with IPv6 has both addresses and asks for none by DHCP | unit |
| 011 | The configs of AOS-CX, ASAv, cEOS, IOSvL2, IOS XE, IOS XR, ISE, NX-OS, Cumulus Linux, FRR, Junos, RouterOS and PAN-OS in a lab with IPv6 have the node's IPv6 address and default route (no test yet) | unit |
| 012 | A SONiC container's config has no management port or address: Docker gives the container its address | unit |
| 013 | An OpenBSD node's `network-config` has no IPv6 address in a lab with IPv6 (no test yet) | unit |
| 014 | A VM answers on the management address Sherpa recorded for it once it has booted (no test yet) | e2e |

## `config_name_server`: a generated device config names the lab's boot server as its name server

**Statement:** a device config Sherpa generates names the lab's boot server, which holds a name record for every node, as its one name server, never the gateway, on IPv4 and, where the model's config carries one, IPv6.
**Relied on by:** anything on a node that reaches another node by its name. **Priority:** P1.

cloud-init, cloudbase-init, Ignition, FTDv, ISE and PAN-OS take the boot server from the same place, and their cases are in `config_addressing` and `generated_config`. Junos and FRR configs set no name server.

| Case | What it shows | Level |
|---|---|---|
| 001 | The lab's DNS settings name its boot server as name server, on IPv4 and, in a lab with IPv6, IPv6, and not its gateway | unit |
| 002 | Arista vEOS | unit |
| 003 | Arista cEOS | unit |
| 004 | Aruba AOS-CX | unit |
| 005 | Cisco ASAv | unit |
| 006 | Cisco IOL and IOL L2, which have one template | unit |
| 007 | Cisco IOSv | unit |
| 008 | Cisco IOSvL2 | unit |
| 009 | Cisco IOS XE | unit |
| 010 | Cisco IOS XR | unit |
| 011 | Cisco NX-OS | unit |
| 012 | Cumulus Linux | unit |
| 013 | MikroTik RouterOS | unit |
| 014 | Nokia SR Linux, with the boot server's IPv4 and IPv6 addresses in a lab with IPv6 | unit |
| 015 | Nokia SR Linux searches the lab's domain, so a node's bare name resolves | unit |

## `device_accounts`: a generated config makes the owner's account and the model's fixed ones, no other

**Statement:** a generated config makes the owner's account, gives the model's fixed vendor accounts the owner's device password, and makes no other account, with the password or its hash unchanged.
**Relied on by:** the user, who logs in with their device password; anyone relying on a node having no default login. **Priority:** P0.

Cases 001 to 014 render the model's config with device passwords that between them use every character a device password may have, and show each in the config unchanged, as many times as the model has accounts.

| Case | What it shows | Level |
|---|---|---|
| 001 | Cisco ASAv: the enable password and the owner | unit |
| 002 | Cisco FTDv: `admin`, its only account | unit |
| 003 | Cisco IOL: the owner | unit |
| 004 | Cisco IOSv and IOSvL2: the owner is the only account of each | unit |
| 005 | Cisco IOS XE: the owner is the only account | unit |
| 006 | Cisco IOS XR: `admin` and the owner | unit |
| 007 | Cisco ISE: the owner, its one administrator | unit |
| 008 | Cisco NX-OS: `admin` and the owner | unit |
| 009 | Cumulus Linux: the owner | unit |
| 010 | MikroTik RouterOS: the first user and the owner are the only accounts | unit |
| 011 | MikroTik RouterOS: the password of the first user and of the owner | unit |
| 012 | Nokia SR Linux: `admin` and the owner are the only accounts | unit |
| 013 | Nokia SR Linux: the password of `admin` and of the owner | unit |
| 014 | Windows: the owner's password in cloudbase-init `user-data`, the one account | unit |
| 015 | SONiC container: the start script makes the owner's account, with their keys and the hash of their password, only when it does not exist | unit |
| 016 | SONiC container: the start script locks `root` and refuses its SSH login, with an owner and without | unit |
| 017 | SONiC container: the owner gets passwordless sudo when the image has sudo | unit |
| 018 | SONiC container: with no sudo in the image, the start script says so and gives the owner no other way to root | unit |
| 019 | SONiC: the account script makes the owner with their keys, the hash of their password and sudo | unit |
| 020 | A cloud-init account with no groups has no groups key, and one with groups has them as one comma-separated string | unit |
| 021 | A device password hash names its algorithm and a salt of its own, and has no rounds field, as the devices that store one read it | unit |
| 022 | A `juniper_srxv3` node's config has the owner and `root` each with a hash of the device password | unit |
| 023 | The configs served over TFTP and HTTP carry only a hash of the device password, and AOS-CX's carries none | unit |
| 024 | The models that store a hash, Arista, Junos, PAN-OS, FRR and SONiC, each have the hash unchanged for every character a password may have, and only the accounts their case in `generated_config` shows (no test yet) | unit |

## `boot_server`: each lab has a boot server for its nodes' addresses, names and network configs

**Statement:** each lab has a boot server on its management network that gives every VM and unikernel its reserved address by DHCP, resolves every node's name, and serves their configs to the nodes that boot by TFTP or HTTP.
**Relied on by:** every VM and unikernel at boot, the nodes that fetch their config, anything in the lab that uses a node's name. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A node resolves another node of its lab by name and by its name in the lab's domain (no test yet) | e2e |
| 002 | A node that boots by TFTP fetches its config from the boot server and comes up with it (no test yet) | e2e |
| 003 | A unikernel gets its recorded address by DHCP (no test yet) | e2e |
| 004 | The boot server's config for a lab: the gateway, itself as name server and TFTP server, the lab's domain, a lease for each node's MAC address, name records for each node, and for a `tftp` node its config's file name in option 67 | unit |
| 005 | An `http` node is sent its config's URL on the boot server in options 67 and 239 | unit |
| 006 | A node with ZTP off keeps its lease and its name and is sent no boot option | unit |
| 007 | In a lab with IPv6 the boot server announces the prefix and its IPv6 name server, and a unikernel's IPv6 address has name records (no test yet) | unit |
| 008 | Only the lab's HTTP configs folder is mounted where the boot server serves HTTP from | unit |
| 009 | The TFTP root and the DHCP leases are outside the HTTP root | unit |
| 010 | The boot server's own folder on the server holds only its config | unit |
| 011 | A redeploy onto `tftp` gives the node its boot option, and the other nodes' entries are as they were | unit |
| 012 | A redeploy off `tftp` removes the node's boot option and keeps its lease | unit |
| 013 | A redeploy onto and off `http` adds and removes the node's two boot options | unit |
| 014 | After a redeploy the boot server's entries are in the order up wrote them: containers, VMs, then unikernels, a container with no MAC address and a unikernel with its IPv6 address | unit |
| 015 | A node with no recorded address has no entry | unit |
| 016 | The boot server's config is written again only when it changed | unit |
| 017 | A redeploy that changes a node's boot options recreates the boot server, and one that does not leaves it running (no test yet) | integration |
| 018 | A redeploy onto another method removes the node's TFTP and HTTP files, and no other node's | unit |
| 019 | A redeploy keeps the files of the method the node has | unit |
| 020 | After the boot server is recreated, the lab's nodes keep their addresses and resolve each other (no test yet) | e2e |

## `config_privacy`: a node's config on the server is read only by Sherpa

**Statement:** every config Sherpa writes for a node on the server is readable by the `sherpa` account alone, whatever the server's umask, and is removed once nothing reads it.
**Relied on by:** the user, whose logins and custom configs are in it, against the other accounts on the server. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | Under a permissive umask, a VM's boot folder and every folder in it are 0700 and every file in it 0600, config ISOs and disk images among them, for a VM of each of `cloud-init`, `cdrom`, `nocloud` and `ignition` | unit |
| 002 | Under a permissive umask, the HTTP configs folder is 0700 and its files 0600, and a TFTP config is 0604, which other accounts read and the group does not | unit |
| 003 | Under a permissive umask, a container's config folders are 0700 and its config files 0600 | unit |
| 004 | A VM's boot files are removed once its media are in the storage pool | integration |
| 005 | A VM's boot files are kept when its media cannot be put in the storage pool | integration |
| 006 | A redeploy removes the node's own folders, and no other node's | unit |
| 007 | A redeploy of a node named like one of the lab's own folders keeps that folder | unit |
| 008 | A redeploy of a VM named like one of the lab's own folders still removes its boot files | unit |
| 009 | A file written again takes the mode asked for and the new contents, whatever mode it had | unit |
| 010 | Writing a file leaves its folder's mode alone | unit |
| 011 | The folders made on the way to a file or a folder are private, and a new folder gets the mode asked for | unit |
| 012 | An existing folder is set to the mode asked for, its setgid bit cleared, and keeps its contents | unit |
| 013 | The TFTP and HTTP configs of a lab are gone from the server once the lab is destroyed (no test yet) | integration |
| 014 | An OS account outside `sherpa` cannot read a running lab's node configs on the server (no test yet) | integration |

The configs of `tftp` and `http` nodes are served on the lab's management network without credentials, to anything that reaches the boot server. That is by design and documented; no case asserts it.
