# Provisioning

What Sherpa puts on a node beyond its first config, and how. Two ways, on two sides:

- **At first boot, by the server.** A manifest's `text_files`, `startup_scripts`, `systemd_units` and `environment_variables` are read by the `sherpa` CLI on the user's machine, sent to the server, and carried to a virtual machine in its cloud-init, cloudbase-init or Ignition config: `text_files` and `startup_scripts` on cloud-init and Windows nodes, `text_files` and `systemd_units` on Flatcar. A node whose config does not carry one of the three is refused (`node_settings_014` to `019` in [manifests.md](manifests.md)). The server keeps their contents with the lab, so they are not for secrets.
- **Once the node is up, by provisioners.** The `ssh` provisioner is run by the CLI itself, over its own SSH connection to a Linux or BSD virtual machine: it copies files and env files and runs scripts as the node's owner, and nothing of them reaches the server. The `container` provisioner's secret files are sent to the server outside the manifest, kept in its memory and mounted in the container at `/run/secrets` before it starts.

How a first config reaches a node, and the certificates in it, are `config_delivery` in [ztp.md](ztp.md). The owner's account, which every step here runs as or writes for, is `owner_login` in [accounts-and-auth.md](accounts-and-auth.md). A container's `environment_variables` are `container_run_002` and `003` in [nodes.md](nodes.md). How the CLI reaches a node for a provisioner, directly or with `--ssh-jump`, a node with no route, and what it may ask for, are `ssh_access` in [client-lab-files.md](client-lab-files.md); the lab's known hosts are that area's too. Who may provision is `access_control_013` and `016`, and that the server opens no path a manifest names is `access_control_018`. Reading and validating these settings in `manifest.toml`, a provisioner's fields included, belongs to Manifests. The definition of a lab the server stores, which names a provisioner's paths and never its contents, and a redeploy from the web UI, which has no files to send, belong to Lab lifecycle.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `provisioner_env_008_an_error_never_shows_a_value`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real libvirt, Docker and the database, unit only for logic with too many branches to cover above.

## `client_content`: the CLI reads what a manifest names for a node, on the user's machine

**Statement:** at up and at redeploy the `sherpa` CLI reads every file a manifest names for a node's first boot and every `$VAR` in its `environment_variables` on the user's machine, and sends their contents and values; a manifest that lists files without their contents is refused by the server.
**Relied on by:** the user, whose paths and variables are their own machine's; the server, which opens no path a manifest names. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A path starting with `~` is under the user's home, a relative path starts at the manifest's folder, and an absolute path is as written | unit |
| 002 | A text file's contents are sent with its destination, owner, group and mode | unit |
| 003 | A text file that cannot be read is refused, naming the node | unit |
| 004 | Startup scripts are read and sent, each under its file name | unit |
| 005 | A script that is empty is refused, naming the node | unit |
| 006 | A systemd unit is sent with its name, its contents and whether it is enabled | unit |
| 007 | Retired: binary files are no longer a setting | — |
| 008 | An `environment_variables` value that is `$NAME` is sent as the value `NAME` has where `sherpa` runs, and any other value as written | unit |
| 009 | A `$NAME` that is not set where `sherpa` runs is refused, naming the variable and the node | unit |
| 010 | An `environment_variables` entry that is not `KEY=VALUE` is refused, naming the node | unit |
| 011 | An up whose manifest lists files with no contents, as one sent to the REST API is, is refused naming the node and the setting, and says to use `sherpa up` (no test at this level yet) | integration |
| 012 | A systemd unit sent under a name that is not a plain file name, a path, `.`, `..` or one starting with `-`, is refused | unit |
| 013 | After a file a manifest names is changed, `sherpa redeploy` rebuilds the node with the new contents (no test yet) | e2e |

A node's `ztp.config` is read the same way: `custom_config_002` to `004` in [ztp.md](ztp.md).

## `boot_files`: a manifest's files are on a virtual machine when its first boot ends

**Statement:** a virtual machine's `text_files` are on the node when its first boot ends, each at its destination with its contents, its owner, its group and its mode, and a Flatcar node also has its `systemd_units`.
**Relied on by:** the user, startup scripts and provisioners that read them. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | On a cloud-init VM, a text file in the owner's home belongs to the owner and their group and has its mode (no test with its ID yet; `tests/labs/nixos-linux` shows it) | e2e |
| 002 | On a cloud-init VM, a text file under folders of the owner's home that did not exist has the client file's contents, and the folders are the owner's (no test yet) | e2e |
| 003 | cloud-init is given each text file as a file only root reads, outside its destination, and no contents in a command | unit |
| 004 | Once the node's users exist each file is moved to its destination, a `~/` one under its owner's home as the node's `passwd` gives it, with missing folders made and its mode set, and nothing staged is left | unit |
| 005 | A missing folder is given to the file's owner only when it is inside their home; one outside stays root's | unit |
| 006 | A file for a user the node does not have is not written, and nothing is given to that user | unit |
| 007 | A node with no text files and no environment variables is given no file and no command for them | unit |
| 008 | On Windows, a file whose mode gives others no access is limited to Administrators and SYSTEM, named by SID, and a file others may read is left with its folder's access | unit |
| 009 | On Windows, a file whose path has `"` or `%` is refused | unit |
| 010 | On Flatcar, the missing folders of a text file under its owner's home are that user's, and a file outside the home or one of root's gets none | unit |
| 011 | On Flatcar, a text file is an Ignition file with its contents, owner, group and mode (no test yet) | unit |
| 012 | On Flatcar, a systemd unit is given with its contents and whether it is enabled | unit |
| 013 | Retired: binary files are no longer a setting | — |
| 014 | A Flatcar node has its text file and its unit enabled (no test yet) | e2e |
| 015 | A Windows node has its text file at its path, and one written with mode `600` is not read by an account outside Administrators (no test yet) | e2e |

## `boot_scripts`: a manifest's scripts run at the end of a virtual machine's first boot

**Statement:** a cloud-init virtual machine's `startup_scripts` run as root, each through its own shebang, after its files are in place; a Windows node runs its `startup_scripts`.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | On a cloud-init VM a startup script ran as root (no test yet) | e2e |
| 002 | The scripts are written executable under Sherpa's startup script folder and run directly, never through `bash`, in the order the manifest lists them | unit |
| 003 | Retired: user scripts are no longer a setting; an ssh provisioner's `scripts` run as the owner | — |
| 004 | The steps of a first boot are in this order: files moved into place, the environment loader, startup scripts (no test yet) | unit |
| 005 | A Windows node's startup scripts are written under `C:\sherpa\startup_scripts` and run with PowerShell (no test yet) | unit |

That a Windows node disables every other account before its startup scripts run is `owner_login_024`.

## `login_environment`: a virtual machine's environment variables are set in its owner's login shells

**Statement:** a virtual machine's `environment_variables` are set in its owner's login shells, from a file only the owner reads, and nowhere else on the node; on Windows they are set for the whole machine.
**Relied on by:** the user, ssh provisioner scripts. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | On a cloud-init VM the owner's login shell has each variable, a command run without a login shell has none, and the file holding them is the owner's with mode `600` (no test yet) | e2e |
| 002 | The file exports each variable, and a value with a single quote in it is the same value once a shell has loaded the file | unit |
| 003 | The file is `.sherpa_env` in the owner's home, the owner's, mode `600`, and a shell that loads it has the variable | unit |
| 004 | The line that loads the file is added to the login file bash reads, in the home the node's `passwd` gives: `.bash_profile` when it is there, `.profile` when not | unit |
| 005 | For a user the node does not have, no login file is written and nothing is given to anyone | unit |
| 006 | On Flatcar the file is private to the owner, and the loading line is appended to the owner's `.bash_profile`, which keeps what it had | unit |
| 007 | On Windows each variable is set machine-wide, and a value with a single quote in it is kept whole | unit |

## `provision_run`: provisioners run on the nodes that have them, once each node takes its owner's login

**Statement:** `sherpa up`, `sherpa redeploy` and `sherpa provision` run the ssh provisioners of the nodes that have them, several nodes at once, each once its node accepts its owner's login and has finished its first boot.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | `sherpa provision --node <node>` runs that node's provisioners again, and no other node's (no test yet) | e2e |
| 002 | `sherpa redeploy` runs the redeployed node's provisioners once it is back, and no other node's (no test yet) | e2e |
| 003 | The nodes with an ssh provisioner are the ones connected to, in manifest order, or the one named alone; a node with no provisioner, or with a container provisioner only, is not connected to | unit |
| 004 | A node named that is not in the manifest is refused, naming it | unit |
| 005 | At up, a node that did not come up with SSH is skipped, as not ready when the lab came up, and the others are provisioned; `sherpa provision` skips none | unit |
| 006 | A lab with no provisioners runs nothing and needs no SSH config in its folder | unit |
| 007 | A lab folder with no SSH config is refused before any node is connected to, saying to run `sherpa up` or `sherpa download` | unit |
| 008 | A node is connected to at the address its `<node>.<lab-id>` entry in the lab's SSH config gives, and one with no entry fails at once, asking whether it was added after the lab was created | unit |
| 009 | A node that refuses the owner's login, or the connection, is tried again: it may not have made the account yet | unit |
| 010 | The Sherpa server refusing the login, when it is the jump host, is not tried again | unit |
| 011 | A node that never accepts the login fails once the lab's ready timeout has passed, saying how long it waited (no test yet) | integration |
| 012 | A provisioner starts on a node only once its cloud-init has finished: a script finds what the node's last first-boot step wrote (no test yet) | e2e |
| 013 | Up to `provision_concurrency` nodes are provisioned at once, eight when it is not set, and a node that finishes makes room for the next (no test yet) | unit |
| 014 | A node's provisioners run in the order the manifest lists them, each its files, then its env files, then its scripts (no test yet) | e2e |

A node the CLI has no route to fails at once: `ssh_access_006`. A provisioner asks a node for a key's passphrase and never a password: `ssh_access_008`.

## `provision_report`: a provisioning run reports each node's result and fails when a node was not provisioned

**Statement:** a provisioning run shows each node's progress and script output under the node's name, ends with a table of every node's result, and exits non-zero when any node was not provisioned, leaving the lab running.
**Relied on by:** the user, scripts that run `sherpa up` and `sherpa provision`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | When one node's script fails, that node is reported `failed` with the script's exit status, the others are provisioned, the command exits non-zero saying how many of how many were not provisioned and to run `sherpa provision`, and the lab is still running (no test yet) | e2e |
| 002 | Results are listed in manifest order, whatever order the nodes finished in | unit |
| 003 | The table shows each node by its name in the lab, with its status and the detail of a failure | unit |
| 004 | Container nodes' results are reported when no node has an ssh provisioner, and one that failed fails the run with the count | unit |
| 005 | Every line a node's script prints is shown with the node's name before it | unit |
| 006 | A line that arrives in pieces is shown once, when it ends | unit |
| 007 | A last line with no newline is shown when the script ends | unit |
| 008 | Output that cannot be written, as when it is piped to a program that has closed, does not stop the run | unit |
| 009 | An up that created its lab only in part still provisions the nodes that came up, and fails as a lab only partly created (no test yet) | e2e |
| 010 | A script's output is shown while it runs, and each node ends with a `provisioned` or a `failed` line (no test yet) | e2e |

## `provisioner_files`: an ssh provisioner copies its files to the node as its owner, never more readable than their mode

**Statement:** an ssh provisioner's `files` are copied to the node as its owner, each to its destination with its mode, replacing what was there in one step, and every client file the provisioner names is read before the node is changed.
**Relied on by:** the user, for credentials their tools on the node read. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up, a file with a `~/` destination is in the owner's home on the node with the client file's contents, its mode, and the owner as its owner (no test yet) | e2e |
| 002 | Every client file of a provisioner is read before anything is done on the node, with where it goes: a file to its destination with its mode, an env file to `~/.env.d/` under its own name with mode `600`, a script as its contents | unit |
| 003 | A client file that is missing fails the node, naming the file, before the node is changed | unit |
| 004 | The missing folders on the way to a destination are made readable by the owner alone, and the file is not made by that step | unit |
| 005 | A file already at the destination keeps its contents and its mode until the copy replaces it | unit |
| 006 | A `~/` destination is under the login user's home, in the command and in the copy, and an absolute one is where it says | unit |
| 007 | A mode written as octal digits is the permission bits the file gets: `644` is `rw-r--r--` | unit |
| 008 | A copy that fails leaves the file that was at the destination as it was, and at no time during a copy is the new file more readable than its mode (no test yet) | integration |
| 009 | After the client file changes, `sherpa provision` replaces the file on the node (no test yet) | e2e |

A file removed from the manifest stays on the node. That is documented; no case asserts it.

## `provisioner_env`: an ssh provisioner's env files are exported by the owner's login shells

**Statement:** an ssh provisioner's `env_files` are copied to `~/.env.d/` on the node, readable by its owner alone, with each value that is exactly `$NAME` or `${NAME}` replaced by the value `NAME` has where `sherpa` runs, and the owner's login shells export them.
**Relied on by:** the user, for tokens that are in their own environment and must not be written to a file or sent to the server. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up, the owner's login shell on the node has an env file's variables, one of them with the value it has on the user's machine (no test yet) | e2e |
| 002 | A value that is exactly `$NAME` becomes the client's value, quoted so the node's shell reads it whole, a single quote and a `$` in it included | unit |
| 003 | `${NAME}`, a line starting with `export`, and an indented line with spaces after the value are resolved the same way | unit |
| 004 | A variable that is set and empty resolves to an empty value | unit |
| 005 | Every other line is kept as written: comments, blank lines, plain and quoted values, and a value with a reference inside it, such as `$HOME/bin:$PATH`, which the node's shell expands | unit |
| 006 | Line endings are kept, `\r\n` and a last line with none included | unit |
| 007 | A reference to a variable that is not set fails before the node is changed, naming the file, the line and the variable | unit |
| 008 | An error never shows a value | unit |
| 009 | The value is read from the environment `sherpa` itself runs in | unit |
| 010 | The line that loads the env files is added once to the login file bash reads, however many times a node is provisioned | unit |
| 011 | A login shell that reads that file exports the variables of every env file, to the programs it starts | unit |

## `provisioner_scripts`: an ssh provisioner's scripts run on the node as its owner, as from a login

**Statement:** an ssh provisioner's `scripts` run on the node as its owner, each through its own shebang, as from a login, with the manifest's `environment_variables` and the env files set, and a script that exits non-zero fails its node with that status.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | At up, a node's script runs as the lab's owner, in the owner's home (no test with its ID yet; `tests/labs/nixos-linux` shows it) | e2e |
| 002 | A script runs with the manifest's `environment_variables` and the env files' variables set, the run ends with the script's exit status, and no copy of the script is left on the node | unit |
| 003 | A script runs as from a login: a tool the owner's login files put on `PATH` is found | unit |
| 004 | A variable of an env file replaces one of the same name that a login file sets | unit |
| 005 | A script with no shebang runs with `sh` | unit |
| 006 | Under a login shell that has no login option, as FreeBSD's `sh` has none, the login files are still read | unit |
| 007 | Under such a shell the env files still win and the run still ends with the script's exit status | unit |
| 008 | A script that exits non-zero stops the node's remaining scripts and provisioners (no test yet) | e2e |
| 009 | A script runs on a FreeBSD node, with what its `~/.profile` sets (no test yet) | e2e |

## `container_secrets`: a container provisioner's files are mounted in its container from the server's memory

**Statement:** a container provisioner's files are mounted read-only in its container at `/run/secrets` before it starts, from a folder in the server's memory that only sherpad enters, are replaced by a redeploy and by `sherpa provision`, and are removed with the lab.
**Relied on by:** the user, for the passwords and keys an image reads from a file. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A container brought up with a secret file reads it at `/run/secrets/<name>` with its mode, and cannot write it (no test yet) | e2e |
| 002 | The CLI reads the files of a node's container provisioners on the user's machine, byte for byte, with mode `444` when none is set, and reads nothing for a node with another provisioner or none | unit |
| 003 | A secret file that is missing on the user's machine is refused, naming it | unit |
| 004 | Files are written with their contents and modes in the node's folder, under a lab folder and a root only the `sherpa` account enters | unit |
| 005 | Writing a node's files again replaces each one, a read-only one included, removes the ones no longer sent, and leaves no temporary file | unit |
| 006 | A set with a bad file in it, a name that is a path, a mode that is not one, contents that are not base64, or two files of one name, changes nothing on the server | unit |
| 007 | A redeploy that sends files replaces the node's, one that sends an empty list removes them and mounts nothing, and one that sends none, as from the web UI, keeps the files the node has | unit |
| 008 | `sherpa provision` rewrites a container's files only when it mounts them from its own folder: a container created without secret files is refused, saying to redeploy it, and one that mounts something else there is refused | unit |
| 009 | Removing a lab's secret files removes that lab's alone, and removing them when there are none is not an error | unit |
| 010 | Secret files sent for a node that is not in the manifest, or that is not a container, are refused, naming the node | unit |
| 011 | At up and at redeploy a bad file is refused, naming the node and the fault, and a set of good files is taken | unit |
| 012 | A secret file's name is one file name of letters, digits and `. _ - + @`, up to 255 long: a path, `.`, `..`, a space, a newline and a shell expression are refused | unit |
| 013 | A secret file's mode is octal digits up to `777` | unit |
| 014 | `sherpa provision` for a node that is not a container writes nothing and is refused as not a container (no test yet) | integration |
| 015 | An up whose secret files cannot be written fails and leaves no lab (no test yet) | integration |
| 016 | A container still reads its secret files after down and resume, and after sherpad restarts (no test yet) | e2e |
| 017 | After the client file changes, `sherpa provision` gives the running container the new contents (no test yet) | e2e |

`sherpa provision` being refused to a user who does not own the lab is `access_control_013`. The secret files are in memory only, so a reboot of the server clears them, and `sherpa provision` sends them again. That is documented; no case asserts it.

## `provisioner_secrecy`: what a provisioner carries is never stored, logged or echoed by the server

**Statement:** the contents of a provisioner's files are never stored by the server, and never shown in an error, a log line or a command line: the manifest the CLI sends has no provisioners, the server refuses one that has, and an ssh provisioner's files and scripts reach the node inside the SSH connection alone.
**Relied on by:** the user, against the server's administrators and its other accounts. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | The manifest the CLI sends has no provisioners, whatever the manifest on disk has, and the server takes it | unit |
| 002 | An up or a redeploy whose manifest has provisioners, as one sent to the REST API or pasted in the web UI has, is refused, naming the node and saying to use `sherpa up`, and a manifest without any is taken (no test at this level yet) | integration |
| 003 | A secret file's contents are base64 in a request, and are not in the debug output of anything that holds them | unit |
| 004 | A request carrying secret files that does not parse is answered without any of what was sent | unit |
| 005 | After an up with an ssh and a container provisioner, the contents of their files are in no file of the lab's folder on the server and nowhere in the database (no test yet) | e2e |
| 006 | While a provisioner's script runs, no process on the node has the script's text or an env file's value in its arguments (no test yet) | e2e |

Root on the server can read a node's memory and disks, and so whatever a provisioner put there. That is by design and documented; no case asserts it.
