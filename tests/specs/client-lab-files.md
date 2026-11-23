# Client lab files and secrets

What the `sherpa` CLI keeps on the user's machine for a lab, and the secrets the server hands it. Every file is in a `.sherpa` folder beside the lab's `manifest.toml`:

| Path | Holds | Readable by |
|---|---|---|
| `.sherpa/lab-info.toml` | the lab's ID, name, owner and management network, and each node's login | as the user's other files |
| `.sherpa/ssh/config` | one host for each node that takes SSH | as the user's other files |
| `.sherpa/ssh/known_hosts` | the host keys the server recorded for the lab's nodes | the user alone |
| `.sherpa/ca/lab-ca.crt` | the lab CA's certificate | as the user's other files |
| `.sherpa/k8s/kubeconfig-<cluster>`, `.sherpa/talos/talosconfig-<cluster>` | a Talos cluster's credentials | the user alone |
| `.sherpa/vault/vault-<node>.toml` | a Vault node's root token and unseal key | the user alone |
| `.sherpa/keycloak/keycloak-<node>.toml` | a Keycloak node's administrator and realm users | the user alone |
| `.sherpa/surrealdb/surrealdb-<node>.toml` | a SurrealDB node's root user | the user alone |

`sherpa up` and `sherpa download` save them, `sherpa redeploy` and `sherpa resume` save them again, a host key that does not match replaces `known_hosts` alone, and `sherpa destroy` deletes the folder. Outside the lab folder the CLI writes one line for the lab in `~/.ssh/sherpa_lab_hosts`, which `~/.ssh/config` includes. The server keeps no copy of the lab info, the SSH config or the known hosts file: it builds them from the lab's records each time they are asked for. It keeps the credentials and the lab CA in the lab's folder on the server.

Who may download a lab's files and ask for its host keys, the owner alone, is `access_control_005` and `010` in [accounts-and-auth.md](accounts-and-auth.md), and the CLI refusing to connect from a folder whose lab is another user's is `access_control_016`. The login token in `~/.sherpa/token` is `user_login_012`. Which lab a folder holds, from `lab-info.toml` and `manifest.toml`, is `lab_identity` in [lab lifecycle](lab-lifecycle.md), and the lab's folder on the server, removed by `lab_destroy`, is there too. The lab CA and the certificates it issues to nodes are `config_delivery_028` and `029` in [ztp.md](ztp.md); here is only the copy of its certificate in the lab folder. Creating a Talos cluster, initialising a Vault node and setting up a Keycloak realm belong to Talos; here is how their credentials reach the user. A container provisioner's secret files are `container_secrets` in [provisioning.md](provisioning.md), and what a provisioner needs of the lab's SSH config is `provision_run_007` and `008`. Moving the files an older client left loose in a lab folder into `.sherpa` belongs to Install and update.

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `ssh_access_001_the_lab_ssh_config_reaches_every_node_directly`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against real services or the network, unit only for logic with too many branches to cover above.

## `saved_lab_files`: up and download save a lab's files in `.sherpa`

**Statement:** `sherpa up` and `sherpa download` save the files the server sends for a lab in a `.sherpa` folder of the lab folder and nowhere else in it, and `sherpa redeploy` and `sherpa resume` save them again.
**Relied on by:** the user; every lab command, which finds its lab by `.sherpa/lab-info.toml`; `sherpa ssh`, provisioners and `ssh`, which read the SSH config. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up, `.sherpa/lab-info.toml` names the lab and its ID, `.sherpa/ssh/config` has a host for each node, and `.sherpa` is all that was added to the lab folder (no test yet; every lab test reads the lab's name and ID from `lab-info.toml` when it brings its lab up) | e2e |
| 002 | The files are saved at their paths under `.sherpa`: `lab-info.toml` as the server gave it, `ssh/config`, `ssh/known_hosts` and `ca/lab-ca.crt`, and nothing is left loose in the lab folder | unit |
| 003 | The lab CA's certificate is saved only when the server sends one | unit |
| 004 | The lab info the server sends names the lab's ID, name, owner, management network, gateway and router, and the login of each node that has one recorded | unit |
| 005 | An up that created its lab only in part still saves the lab's files, so the lab is inspected and destroyed from its folder (no test yet) | e2e |
| 006 | A file that cannot be saved after up is a warning that says to run `sherpa download <lab-id> --force`, and up goes on (no test yet) | e2e |
| 007 | `sherpa download <lab-id>` in an empty folder saves the same files and no `manifest.toml`, and `sherpa inspect` and `sherpa ssh` then work there (no test yet) | e2e |
| 008 | `sherpa download` in a folder that already has a lab's files is refused and changes nothing; with `--force` the files are replaced (no test yet) | e2e |
| 009 | After `sherpa redeploy` and after `sherpa resume` the folder's files are the ones the server has then (no test yet) | e2e |
| 010 | `sherpa download` in a folder that has only one of the files it would save, such as a Vault node's credentials and no `lab-info.toml`, is refused naming that file, and the file is kept | unit |

An up the server refuses writes no `lab-info.toml`: `owner_login_001` checks it. A console opened from a folder made by `sherpa download` is `console_access_005`.

## `lab_file_privacy`: the lab's secret files are readable by the user alone

**Statement:** every folder of `.sherpa` is entered by its owner alone on Linux and macOS, every file of it that holds credentials or host keys is readable by the current user alone on Linux, macOS and Windows from the moment it exists, and nothing is written through a symlink.
**Relied on by:** the user, against the other accounts of their machine. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up of a lab with a Talos cluster and an application node, `.sherpa` and every folder in it have mode `700`, and `known_hosts` and every credentials file mode `600` (no test yet) | e2e |
| 002 | Saving a lab's files leaves `.sherpa`, `ssh` and `ca` with mode `700` and `known_hosts` with mode `600` | unit |
| 003 | A folder that is missing is made, and one that others can enter is closed to them | unit |
| 004 | A `.sherpa`, or a folder of it, that is a symlink is refused, and nothing is written in what it points to nor is its mode changed | unit |
| 005 | A file where a folder belongs is refused | unit |
| 006 | A new secret file is readable by the current user alone: mode `600` on Linux and macOS, and on Windows an access list that names that user alone and inherits nothing | unit |
| 007 | A secret written over a file others can read replaces it, readable by the user alone, and leaves no temporary file | unit |
| 008 | A symlink where a secret file belongs is replaced by the file, and what it pointed to is untouched | unit |
| 009 | A secret that cannot be written fails, saying what the file is and where | unit |
| 010 | A write that fails part way leaves no partial file beside the secret (no test yet) | unit |

Cases 006 and 007 read the file's access list when they run on Windows, so they prove the Windows half only in a Windows run. `lab-info.toml`, the SSH config and the lab CA's certificate hold no secret and are written as the user's other files are. On Windows the folders of `.sherpa` keep the access of the lab folder; no case asserts it.

## `ssh_config_hosts`: the lab's SSH config has a host for each node, logging in as its owner

**Statement:** the lab's SSH config has one host for each node that takes SSH, named by its address, `<node>.<lab-id>` and `<node>.<lab-id>.sherpa.lab.local`, that logs in with the node's recorded login, or else as the lab's owner, with the user's own keys, and works from any folder.
**Relied on by:** the user's `ssh`, `scp` and tools built on them; `sherpa ssh`; provisioners. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The host of each node logs in as the lab's owner and names no identity file | e2e |
| 002 | From another folder, `ssh -F <lab folder>/.sherpa/ssh/config` logs in to a node by `<node>.<lab-id>`, by `<node>.<lab-id>.sherpa.lab.local` and by its address (no test yet) | e2e |
| 003 | Each node has one host, named by its address and its two names, at its address, logging in with its recorded login, or as the lab's owner when it has none, and no host names an identity file | unit |
| 004 | A node whose model takes no SSH login, Talos Linux, has no host, a node with no management address has none, and a node whose image is no longer recorded keeps its host | unit |
| 005 | The SSH config and the known hosts file are built from the lab's records when they are asked for, the nodes in name order | unit |
| 006 | Every host also accepts the older key and key exchange algorithms that older network operating systems offer | unit |
| 007 | The saved config names the lab's known hosts file by its full path in the lab folder, and every other line is as the server sent it | unit |
| 008 | On Windows, a host whose key is not checked names `NUL` as its known hosts file (no test yet) | unit |
| 009 | A lab folder with a space in its path names its known hosts file in double quotes, which ssh reads as one file | unit |

The config has no jump host: `ssh_access_001`. Which nodes are given the owner's login is `owner_login` in [accounts-and-auth.md](accounts-and-auth.md).

## `ssh_config_include`: a lab's nodes are reached by name from any folder

**Statement:** `sherpa up` and `sherpa download` list the lab's SSH config in `~/.ssh/sherpa_lab_hosts`, which `~/.ssh/config` includes before anything else, so `ssh <node>.<lab-id>` works from any folder, and everything else in the user's SSH config is kept.
**Relied on by:** the user's `ssh`, editors and tools that read `~/.ssh/config`. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up, `ssh <node>.<lab-id>` with no `-F` logs in to the node from another folder (no test yet) | e2e |
| 002 | The first lab listed makes `~/.ssh` when it is missing, and `sherpa_lab_hosts` with its header and the lab's `Include` | unit |
| 003 | The `Include` of `sherpa_lab_hosts` is put at the top of `~/.ssh/config`, above the hosts the user has, which are kept | unit |
| 004 | A lab listed twice is in the file once | unit |
| 005 | A lab folder with a space in its path is included in double quotes, and its line is found again when it is checked and when it is removed | unit |
| 006 | A lab folder with a `"` in its path is not listed | unit |
| 007 | A `~/.ssh/config` that is a symlink stays one, and the file it points to is the one changed and keeps its mode | unit |
| 008 | Labs listed at the same moment are all listed, and `~/.ssh/config` includes `sherpa_lab_hosts` once | unit |
| 009 | Removing a lab's line removes that line alone: the other labs' lines, a line whose path only begins the same, and `~/.ssh/config` are as they were | unit |
| 010 | Removing a lab that is not listed, or when there is no `sherpa_lab_hosts`, is not an error and changes nothing | unit |
| 011 | A lab that cannot be listed is a warning at up and at download, which go on (no test yet) | unit |

`sherpa destroy` removing the lab's line is `destroy_cleanup_004`.

## `stale_includes`: `sherpa ssh-config` shows and removes the lines of labs that are gone

**Statement:** `sherpa ssh-config inspect` shows each line of `~/.ssh/sherpa_lab_hosts` as valid, stale, broken or unknown, and `sherpa ssh-config clean` removes the stale and the broken lines and no other.
**Relied on by:** the user, after a lab was destroyed from somewhere other than its folder. **Priority:** P3.

| Case | What it shows | Level |
|---|---|---|
| 001 | After a lab is destroyed from somewhere other than its folder, `sherpa ssh-config clean` removes its line and `sherpa ssh-config inspect` no longer lists its folder | e2e |
| 002 | A line whose lab the server lists for the user is valid, and shown with the lab's ID | unit |
| 003 | A line whose lab the server does not list for the user is stale | unit |
| 004 | A line whose SSH config no longer exists is stale | unit |
| 005 | A line whose lab folder has no `lab-info.toml` that can be read is broken | unit |
| 006 | A line for a file that is not a lab's `.sherpa/ssh/config` is broken | unit |
| 007 | A second line for the same file is stale, and clean leaves one | unit |
| 008 | Clean removes a stale line and keeps a valid one and the file's header | unit |
| 009 | Without a login, or without a server that answers, a line whose files are good is unknown and clean keeps it, and both commands say why the server was not asked (no test yet) | unit |

## `node_host_keys`: nodes are checked against the host keys the server recorded

**Statement:** the server records the SSH host key each node offers when it becomes ready, the lab's known hosts file lists the recorded keys by node address, and a node with a recorded key is checked against it strictly, so the user is never asked to confirm a node's key; when a node's key is not the one saved, `sherpa ssh` and provisioners fetch the lab's keys from the server and connect once more.
**Relied on by:** the user, `sherpa ssh`, provisioners, `ssh` and tools that use the lab's known hosts file. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up, the key in `.sherpa/ssh/known_hosts` for a VM is the host key on the node's disk | e2e |
| 002 | After `sherpa redeploy`, the known hosts file has the rebuilt node's new key, and `ssh -F .sherpa/ssh/config` logs in with no question and no host key error (no test yet) | e2e |
| 003 | When a node's key has changed since the files were saved, `sherpa ssh` warns that the key changed, loads the new key from the server and connects (no test yet) | e2e |
| 004 | When the folder's known hosts file has no key for a node, `sherpa ssh` loads it from the server, says so, and connects (no test yet) | e2e |
| 005 | Retired: nodes take SSH on port 22 alone, so no key is named `[address]:port`; see 017 | — |
| 006 | A key recorded for a node that is no longer in the lab, or that takes no SSH, is left out | unit |
| 007 | A host with a recorded key is checked strictly against the lab's known hosts file, and one with none is not checked and saves no key | unit |
| 008 | The known hosts file is saved in the lab folder only when the server recorded a key | unit |
| 009 | A node's key that is unknown, or that has changed, makes the CLI fetch the lab's keys | unit |
| 010 | The key of the Sherpa server as a jump host, unknown or changed, and any other failure, fetch nothing | unit |
| 011 | A node whose key still does not match the fetched one is refused, saying it is not the key the Sherpa server has for it, and a fetch that fails is reported with the failure to connect (no test yet) | integration |
| 012 | A container that is ready on port 22 has the key it offered recorded | unit |
| 013 | A running VM with no recorded key is probed on SSH for one, also when its ready check is skipped | unit |
| 014 | A recorded key is kept through the node's state changes, is no other node's, and can be cleared | integration |
| 015 | A key that differs from the recorded one replaces it and is logged as changed, and the same key is not written again (no test yet) | unit |
| 016 | The server reads a node's key again only for a virtual machine that takes SSH, and a node that is not ready on SSH keeps its recorded key (no test yet) | integration |
| 017 | The known hosts file names a key by its node's address | unit |

A resumed VM keeping its host keys is `lab_down_resume_003`. The readiness checks that offer a key are `node_ready_013`, `033` and `034` in [nodes.md](nodes.md). Only the lab's owner is given its host keys: `access_control_010`.

## `lab_credentials`: the owner is given the credentials of the lab's application nodes and clusters

**Statement:** each Keycloak and SurrealDB node has an administrator of its own with a generated password, kept on the server in a file only sherpad reads and kept when the node is redeployed, and up and download give the lab's owner those, each Vault node's root token and unseal key and each Talos cluster's kubeconfig and talosconfig, as files of `.sherpa` readable by the user alone.
**Relied on by:** the user, `kubectl`, `talosctl`, `vault` and the applications' own clients. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After up of a lab with a Talos cluster, `.sherpa/k8s/kubeconfig-<cluster>` and `.sherpa/talos/talosconfig-<cluster>` are there, readable by the user alone | e2e |
| 002 | `sherpa download <lab-id> --force` puts back credential files that were deleted, byte for byte and readable by the user alone | e2e |
| 003 | After up of a lab with a Keycloak and a Vault node, `.sherpa/keycloak/keycloak-<node>.toml` and `.sherpa/vault/vault-<node>.toml` are there, readable by the user alone | e2e |
| 004 | The user and password in `.sherpa/surrealdb/surrealdb-<node>.toml` sign in to the node as root (no test yet) | e2e |
| 005 | Each cluster's kubeconfig and talosconfig are saved under the cluster's name | unit |
| 006 | A Vault node's file has its address, root token and unseal key, and is readable by the user alone | unit |
| 007 | A Keycloak node's file has its address, realm, issuer, account and admin console addresses, administrator and realm users, and is readable by the user alone | unit |
| 008 | A SurrealDB node's file has its address, user and password, and is readable by the user alone | unit |
| 009 | A new Keycloak node gets the administrator `admin` with a generated password, and the lab's owner as its realm user with another, saved in a file only sherpad reads | unit |
| 010 | A new SurrealDB node gets the user `root` with a generated password, saved in a file only sherpad reads | unit |
| 011 | A generated password is 20 letters and digits, with a lowercase letter, an uppercase letter and a digit, and no two are the same | unit |
| 012 | Two nodes of one lab get different passwords | unit |
| 013 | A node that is redeployed keeps the administrator and users saved for it | unit |
| 014 | A Keycloak container starts with its saved administrator, the same one after a redeploy, and a second node with its own | unit |
| 015 | A SurrealDB container starts with its saved root user, the same one after a redeploy, with the password in no argument of its command | unit |
| 016 | A generated password is in no log line when a node is created or redeployed | unit |
| 017 | An administrator's password is not in its debug output | unit |
| 018 | A Vault node's root token and unseal key are saved in a file only sherpad reads, and read back | unit |
| 019 | Up and download list each node's credentials under the node's name, in name order, from the files saved for the lab, and none for a lab that has none | unit |
| 020 | A download gives the owner each Keycloak and SurrealDB node's administrator | unit |
| 021 | A download gives each cluster's endpoint, its gateway and Hubble UI addresses, and its saved talosconfig and kubeconfig, under the cluster's name | unit |

What a kubeconfig, a talosconfig and a Keycloak realm hold, and how a Vault node is initialised, belong to Talos. The server's copies go with the lab's folder: `lab_destroy`. A Keycloak or SurrealDB administrator's password is also in its container's environment on the server, where the `docker` group reads it. That is documented; no case asserts it.

## `destroy_cleanup`: destroy removes the lab's files from the user's machine

**Statement:** once the server has destroyed a lab, `sherpa destroy` deletes the lab folder's `.sherpa` whole and the lab's line in `~/.ssh/sherpa_lab_hosts`, and nothing else in the lab folder; while the lab still exists on the server the files are kept.
**Relied on by:** the user, whose machine keeps no credentials of a lab that is gone. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | After `sherpa destroy`, `.sherpa` and the lab's line in `~/.ssh/sherpa_lab_hosts` are gone, and `manifest.toml` and the user's other files are as they were (no test yet) | e2e |
| 002 | `.sherpa` is deleted with everything in it, a file Sherpa did not write included, the other files of the lab folder are kept, and a second run finds nothing to delete | unit |
| 003 | A `.sherpa` that is a symlink is not deleted, and neither is what it points to | unit |
| 004 | The lab's line is removed from `sherpa_lab_hosts` and another lab's is kept | unit |
| 005 | A destroy the server could not finish keeps the local files and says so, and the next `sherpa destroy` removes them (no test yet) | integration |
| 006 | A file or a line that cannot be removed is a warning, and the destroy still succeeds (no test yet) | unit |

A lab destroyed from somewhere other than its folder, by an admin, from the web UI or from a second folder it was downloaded to, leaves that folder's `.sherpa` as it is; its line in `~/.ssh/sherpa_lab_hosts` is removed by `stale_includes_001`.

## `ssh_target`: `sherpa ssh` finds its node in the lab's files, or says what is missing

**Statement:** `sherpa ssh <node>` takes a node by its name, by `<node>.<lab-id>` or by its full name, refuses before it connects a node the lab does not have and a folder without the lab's connection files, saying how to get them, and exits with the exit code of the node's shell.
**Relied on by:** the user, scripts that run `sherpa ssh`. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A node is taken by its name, by `<node>.<lab-id>` and by `<node>.<lab-id>.sherpa.lab.local` | unit |
| 002 | A node that is not in the folder's `manifest.toml` is refused, naming the node, before any lab file is needed | unit |
| 003 | A node with no host in the lab's SSH config is refused, naming it: a comment, a `HostName` line and the host of a node with a longer name are not it | unit |
| 004 | A folder with a manifest and no connection files says to run `sherpa up`, or `sherpa download <lab-id> --force` | unit |
| 005 | A folder made by `sherpa download`, with no manifest, connects by any of the node's names | unit |
| 006 | Connection files that belong to another lab are refused | unit |
| 007 | A connection file that is there and cannot be read fails with the cause, not with the advice to run up | unit |
| 008 | A node's shell that ends with a non-zero exit code is the exit code of `sherpa ssh` | unit |
| 009 | After the `~.` escape, the line that says the connection is closed starts on a new line | unit |

Opening the shell, directly or with `--ssh-jump`, is `ssh_access_002` and `003`. A lab that is another user's is refused first: `access_control_016`.

## `ssh_access`: the CLI reaches nodes directly, or through the server with `--ssh-jump`

**Statement:** `sherpa ssh` and SSH provisioners reach a lab's nodes directly, or through the Sherpa server with `--ssh-jump`, and fail at once with a hint when a node can't be reached.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The lab's SSH config reaches every node directly, with no jump host | e2e |
| 002 | `sherpa ssh` connects to a node itself | e2e |
| 003 | `sherpa ssh --ssh-jump` connects only to the Sherpa server, which connects to the node | e2e |
| 004 | A provisioner (`sherpa provision`) connects to its node itself | e2e |
| 005 | A provisioner with `--ssh-jump` connects to its node through the Sherpa server | e2e |
| 006 | A node with no route from the client fails at once, not after the ready timeout, saying to use `--ssh-jump` or the lab's tailnet | integration |
| 007 | An IPv6 server address is the jump host without its URL brackets | unit |
| 008 | A provisioner may ask for the passphrase of the user's key at the node, the first hop without a jump, but never for a node password | unit |
| 009 | An owner with an RSA key of 4096 bits or less logs in to a Cisco IOL node by key: the IOL config's `ip ssh pubkey-chain` has the MD5 `key-hash` of that key, and of no key IOL refuses | unit |

Cases 001 to 005 are in `tests/labs/ssh-access`; 006 to 008 are Rust tests in the client crate
(`cargo test -p sherpa ssh_access_`, and `-- --ignored` for 006); 009 is in the server crate
(`cargo test -p sherpad ssh_access_`), and `tests/labs/cisco-iol` logs in to IOL by key end to end.
