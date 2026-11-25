# Install and update

Putting Sherpa on a host, keeping it up to date and taking it off: the install, update and uninstall scripts, `sherpad init`, what sherpad makes when it first starts, the modes of the server's files, the database's schema and what a database, a lab, a `sherpa.toml` and a lab folder an older release wrote become under this one, the database backup, the `sherpad` commands that run the server process, and what a client needs before its first command: its install, its config, the server's certificate and the version check.

Other areas own what the installed system does once it runs:

- The built-in `admin` account at startup (sherpad refuses to start without it, or creates it from `SHERPA_ADMIN_PASSWORD`) is `builtin_admin_001` to `003` in [accounts-and-auth.md](accounts-and-auth.md). Its creation by `sherpad init`, with the first user, is `server_init` here. Who may ask for a backup is `admin_only`; the backup itself is `database_backup` here.
- The login token secret, made once at first start and private, is `user_sessions_023` and `024`. The database key is `database_key` here; a device password sealed with it is `device_password_011`.
- A management prefix too small for a lab, which `sherpad init` refuses, is `lab_addressing_008` in [lab-lifecycle.md](lab-lifecycle.md).
- The `[tailscale]` table of `sherpa.toml` is `tailnet_settings` in [tailnet.md](tailnet.md). The rest of the file is `server_config` here.
- A node an earlier release defined with its consoles on TCP is `console_upgrade` in [consoles.md](consoles.md).
- The current layout of a lab folder is [client-lab-files.md](client-lab-files.md). Moving an older layout into it is `lab_folder_upgrade` here.
- Importing, scanning and choosing images is [images.md](images.md).
- A node state, a lab status or an error code this release does not know reads as unknown: `node_state_015`, `lab_status_007`, `api_errors_005`.
- `/health` and `sherpa server status` are `server_status` in [inspect-and-web-ui.md](inspect-and-web-ui.md).
- The lab CA and the certificates of nodes are `config_delivery` in [ztp.md](ztp.md); the server's own certificate is `server_certificate` here.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `update_script_007_a_release_that_refuses_the_database_is_not_installed`. The level is the highest that can show the case:

- **e2e (fresh VM):** the scripts run as root on a bare Ubuntu guest that `tests/harness/test_install.sh` builds on the host and destroys. The install, reboot, uninstall and update cases need it: nothing less has systemd, Docker, libvirt and a clean `/opt/sherpa`. An update case needs two releases, an earlier one to install and this one to update to.
- **e2e (Windows):** the client install script on a Windows guest. Nothing runs one yet.
- **integration:** a real database (in memory, or a file in a temporary folder), real libvirt, or a running sherpad.
- **unit:** logic with many branches. **unit (bats):** one function of a script, sourced with its commands stubbed and its paths in a temporary folder (`tests/bats/`). The self-tests of `tests/harness/vm_release_test.py` that run a script's function the same way are unit too.

## Where things are

| Path | Holds | Mode, owner | Made by |
|---|---|---|---|
| `/opt/sherpa` | everything below | `755 root:root` | install, update |
| `/opt/sherpa/bin/sherpad`, `sherpa` | the binaries, linked from `/usr/local/bin` | folder and files `755 root:root` | install, update |
| `/opt/sherpa/config/sherpa.toml` | the server's settings | folder `750`, file `600`, `sherpa:sherpa` | install (folder), `sherpad init` (file), update |
| `/opt/sherpa/env/sherpa.env`, `sherpa.env.example` | the service's environment | folder `750`, files `600`, `sherpa:sherpa` | install, update |
| `/opt/sherpa/.secret` | the login token secret and `db.key` | folder `700`, files `600`, `sherpa:sherpa` | install, update (folder), sherpad (files) |
| `/opt/sherpa/.certs` | the server's certificate and key | folder `700`, key `600`, `sherpa:sherpa` | install, update (folder), sherpad (files) |
| `/opt/sherpa/db/sherpa.sqlite`, `db/backups` | the database and its backups | `db` `750`, `backups` `700`, files `600`, `sherpa:sherpa` | install, update, sherpad |
| `/opt/sherpa/logs/sherpad.log` | the log when sherpad is not run by systemd | folder `750`, file `640` | install, update, sherpad |
| `/opt/sherpa/labs` | each lab's files | `2750 sherpa:sherpa` | install, update, sherpad |
| `/opt/sherpa/libvirt` | the folder of the storage pool | `755 root:root` | install, update |
| `/opt/sherpa/libvirt/images` | the storage pool: VM disks and ZTP media | `2750 sherpa:kvm` | install, update, sherpad |
| `/opt/sherpa/images` | the node images | `775 sherpa:sherpa` | install, update |
| `/opt/sherpa/images/blank_disk` | the blank disk templates, which only sherpad, as `sherpa`, reads, to copy them | folder `775`, files `400`, `sherpa:sherpa` | install, update |
| `/opt/sherpa/microvm` | Cloud Hypervisor, the host kernel `vmlinuz`, `initramfs.img` and the base disks in `images` | folders `755 sherpa:sherpa`, `cloud-hypervisor` `755 root:root`, `vmlinuz` and `initramfs.img` `644 root:root` | install, update (files), sherpad (base disks) |
| `/opt/sherpa/containers`, `bins`, `run` | sherpad's container and binary folders, its PID file and the console sockets | `755 sherpa:sherpa` | install, update |
| `/etc/systemd/system/sherpad.service`, `/etc/logrotate.d/sherpad`, `/etc/tmpfiles.d/sherpa-thp.conf` | the service, log rotation, the transparent hugepages boot rule | `644 root` | install |

## `install_preflight`: the install script refuses a host or a request it cannot install, before it changes anything

**Statement:** the install script stops with a message that says what is wrong, before it installs a package, creates an account or writes under `/opt/sherpa`, when the host or the request is one it cannot install.
**Relied on by:** the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | Run by an account that is not root, it stops and says to run it with `sudo` | unit (bats) |
| 002 | On a system that is not Ubuntu, an Ubuntu older than 24.04, or one with no `/etc/os-release`, it stops and names what it found (no test yet: the four tests there run a copy of the function written in the test) | unit (bats) |
| 003 | On a CPU with neither VT-x nor AMD-V it stops and says to enable virtualization | unit (bats) |
| 004 | Without `curl` it stops and says how to install it | unit (bats) |
| 005 | With a container named `sherpa-db` on the host, it stops and gives the release to update to first, with its command; a container whose name only contains `sherpa-db`, and a host with no Docker, pass | unit (bats) |
| 006 | A listen address that is not an IPv4 address (a hostname, an octet over 255, blanks) stops it; an address in `SHERPA_SERVER_IPV4`, `0.0.0.0` included, is taken without a prompt | unit (bats) |
| 007 | `--help` prints the usage with the options and the environment variables, and exits 0 | unit (bats) |
| 008 | An unknown option, and `--version` with no value, stop it with an error | unit (bats) |
| 009 | With `SHERPA_ARTIFACT_DIR` set, no `--version`, or a folder without the sherpad archive, stops it | unit (bats) |
| 010 | On a bare host, each refusal that can be reached there (not root, a bad address, missing archives, an unknown option) leaves no `/opt/sherpa` and no `sherpa` account | e2e (fresh VM) |

## `install_host`: the install script leaves a bare host ready for `sherpad init`

**Statement:** on a bare Ubuntu 24.04 or later host with KVM, the install script installs Docker from its signed apt repository, libvirt and QEMU, the `sherpa` account, the release's binaries, the router image by its digest, the service and its environment file, and running it again on an installed server keeps the server's settings and data.
**Relied on by:** the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpad` and `sherpa` are the release's binaries, `755` and owned by root, linked from `/usr/local/bin`, and each reports the version asked for | e2e (fresh VM) |
| 002 | Docker and libvirt are running; the `sherpa` account exists, cannot log in, and is in the `libvirt`, `kvm` and `docker` groups; the user who ran `sudo` is in the `sherpa` group | e2e (fresh VM) |
| 003 | The service is installed and enabled but not started: it runs `sherpad start --foreground` as `sherpa` with its four capabilities, needs Docker and libvirt, and keeps `/run/sherpa` `0700` across restarts; `sherpad.log` is rotated daily | e2e (fresh VM) |
| 004 | `sherpa.env` holds the listen address and the ports given in `SHERPA_SERVER_IPV4`, `SHERPA_SERVER_WS_PORT` and `SHERPA_SERVER_HTTP_PORT`, with 3030 and 3031 for ports left out | e2e (fresh VM) |
| 005 | The router image is in Docker under the name sherpad runs it by | e2e (fresh VM) |
| 006 | Transparent hugepages are `madvise`, or `always` where the host had it, and the boot rule keeps `madvise` | e2e (fresh VM) |
| 007 | Transparent hugepages at `never` are set to `madvise` now and by a boot rule; `always` is left, with no rule; a kernel without them is skipped with a warning | unit (bats) |
| 008 | After `sherpad init` and a reboot, sherpad is running without anyone starting it and the first user logs in | e2e (fresh VM) |
| 009 | Run again on a running server, it stops sherpad, installs, and leaves `sherpa.toml`, the server's key, the database and its key as they were: after a start, the first user logs in | e2e (fresh VM) |
| 010 | Without `--version` it installs the latest release GitHub lists (no test yet) | e2e (fresh VM) |
| 011 | The closing message gives the `sherpad init` command with the address and ports chosen | unit (bats) |
| 012 | An existing `sherpa` system account with another shell is given `nologin` (no test yet) | unit (bats) |
| 013 | A release whose sherpad archive cannot be fetched stops the install and names the URL; one without the `sherpa` archive is installed with a warning (no test yet) | unit (bats) |
| 016 | Docker is installed from Docker's apt repository for the host's Ubuntu release, signed by Docker's key, whose SHA-256 is checked against the one the script pins; no script is fetched to run | unit (bats) |
| 017 | A Docker key that is not the pinned one stops the install before apt trusts it: no keyring, no source and no package | unit (bats) |
| 018 | The router image is pulled by the digest the script pins and tagged with the name sherpad runs it by; a pull that fails stops the install | unit (bats) |
| 019 | Run again, it keeps `sherpa.env` line for line, an operator's lines and the settings it holds included, and adds at its end only the settings it writes that the file lacks; a run that finds none missing leaves the file as it was | unit (bats) |

## `blank_disks`: the scripts make each blank disk template that is missing and never change one that is there

**Statement:** the install script and every run of the update script make each blank disk template that is missing from `/opt/sherpa/images/blank_disk`, with the size, file system and label the nodes that boot from a copy need, and leave an existing file as it is.
**Relied on by:** nodes whose config or data disk is a copy of one (`config_delivery` in [ztp.md](ztp.md)); the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After an install on a bare host, each of the 14 templates is there with its size, file system and label, `400 sherpa:sherpa`, in a folder `775` | e2e (fresh VM) |
| 002 | Each template is made with its size, file system and label, `400`, and no temporary file is left | unit (bats) |
| 003 | A template, a folder and even a dangling link of a template's name that are there keep their contents, mode and owner, and the missing templates are made beside them | unit (bats) |
| 004 | A second run makes nothing and touches no file | unit (bats) |
| 005 | With a tool missing, nothing is made, not even the folder, and the error names the tools and the packages that hold them | unit (bats) |
| 006 | A template that fails to build leaves no file of its name and stops the script; the templates made before it stay | unit (bats) |
| 007 | The update script installs the tools' packages only when a tool is missing | unit (bats) |
| 008 | The update script makes the missing templates before it stops sherpad, and also when there is nothing to update; when one cannot be made, the server is left running the release it had (no test yet: `update_tests.bats` reads the order of the calls in the script's text) | e2e (fresh VM) |
| 009 | Cases 002 to 006 hold for the copy of the section in each script | unit (bats) |
| 010 | Every blank disk, and the blank disk folder, is written as `sherpa`, never as root, by both scripts: a link a member of the `sherpa` group puts in the folder gets nothing of root's | unit (bats) |

## `microvm_host_tools`: the scripts put what the microVM nodes run in on the host, and the uninstall script removes it

**Statement:** the install script and every run of the update script install the packages the microVM guest is built from, install the Cloud Hypervisor binary whose SHA-256 the script pins, build the guest initramfs from the running kernel with the files of `scripts/microvm` at the release's tag, install that kernel beside it, and make the folder and the images folder `sherpa` writes; the uninstall script removes the folder in every mode.
**Relied on by:** nodes of the privileged container models (cEOS, SR Linux); the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After an install on a bare host, `/opt/sherpa/microvm` holds `cloud-hypervisor` `755 root:root`, `vmlinuz` and `initramfs.img` `644 root:root`, and `images`, all `sherpa`'s folders `755`; the binary reports the pinned version | e2e (fresh VM) |
| 002 | Cloud Hypervisor is installed `755`, root's, from the pinned URL when its SHA-256 is the pinned one | unit (bats) |
| 003 | A download whose SHA-256 is not the pinned one stops the script and installs nothing | unit (bats) |
| 004 | The folder and the images folder are made `755` and `sherpa`'s, an existing one gets the same, and a symlink at or above the folder stops the script with the link named | unit (bats) |
| 005 | The guest build files come from the repository at the tag of the release, or from `microvm` in `SHERPA_ARTIFACT_DIR` when local artifacts are used; a file that cannot be fetched stops the script | unit (bats) |
| 006 | `busybox-static`, the running kernel's `linux-modules-extra`, `zstd` and `cpio` are installed | unit (bats) |
| 007 | `build-guest.sh` writes an `initramfs.img` that holds the init, `modules.list`, busybox, GNU tar and the kernel's modules with their `modules.dep`, installs the kernel as `vmlinuz`, both `644`, and leaves no temporary file | unit (bats) |
| 008 | The update script does all of it, before it stops sherpad and also when there is nothing to update (no test yet: `update_tests.bats` reads the order of the calls in the script's text) | e2e (fresh VM) |
| 009 | Cases 002 to 005 hold for the copy of the section in each script | unit (bats) |
| 010 | The uninstall script removes `/opt/sherpa/microvm` in every mode, and with a symlink at or above it removes nothing through it and names the link | unit (bats) |
| 011 | The installed unit, and the drop-in the update adds to an older one, make stopping sherpad stop only sherpad (`KillMode=process`), so its microVMs keep running (no test yet) | e2e (fresh VM) |

## `server_init`: `sherpad init` sets up a new server and keeps an existing one

**Statement:** `sherpad init`, run as `sherpa`, makes the server's folders, `sherpa.toml`, the libvirt network and storage pool, the database, the `admin` account and a first user, and run again it keeps the config, the accounts and the data that are there.
**Relied on by:** the admin; sherpad, which starts on none of these missing. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | After an install, `sherpad init` answered at its prompts (the `admin` password twice, a username, its password twice, administrator, no key) exits 0; sherpad then starts, the first user logs in as an admin, and the user list has them and `admin` | e2e (fresh VM) |
| 002 | Run as any account but `sherpa`, root included, it is refused, carrying the account and the command, before anything is written | unit |
| 003 | The `sherpa.toml` it writes holds the address, ports and management prefixes given, and the defaults for everything else (no test yet) | unit |
| 004 | An existing `sherpa.toml` is kept, whatever is given on the command line; `--force` writes it again (no test yet) | integration |
| 005 | With `SHERPA_ADMIN_PASSWORD`, `SHERPA_USER_NAME`, `SHERPA_USER_PASSWORD`, `SHERPA_USER_IS_ADMIN` and `SHERPA_USER_SSH_KEY` set it asks nothing; an empty `SHERPA_USER_PASSWORD` is refused (no test yet) | integration |
| 006 | A yes or no answer is `y`, `yes`, `n` or `no` in any case, the default when empty, and anything else is refused | unit |
| 007 | The SSH key answer is the key itself, a path to a file that holds it, or nothing | unit |
| 008 | Two passwords that differ, and an empty password, are refused (no test yet) | unit |
| 009 | Run again, it leaves `admin`, its password and the users that exist, and asks for no first user (no test yet) | integration |
| 010 | It makes the libvirt network `sherpa-bridge` on the bridge `br-sherpa0`, active | integration |
| 011 | It makes the storage pool with its folder, active | integration |
| 012 | A storage pool that is there is left as it is | integration |
| 013 | A storage pool that is defined but stopped is started | integration |
| 014 | The network and the pool start with libvirt after a reboot (no test yet) | integration |

## `server_file_modes`: the server's files are readable only by the accounts that need them

**Statement:** the files and folders of the table above have its modes and owners, however the scripts and sherpad were run and whatever an older release left there: the settings, the secrets, the database, the logs and the lab data are closed to every other account on the server, and `sherpa.toml` and the env files to the `sherpa` group too; what systemd and the scripts run is root's, so neither `sherpa` nor its group can change it, and neither the scripts nor `sherpad doctor` change anything as root through a symlink.
**Relied on by:** the admin; every user, whose password hashes, device password and lab data are in them. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | After an install and `sherpad init` on a bare host, each path of the table has its mode and owner | e2e (fresh VM) |
| 002 | `sherpa.toml`, `sherpa.env` and `sherpa.env.example` can be read by `sherpa`, and not by the user who installed, a member of the `sherpa` group, nor by `nobody` | e2e (fresh VM) |
| 003 | `sherpad init` writes `sherpa.toml` `0600` under a umask of `0000`, `0002`, `0022` and `0077`, and a wider file it replaces gets `0600` too | unit |
| 004 | sherpad at start makes a missing lab, database, backup, storage pool or console folder with its mode | unit |
| 005 | sherpad at start narrows such a folder that is wider, and gives it its group | unit |
| 006 | A folder sherpad cannot narrow is logged as a warning and sherpad still starts (no test yet) | unit |
| 007 | A new database file, and the write-ahead log beside it, are `0600` from their first write | integration |
| 008 | A database file that is wider is `0600` once it has been opened | integration |
| 009 | `sherpad.log` is made `0640`, and a wider one is narrowed with its contents kept | unit |
| 010 | The update script gives `sherpa.toml`, the env files, `sherpad.log` and its rotated copies their modes and `sherpa` as owner, setting the mode as `sherpa`, and changes no file's contents | unit (bats) |
| 011 | The update script skips one of these files that is missing, and creates none | unit (bats) |
| 012 | The update script leaves a symlink in the place of one, or in the place of its folder, and the link's target, as they are, and says so | unit (bats) |
| 013 | The install and update scripts make `/opt/sherpa`, `bin` and `libvirt` root's and `755`, the `775 sherpa:sherpa` an older release left included; they give the config, env, log and database folders `750`, and the lab folder and the storage pool `2750`, every run; and they make `.secret` and `.certs` `700`, the images folder `775`, and `containers`, `bins` and `run` `755`, all `sherpa`'s, when missing, leaving one that is there | unit (bats) |
| 014 | The update script does all of this when there is nothing to update, too (no test yet: `update_tests.bats` reads the order of the calls in the script's text) | e2e (fresh VM) |
| 015 | `sherpad doctor --images` sets every folder under the images folder to `775`, every blank disk template to `400` and every other file to `660` | unit |
| 016 | Nothing in those folders changes owner or mode: the scripts change the folders only, never what is in them | unit (bats) |
| 017 | A symlink at `/opt/sherpa`, at one of those folders, or at a folder above them stops the script with the link named, and the link's target keeps its owner and mode | unit (bats) |
| 018 | Both scripts install a binary `755`, root's, in place of the file or the link at its name, and never write through the link | unit (bats) |
| 019 | After an install and `sherpad init`, the user who installed, a member of the `sherpa` group, can write nothing in `/opt/sherpa` but the images and blank disk folders, and `sherpa` cannot write `/opt/sherpa`, `bin`, the binaries, `libvirt` or the service's unit | e2e (fresh VM) |
| 020 | `sherpad doctor --images` skips a symlink under the images folder, leaving the link's target's mode as it is, names the link, and fixes the rest | unit |

## `database_key`: the database's secrets open only with the key sherpad made for it

**Statement:** sherpad seals the database's secret values with a 32-byte key it makes once, in `/opt/sherpa/.secret/db.key` and never in the database, and it refuses to start without the key the database's values were sealed with, never replacing it.
**Relied on by:** every user, for their device password; the admin, whose backup holds only ciphertext without the key. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The first start makes the key file, `0600`, and later starts load the same key | unit |
| 002 | With sealed values in the database and no key file, sherpad is refused, carrying the file to restore, and makes no new key | unit |
| 003 | A key file that is not 32 bytes long is refused as damaged, carrying the file, and left as it is | unit |
| 004 | The database records the key at its first use: the same key passes from then on, another key is refused as not this database's, and a database opened with no key is refused | integration |
| 005 | A key prints as `SecretKey(..)`, never its bytes | unit |
| 006 | The same value sealed twice gives two ciphertexts | unit |
| 007 | After the database and the key are restored together on a new install, sherpad starts and a user's device password still opens (no test yet) | e2e (fresh VM) |

## `server_certificate`: sherpad renews its own certificate on every start, or serves the admin's and says what is wrong with it

**Statement:** with TLS on and `self_generate_cert` on, sherpad keeps its key in `/opt/sherpa/.certs` (making a new one only when there is none that loads) and writes a new self-signed certificate for it on every start, valid for `cert_validity_days` and naming the server's current addresses, each file written whole or not at all; with `self_generate_cert` off it never writes either file, does not start without both, and warns at start and in `sherpad status` about a certificate that expires within 30 days or does not name the server's addresses; either way it logs the key pin a client is asked to check.
**Relied on by:** the admin, who gives users the key pin; `certificate_trust`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A start with no certificate writes the certificate and its key, the key `0600` | unit |
| 002 | The certificate names the `san` list of `sherpa.toml`, or without one the server's listen addresses (every interface's when it listens on all), the loopback names and its hostname (no test yet) | unit |
| 003 | The key pin sherpad logs at start is the one the client shows for that certificate | unit |
| 004 | Retired: a start with a certificate and key there no longer serves them unchanged; see 009 | — |
| 005 | With `self_generate_cert` off and the certificate or the key missing, sherpad does not start, carrying the missing file | unit |
| 006 | Every certificate of a file that holds a chain is served | unit |
| 007 | A certificate file with no certificate in it is refused, carrying the file | unit |
| 008 | `GET /cert` on the HTTP port answers with the certificate, without a login (no test yet) | integration |
| 009 | Each start writes a new certificate with the same key, so its key pin does not change | unit |
| 010 | A start with no key file makes a new key, and a certificate for it | unit |
| 011 | A key file that does not load is replaced by a new `0600` key | unit |
| 012 | A start after the server's address changed writes a certificate naming the new address, not the old one | unit |
| 013 | With `self_generate_cert` off, a start leaves the certificate and key byte for byte as they were | unit |
| 014 | With `self_generate_cert` off, a certificate that expires within 30 days is served with a warning at start | unit |
| 015 | With `self_generate_cert` off, a certificate that does not name the server's address is served with a warning at start naming the address | unit |
| 016 | With `self_generate_cert` off, `sherpad status` reports the same warnings | unit |
| 017 | Each certificate sherpad writes for its kept key has a serial number of its own | unit |

## `schema_migration`: sherpad brings the database to its schema in one step, or leaves it as it was

**Statement:** when sherpad starts, and in `sherpad init`, a database at any earlier schema version is brought to this release's version in one transaction, and a database this release cannot use is refused with the reason and left exactly as it was; `sherpad check-database` gives the same answer without changing anything.
**Relied on by:** the admin; the update script, which asks before it installs a release. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A new database gets this release's schema version, and a second start changes nothing | integration |
| 002 | A database with a newer schema version than this release's is refused, the error carrying both versions, and is left unchanged | integration |
| 003 | `check-database` refuses a newer schema version the same way | integration |
| 004 | A database with a lab that logs in to its nodes with the server's shared SSH key is refused; the steps before that one are undone too, so its version and its records are as they were | integration |
| 005 | That refusal carries each such lab's ID, name and owner | unit |
| 006 | `check-database` refuses such a database the same way, and the database is unchanged | integration |
| 007 | `check-database` passes an empty database, an older one with no such lab and one at this release's version, and none of them changes | integration |
| 008 | `sherpad check-database` on a server with no database file passes and creates none | integration |
| 009 | `sherpad check-database` on a database file at this release's version passes, and the file is as it was | integration |
| 010 | A step that fails part-way leaves the database at the version it had, with its records (no test yet: only the refusal of 004 shows it) | integration |

The whole path, a server set up by an earlier release that starts under this one with its users, is `update_script_001`.

## `upgraded_records`: what an older release stored means the same after the upgrade

**Statement:** after the upgrade every lab, node, image and user an older release stored is still there, each value reads as what this release means by it, and only values this release has no use for are gone.
**Relied on by:** the admin and every user of a server that is updated. **Priority:** P1.

Each case builds a database at the schema version that stored the value, upgrades it and reads it with this release's code.

| Case | What it shows | Level |
|---|---|---|
| 001 | A lab with a stored status is `ready`, with its records | integration |
| 002 | A lab stored with no definition has none, and keeps its description and its nodes' settings | integration |
| 003 | Labs that hold values this release does not keep (a key of their own, the provisioning login setting, a loopback network) keep their records, their definitions and their nodes | integration |
| 004 | A node recorded `created` is `stopped`; a node in any other state keeps it | integration |
| 005 | A VM node with no ready check gets port 22, a Talos node 50000, and a container or unikernel its model's; none is skipped | integration |
| 006 | A node with no recorded login records `sherpa`, and a node with one keeps it | integration |
| 007 | CPU pins stored on nodes are each node's pins, CPUs two nodes shared included | integration |
| 008 | Nodes that hold values this release does not keep (an import mark, a VNC port) keep their records and can be changed | integration |
| 009 | A user keeps their account and has no device password | integration |
| 010 | A stored definition loses the SSH keys and key files of its nodes and keeps every other list | integration |
| 011 | Images of Cisco CSR 1000v and IOSvL2 at 16 data interfaces have 15; another model's 16 is kept | integration |
| 012 | SONiC images at 52 data interfaces have 32; a count an admin set, and another model's 52, are kept | integration |
| 013 | A model and kind with images and no default gets its newest as default; a default that is there is kept | integration |
| 014 | A model and kind with two defaults keeps the newest of them (no test yet) | integration |
| 015 | Every pair of the old ZTP switch and method, for every model, becomes one ZTP setting the model takes, and the pair a release shipped becomes the model's default | integration |
| 016 | An image of a model this release does not know has ZTP off | integration |
| 017 | An SRX image on TFTP is on CD-ROM; its other settings, and another model on TFTP, are kept | integration |
| 018 | A generic VM image's CD-ROM bus of `virtio` is `sata`; another model's is kept | integration |
| 019 | Images, and the nodes of stored definitions, under an earlier model name have the model and kind of today, and each default is kept | integration |
| 020 | Each image keeps the settings of its own kind, as shipped and as an admin edited them | integration |
| 021 | A unikernel image with no boot mode boots its kernel directly, and values an image held for another kind are gone | integration |
| 022 | Images that hold values this release does not keep (a dedicated management interface, a ZTP login) keep their settings and their default, and can be edited | integration |
| 024 | A stored definition loses the `p2p` value of its links and keeps every link with its ends and impairment | integration |

## `update_script`: the update script installs a release over a running server, or leaves the installed one in place

**Statement:** the update script fetches the release asked for, and pulls the router image it pins, while sherpad still runs, backs up the database and its key, replaces the binaries and starts sherpad again with the settings, accounts and data the server held, and when it cannot it says why and leaves, or puts back, the installed release's binaries, database and key, with sherpad running.
**Relied on by:** the admin. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | On a server installed and set up by an earlier release, the update leaves this release's binaries in place and sherpad running; the admin and a user made before it log in, and `sherpa.toml`, `sherpa.env`, the server's key, the database key when the earlier release made one, and the users' records are as they were | e2e (fresh VM) |
| 002 | After a reboot of the updated server, the same holds | e2e (fresh VM) |
| 003 | A host with no `/opt/sherpa/bin/sherpad` is refused, pointing at the install script (no test yet) | unit (bats) |
| 004 | A server with no database file and a `sherpa-db` container, or the data folder it kept, is refused, with or without Docker, naming the release to update to first and its command | unit (bats) |
| 005 | A server with a database file passes that check whatever else is there, and so does one with neither the container nor its data | unit (bats) |
| 006 | Refusing such a server, the script has stopped nothing and the installed binary is byte for byte what it was | unit (bats) |
| 007 | A release whose sherpad refuses the database stops the update: the labs it names are shown, nothing is installed, and the installed release is started again | unit (bats) |
| 008 | After that refusal the installed sherpad binary is the one that was there | unit (bats) |
| 009 | The new sherpad's check runs as `sherpa`, and is skipped on a server with no database | unit (bats) |
| 010 | Updating to a release older than 0.6.0 skips the check and keeps the server's shared SSH key | unit (bats) |
| 011 | The server's shared SSH key pair is deleted, with its folder when that leaves it empty; another file in the folder, and a folder with no key, are kept | unit (bats) |
| 012 | A server already at the version asked for has nothing downloaded and sherpad is not stopped (no test yet) | e2e (fresh VM) |
| 013 | `--force` installs the version that is there again (no test yet) | e2e (fresh VM) |
| 014 | Without `--version` it updates to the latest release GitHub lists (no test yet) | e2e (fresh VM) |
| 015 | A service unit with no `RuntimeDirectory` gets a drop-in that adds `/run/sherpa`, `0700` and kept across restarts (no test yet) | unit (bats) |
| 016 | With `SHERPA_ARTIFACT_DIR` set, no `--version`, or a folder without the sherpad archive, stops it before anything changes | unit (bats) |
| 017 | `--help` prints the usage and exits 0; an unknown option is an error | unit (bats) |
| 018 | A sherpad archive that cannot be fetched stops the update, naming the URL, before sherpad is stopped: the installed binaries are in place and sherpad keeps running | unit (bats) |
| 019 | An installed binary is run for its version only when it is a file of root's that no other account can write; one of another owner, or a link, is never run, and is replaced | unit (bats) |
| 020 | A symlink at or above the folder of the server's shared SSH key removes nothing, and says so | unit (bats) |
| 021 | With sherpad stopped and before the new release runs, the database, its WAL and shared-memory files when there, and the key are copied, read as `sherpa`, to a new folder in `/var/backups/sherpa`, `0700` and root's, files `0600`, and the update names the folder | unit (bats) |
| 022 | A failure after sherpad is stopped and before the binaries are replaced, a backup that cannot be written among them, stops the update with nothing installed, and starts the installed release again | unit (bats) |
| 023 | A new release that does not start is undone: its binaries are replaced by the installed release's, the database and key are restored from the backup with the WAL and shared-memory files the new release left removed, the installed release is started, and the backup is named | unit (bats) |
| 024 | sherpad counts as started only when it is active for ten seconds and systemd does not start it again in that time | unit (bats) |
| 025 | After the update of a server set up by an earlier release, the folder it names holds the database, root's only, and the key, when the server had one, is the server's | e2e (fresh VM) |
| 026 | The update installs each binary root's, `0755`, in place of the file or link at its name; a new release that does not start gets back only the binaries the update moved aside, each with the owner it had | unit (bats) |
| 027 | A symlink at or above `/var/backups/sherpa` stops the update before any binary is replaced, writes nothing through it, and starts the installed release again | unit (bats) |
| 028 | `--version` with no value, or followed by another option, is an error that says so and shows the usage | unit (bats) |
| 029 | The router image is pulled by the digest this release's install script pins and tagged with the name sherpad runs it by; a pull that fails stops the update | unit (bats) |
| 030 | The router image is pulled before sherpad is stopped, also when there is nothing to update, and the closing summary names it by its digest (no test yet) | e2e (fresh VM) |

## `uninstall_script`: the uninstall script removes the service and only the data it is told to

**Statement:** the uninstall script refuses, before it changes anything, while a lab runs; otherwise it stops and removes the sherpad service and what the install script put outside `/opt/sherpa`, keeps the data unless told to remove it, says before its prompt and again at the end every path, account and resource it keeps, and removes no package, container, image, network, pool or VM, and no account but `sherpa` with `--remove-all`.
**Relied on by:** the admin. **Priority:** P1.

The script finds a lab's resources as sherpad does, without sherpad running: the labs are the folders in `/opt/sherpa/labs`, named by lab ID; a VM, container or network belongs to a lab when its name is a node or network name, a hyphen and the lab's ID; a Tailscale gateway is the container labelled `sh.erpa.role=tailnet-gateway`, its lab in `sh.erpa.tailscale.lab`. A running VM is an active libvirt domain, a running container one `docker ps` lists.

| Case | What it shows | Level |
|---|---|---|
| 001 | With no option or `--keep-data`: sherpad is stopped, and the service unit, the log rotation rule, the hugepages boot rule, the `sherpad` binary and its link are gone; the database, its key, `sherpa.toml`, the server's certificate and the `sherpa` CLI with its link are as they were | e2e (fresh VM) |
| 002 | Installing again after that and starting sherpad, the same admin logs in | e2e (fresh VM) |
| 003 | `--remove-data` does the same and empties the database folder, and keeps the database key | e2e (fresh VM) |
| 004 | Installing again after that, `sherpad init` makes a new database and its first user logs in | e2e (fresh VM) |
| 005 | `--remove-all` also removes `/opt/sherpa`, `/run/sherpa` and the `sherpa` CLI's link | e2e (fresh VM) |
| 006 | In each mode the packages, the accounts and groups (`sherpa` among them, but with `--remove-all`), Docker's images, volumes and containers, and libvirt's networks, pools and domains are what they were, and Docker and libvirt still run | e2e (fresh VM) |
| 007 | Run again on a host where nothing is left, it exits 0 | e2e (fresh VM) |
| 008 | A link in `/usr/local/bin` is removed only when it points at Sherpa's binary: one to another program, and a file, are kept | unit |
| 009 | Without `--force` it asks, and any answer but `yes` removes nothing and exits 0 | unit (bats) |
| 010 | `--help` prints the usage and exits 0; an unknown option is an error; not root is refused | unit (bats) |
| 011 | A `sherpa-db` container on the host is stopped and removed | unit (bats) |
| 012 | With a symlink at or above the database or the binaries folder, nothing is removed through it, and the link is named | unit (bats) |
| 013 | With a lab's VM running, each mode, with and without `--force`, exits 1 before its prompt, names the lab ID and the node, says to run `sherpa destroy`, and changes nothing; another domain is not taken for a lab's | unit (bats) |
| 014 | The same with a lab's container running; another container is not taken for a lab's | unit (bats) |
| 015 | The same with a lab's Tailscale gateway running | unit (bats) |
| 016 | With a lab of one Alpine container running on the installed server, each mode with `--force` exits non-zero, names the lab and the node, and the service, the files and Docker's and libvirt's resources are as they were | e2e (fresh VM) |
| 017 | With `--remove-all`, the domains, containers, libvirt and Docker networks and gateway volumes of stopped labs, and the storage pool, are listed by name before the prompt and in the closing summary, and resources of no lab are not | unit (bats) |
| 018 | With `--keep-data`, the prompt and the closing summary name what is kept and where: the database, `.secret`, `.certs`, `sherpa.toml`, `sherpa.env`, the lab folders, `/run/sherpa`, the update backups, the `sherpa` account | unit (bats) |
| 019 | With `--remove-data`, the same but the database, which is named as removed | unit (bats) |
| 020 | With `--remove-all`, the removed list names the update backups, and the kept list names the packages and none of what is removed | unit (bats) |
| 021 | `--remove-all` removes the `sherpa` account and its group, and no group lists `sherpa` | e2e (fresh VM) |
| 022 | With lab folders on the host and Docker or libvirt not answering, it exits 1, says it cannot tell whether a lab runs, and changes nothing | unit (bats) |
| 023 | `--remove-all` removes `/var/backups/sherpa`; with a symlink at or above it, nothing is removed through it, and the link is named | unit (bats) |

## `database_backup`: a backup is a consistent copy only `sherpa` can read

**Statement:** `sherpa server backup` copies the database, while sherpad keeps using it, to a new file named after the time in `/opt/sherpa/db/backups` that only `sherpa` can read, and never writes over a file.
**Relied on by:** the admin, before an update. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A backup taken while the database is open is one file, `0600`, that opens with the same records, and the database goes on working | integration |
| 002 | A backup to a file that exists is refused, and the file is as it was | integration |
| 003 | The file is `sherpa-<time>.sqlite`, the time in UTC to the second | unit |
| 004 | The backups folder is restricted to `sherpa` before the copy is written (no test yet) | integration |
| 005 | `sherpa server backup` prints the file's path and size (no test yet) | e2e |

## `sherpad_daemon`: the `sherpad` commands run one server process and say what state it is in

**Statement:** `sherpad start`, `stop`, `restart`, `status` and `logs` run at most one server on the host, in the background or, as systemd runs it, in the foreground, and each says truthfully whether the server is running.
**Relied on by:** the admin; systemd; the scripts. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A background start whose server exits at once is reported as failed, with its exit status; one that keeps running is reported as started | unit |
| 002 | The PID file holds the server's PID, a missing one reads as no server, and removing it twice is no error | unit |
| 003 | A PID file that names 0, 1, a negative number or no number is refused, so `stop` never signals a process group or `init` | unit |
| 004 | A process counts as running until it has exited | unit |
| 005 | SIGTERM stops the server | unit |
| 006 | SIGINT stops the server | unit |
| 007 | A second `start` while a server runs is refused (no test yet) | integration |
| 008 | `stop` ends the server and removes the PID file; with a PID file whose process is gone it says the server is not running (no test yet) | integration |
| 009 | Without `sherpa.toml`, or with one that does not parse, sherpad does not start and says which file; started in the background, the error is in `sherpad.log` (no test yet) | integration |
| 010 | A server that fails is started again by systemd (no test yet) | e2e (fresh VM) |
| 011 | A server holds an exclusive lock on the PID file while it runs, so a second start fails at the lock, carrying the PID; once the lock is let go, a start takes it | unit |
| 012 | A PID file whose process is not `sherpad`, by the program name it was started as, reads as no server: `stop` signals nothing and removes the stale file; one whose process is `sherpad` reads as running | unit |

## `server_config`: sherpad reads `sherpa.toml` as `sherpad init` or an older release wrote it

**Statement:** sherpad starts on a `sherpa.toml` written by `sherpad init` of this or an older release: a table left out takes its defaults and a setting it does not read is ignored.
**Relied on by:** the admin after an update. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | Retired: sherpad reads no `[ztp_server]` table | — |
| 002 | Retired: sherpad gives no warning for a `[ztp_server]` table | — |
| 003 | Retired: sherpad gives no warning for a `[ztp_server]` table | — |
| 004 | An `[otel]` table with a setting sherpad does not read loads | unit |
| 005 | A config with no `[otel]`, `[scanner]` or `[host_memory]` table loads with each one's defaults, and a table with one setting keeps the defaults of the others | unit |
| 006 | A key `[host_memory]` does not have is refused | unit |
| 007 | `SHERPA_SERVER_IPV4`, `SHERPA_SERVER_IPV6`, `SHERPA_SERVER_WS_PORT` and `SHERPA_SERVER_HTTP_PORT` in the environment override the file, and a value that does not parse is ignored with a warning (no test yet) | unit |

## `image_folder_rename`: retired

Retired: sherpad does not move image folders. Cases 001 to 009 are retired, and their numbers stay taken.

## `upgraded_labs`: a lab an older release built keeps working, and sherpad says what needs the user

**Statement:** a lab built by an older release is listed, changed and destroyed by this one, the secrets saved for it still serve it, and sherpad warns of nodes that still have a login this release gives no node.
**Relied on by:** the user after an update, the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A Keycloak node whose saved credentials have no administrator of its own gets one at its next start, with a new password, and the saved file no longer holds the user `sherpa` or its password | unit |
| 002 | A lab CA saved with its subject in another order still issues node certificates that chain to it | unit |
| 003 | At start sherpad warns once, with the count, when nodes still log in as `sherpa`, and says to redeploy them; with none it says nothing (no test of the warning yet: the test there shows the count of none) | integration |
| 004 | A lab brought up before an update is inspected, taken down, resumed and destroyed after it (no test yet) | e2e (fresh VM) |

## `lab_folder_upgrade`: the CLI moves an older client's lab files into `.sherpa` before it uses the folder

**Statement:** before a command uses a lab folder, the CLI moves the lab files an older client saved beside the manifest into `.sherpa`, private to the user, changes nothing the user wrote, and stops the command when it cannot, except `destroy`.
**Relied on by:** the user with labs set up by an older client. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The lab info, SSH config, known hosts, lab CA certificate and the Vault, Keycloak and cluster credentials move to their places in `.sherpa`, and the user's own files stay where they are | unit |
| 002 | The moved SSH config names the known hosts file at its new place, whether the old line gave it by a full path or by name, and its other lines are as they were | unit |
| 003 | The lab's `Include` in `~/.ssh/sherpa_lab_hosts` names the moved config, in the same position, and other labs' lines are untouched | unit |
| 004 | The folders made are `0700` and each moved secret is `0600`, whatever it was | unit |
| 005 | Only files the client generated are taken: by their exact names, a Vault or Keycloak file only with the keys the client wrote, and cluster credentials only for a cluster of the manifest or with both files | unit |
| 006 | A folder already in the `.sherpa` layout is left exactly as it is | unit |
| 007 | A move that stopped part-way is finished at the next command, and a loose file replaces the one already in `.sherpa` | unit |
| 008 | An `Include` still at the old path after the files have moved is corrected | unit |
| 009 | An `Include` present at both paths is left once, at the new one | unit |
| 010 | With no `Include` for the lab, or no `sherpa_lab_hosts`, nothing in `~/.ssh` is written | unit |
| 011 | A folder that is not a lab's, even one with files named like credentials, is untouched and gets no `.sherpa` | unit |
| 012 | A `.sherpa` that is a symlink is refused, and neither its target nor the loose files change | unit |
| 013 | `sherpa ssh-config` moves the files of each lab folder its index includes by the old config's name, and of no other | unit |
| 014 | When the move fails, every command that uses the lab folder stops with the reason, and `destroy` goes on | unit |
| 015 | The failure is reported once, as a failure to move the lab files | unit |
| 016 | `destroy` after a failed move removes the lab's files, the loose ones too, and its line in `~/.ssh`, and leaves the user's own files | unit |
| 017 | `destroy` after a failed move still refuses a `.sherpa` that is a symlink | unit |
| 018 | In a folder an older client left, `sherpa inspect` says which files moved and then prints the lab (no test yet) | e2e |

## `client_install`: the Windows install script puts one `sherpa.exe` in the user's own folder

**Statement:** `sherpa_client_install.ps1` installs the release asked for, or the latest, as `$HOME\.sherpa\bin\sherpa.exe` without administrator rights, puts that folder on the user's `PATH` once, and replaces an existing install only when told to.
**Relied on by:** the user on Windows. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A first install puts `sherpa.exe` in place, adds its folder to the user's `PATH`, and `sherpa --version` reports the release (no test yet) | e2e (Windows) |
| 002 | `-Version` is taken with or without the leading `v`; without it the latest release is installed (no test yet) | e2e (Windows) |
| 003 | Over an existing install it asks, and any answer but yes or Enter leaves the installed version (no test yet) | e2e (Windows) |
| 004 | `-Update` with no install is refused, at the version asked for does nothing, and otherwise replaces `sherpa.exe` without asking and leaves `PATH` alone (no test yet) | e2e (Windows) |
| 005 | A release that cannot be downloaded leaves the installed version and exits 1 (no test yet) | e2e (Windows) |
| 006 | A second install does not add the folder to `PATH` again (no test yet) | e2e (Windows) |

## `client_config`: the CLI finds its server from its own config, or from the command line

**Statement:** the CLI reads the server's address, ports and TLS settings from `~/.sherpa/config/sherpa.toml`, which `sherpa init` writes, uses its defaults when there is no file, and says which file is wrong when it does not parse.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | With no config file the CLI uses its defaults | unit |
| 002 | A config file that does not parse is an error that carries the file | unit |
| 003 | The server URL is `wss://<address>:<port>/ws` with TLS on, and `ws://` with it off | unit |
| 004 | The port the certificate is fetched from is the one in `[server_connection]` | unit |
| 005 | `sherpa init` writes the address and port answered, and leaves an existing file unless `--force` is given (no test yet) | e2e |
| 006 | `--server-url` and `SHERPA_SERVER_URL` take the place of the file's address for one command (no test yet) | e2e |

## `certificate_trust`: the CLI talks only to a server whose certificate it has reason to trust

**Statement:** the CLI connects over TLS only when the server's certificate was issued by a public CA, by the CA file the user configured, or carries the public key of the certificate the user accepted for that server after being shown its key pin (the SHA-256 of that key), so a certificate the server made again with the same key is trusted without a question; `--insecure` alone skips the check, and asks and saves nothing.
**Relied on by:** every user, for the password and the secrets that cross the connection. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The first connection to a server with a self-signed certificate shows its subject and key pin and asks; after yes the command runs, and the next command asks nothing (no test yet) | e2e |
| 002 | After no, the command fails, nothing is saved, and the next command asks again; with no terminal to ask on it fails saying so (no test yet) | e2e |
| 003 | A certificate no public CA issued is refused, and marked as the kind the user is asked about | unit |
| 004 | With no config and no `--insecure`, the server's certificate is checked | unit |
| 005 | `--insecure` connects without asking about, fetching or saving a certificate | unit |
| 006 | `--insecure` accepts each signature scheme the TLS library can check | unit |
| 007 | A certificate issued by the configured CA file is trusted without a question | unit |
| 008 | The certificate to show is fetched from the server's HTTP port, not the port the CLI connects to | unit |
| 009 | The trust store is made `0700` | unit |
| 010 | A saved certificate is read back for its server, and listed | unit |
| 011 | A removed certificate is gone, and removing it again says there was none | unit |
| 012 | Each server and port has its own entry, and a URL with no port is stored under its scheme's default | unit |
| 013 | Text that is not a certificate is not saved | unit |
| 014 | A self-signed certificate is shown as self-signed, with its issuer | unit |
| 015 | `sherpa cert` takes a host, a host and port, or a `ws://` or `wss://` URL, and a host alone gets the server's port, so it names the entry a connection uses | unit |
| 016 | `sherpa cert` refuses any other URL, a path, and nothing | unit |
| 017 | `sherpa cert list`, `show`, `trust` and `delete` print, fetch and remove the entries, asking before they change one (no test yet) | e2e |
| 018 | A server that presents a certificate with another key than the one saved for it is not connected to, and the user is not asked | unit |
| 019 | A server that presents a new certificate with the key of the one saved for it is connected to without a question | unit |

## `client_version`: a CLI of another version than the server's says so and carries on

**Statement:** when the CLI connects, it compares its version with the one the server sends, prints a warning with both when they differ, and runs the command either way.
**Relied on by:** the user after the server is updated. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A server of another version gets `WARNING: Version mismatch` with both versions on standard error, and the command's result is unchanged (no test yet) | integration |
| 002 | A server of the same version gets no warning (no test yet) | integration |
| 003 | A first message that is not the server's greeting does not stop the command (no test yet) | unit |
