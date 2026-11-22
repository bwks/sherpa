# Inspect and web UI

What Sherpa shows about labs and nodes, and to whom: what `sherpa inspect` prints and how it exits, the tables the CLI draws, what the server reports about a lab and where it reads it from, the lists of labs, the web UI's lab and node pages, the actions they offer and how they keep up with the server, the job page of a lab made or destroyed in the web UI, the create-lab form, the sidebar, the dashboard and the theme, what keeps a page from running what a user typed, the API documents the server publishes, `sherpa server status` and `/health`, and what a refused or failed request is answered with.

Other areas own what is shown when it is the subject of their behaviour:

- What state a node is in, and that inspect reads a container's from Docker, is `node_state` in [nodes.md](nodes.md), with which actions a node's state allows (`node_state_020`). A lab's status from its nodes is `lab_status` in [lab-lifecycle.md](lab-lifecycle.md).
- Who may see a lab, a list or a job, over RPC, REST and the web UI, is `access_control` in [accounts-and-auth.md](accounts-and-auth.md): an admin inspecting another user's lab (`004`), every lab operation (`009`), the lab lists (`011`), web jobs (`012`), unknown labs and nodes (`013`), no login (`015`). The admin pages are `admin_only_004`. Login, the session cookie, the login page and its notices are `user_login` and `user_sessions`; the profile page is `account_keys`, `user_passwords` and `device_password`.
- What a link's state is, and what the server reports for it, is `link_state` in [links-and-bridges.md](links-and-bridges.md). How the CLI prints it is `cli_tables_011` here.
- The Tailscale table and the status in it are `tailnet_status` in [tailnet.md](tailnet.md). The Provisioning table is `provision_report_003` in [provisioning.md](provisioning.md). The image tables and the admin image pages are `image_list`, `image_settings` and `image_import` in [images.md](images.md).
- Start, stop, redeploy and destroy from the web UI are the operations of [lab-lifecycle.md](lab-lifecycle.md) and [nodes.md](nodes.md); what a manifest pasted in the web UI is refused for is in [manifests.md](manifests.md), `provisioner_secrecy_002` and `tailnet_key_011`. The lab download, and the files in it, are `saved_lab_files` and `access_control_010`.
- Which lab a folder holds, which `sherpa inspect` asks the server for, is with the lab folder's files in [client-lab-files.md](client-lab-files.md) and lab lifecycle.
- The web UI opens no console: `console_access_004` in [consoles.md](consoles.md).

Each behaviour lists its cases. A case is one test, whose name is its ID: `<behaviour>_<NNN>_<description>`, such as `cli_tables_011_the_links_table_shows_each_links_state`. The level is the highest that can show the case: e2e through the `sherpa` CLI, integration against a running sherpad or the database, unit for a table, a template or a handler's logic. A case marked e2e (browser) needs a browser driving the pages' scripts; nothing runs one today.

## Where the CLI and the web UI differ

They show the same answer of the server, `lab_details`, differently. The cases below record each as it is.

| What | `sherpa inspect` | Web UI |
|---|---|---|
| The lab's status | not printed; only each node's state | a badge on the labs list and the lab page |
| A lab that is not ready | a `Not ready:` line that says what to do | the phase as a badge, and no start or stop |
| A link's state | the State column of the Links table | not shown |
| The Tailscale gateway | the Tailscale table | not shown |
| A node's vCPUs and memory | columns of Active Nodes; `-` for none | on the node's page only; `Not limited` or `Unknown` for none |
| A node's disks | one per line | on one line, separated by commas |
| A node's MAC address, image version, ZTP setting and image settings | not shown | on the node's page |
| The lab's owner | the `User` row | `Owner` |
| The labs of a user | no command lists them | the labs list |
| Every user's labs | no command | the admin's list, which shows each lab's phase and not its status |

## `inspect_command`: `sherpa inspect` prints the lab of the current folder

**Statement:** `sherpa inspect`, which is also what `sherpa` runs with no subcommand, prints the lab of the current folder as the server has it now and exits 0, or says why it cannot and exits 1.
**Relied on by:** the user; scripts and lab checks that read its output or its exit status. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | In a lab's folder it prints the heading `Sherpa Environment - <name>-<id>`, the server it connects to, the Lab Info table and an Active Nodes row for every node with its state, and exits 0 (no test with its ID yet; `tests/labs/ssh-access` reads the server from it, `tests/labs/owner-login` the Active Nodes rows and the VM suite the lab's ID) | e2e |
| 002 | `sherpa` with no subcommand prints the same (no test yet) | e2e |
| 003 | For a lab the server does not have it prints no table, says the lab was not found and exits 1 (no test with its ID yet; the teardown of eight lab checks relies on the exit status) | e2e |
| 004 | The Active Nodes, Links and Bridges tables are each printed only for a lab that has nodes, links or bridges (no test yet) | e2e |
| 005 | For a lab that is not ready it prints `Not ready:` and what to do under the Lab Info table, and still exits 0 (no test yet) | e2e |

Inspect prints no status of the lab as a whole. The Tailscale table of a lab with a gateway is `tailnet_status_012`. An admin's inspect of another user's lab is `access_control_004`.

## `cli_tables`: the CLI prints what it lists as tables

**Statement:** the CLI prints what it lists as bordered tables, and a lab's tables hold what the server reported for each node, link and bridge, with `-` where there is nothing to show.
**Relied on by:** the user; scripts and lab checks that read a table's cells. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | A table with a title has it centred above the headers | unit |
| 002 | The columns a table names as right-aligned are, and the others are left-aligned | unit |
| 003 | A table with no rows still shows its headers | unit |
| 004 | A table with no header row shows its title and rows only | unit |
| 005 | A table given a width wraps its widest column between words to stay within it | unit |
| 006 | A title, wrapping and right alignment work together in one table | unit |
| 007 | The Lab Info table has a row for the ID, name, description, user, management network, IPv4 gateway and router, client host and client folder, and no IPv6 row for a lab with no IPv6 | unit |
| 008 | The Lab Info table of a lab with IPv6 has its IPv6 network, gateway and router, and one with no description or client origin has no row for them (no test yet) | unit |
| 009 | An Active Nodes row names the node `<name>.<lab_id>` and gives its model, kind, state, management IPv4 and disk, with `-` for no IPv6, vCPUs or memory | unit |
| 010 | An Active Nodes row gives the vCPUs, memory and IPv6 address a node has, several disks one per line, and `-` for no disk (no test yet) | unit |
| 011 | The Links table gives each link's two nodes and interfaces, its type and its state, as `attached`, `peer down: <node>`, `failed: <reason>` or `not checked` | unit |
| 012 | The Bridges table gives each bridge's name, its network and its nodes, separated by commas (no test yet) | unit |
| 013 | The Nodes table `sherpa up` prints names each node `<name>.<lab_id>` with its management address, `<address>:<port>` to connect to, its model and kind, and `-` for a node with no address | unit |

## `lab_details`: what the server reports about a lab is its records as they are now

**Statement:** the server answers `sherpa inspect`, `GET /api/v1/labs/{id}` and the web UI's lab and node pages alike from the lab's records as they are now: the lab's details and owner, each node with its model, kind, addresses, vCPUs, memory and disks, each link and bridge by its nodes' names, and the lab's status and phase.
**Relied on by:** the user, the web UI, scripts reading the JSON. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A lab's details are its record's: ID, name, owner's name, management network, gateway and router, the IPv6 ones it has and none it does not, description and client host | unit |
| 002 | A node's details are its record's name, management address, vCPUs and memory, and its disks are the volumes of the storage pool named for it, not another node's | unit |
| 003 | The RPC `inspect` and `GET /api/v1/labs/{id}` give the same answer for a lab: its details, each node with its model and kind, each link with its nodes' names and interfaces, each bridge with its members' names, and the lab's status and phase (no test yet) | integration |
| 004 | A lab with no nodes is answered with empty lists and the status `empty` (no test yet) | integration |
| 005 | The answer holds no password, token or key: of a lab's accounts only each node's login name (no test yet) | integration |
| 006 | An answer from a server that sends no phase reads as a `ready` lab (no test yet) | unit |
| 007 | With libvirt unreachable, or its storage pool missing, the lab is not reported at all, a lab of containers included (no test yet) | integration |

Each node's state in the answer is `node_state_004` to `009`, each link's state `link_state_006`, the gateway's status `tailnet_status`.

## `labs_list`: a user's labs are listed with their status

**Statement:** a user's labs are listed with each lab's name, ID, description, node count and status, or its phase while it is not ready; the admin's list holds every user's labs with their owners and phases.
**Relied on by:** the user, the admin finding labs left behind, scripts reading the JSON, `sherpa ssh-config`. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The RPC `labs.list` and `GET /api/v1/labs` give each of the user's labs with its ID, name, description, node count, status and phase, and their total (no test yet) | integration |
| 002 | A row of the web UI's labs list shows the lab's name, linked to its page, its ID and its node count | unit |
| 003 | A user with no labs is shown that they have none, with a link to create one | integration |
| 004 | A ready lab's row shows its status as a badge: green for running, amber for starting and partial, red for failed, grey for stopped, empty and unknown (no test yet) | unit |
| 005 | A lab that is not ready shows its phase, amber while it is created or destroyed and red when left `destroy failed`, `interrupted` or unknown, never its status, and offers no start or stop | unit |
| 006 | A ready lab's row offers stop while the lab is running or starting, start and stop while partial, and start otherwise (no test yet) | unit |
| 007 | A row of the admin's list shows the lab's owner and phase, and offers no start or stop for a lab that is not ready | unit |
| 008 | The admin's list holds the labs of every user, in name order, each with its owner and node count, and `unknown` for an owner whose account is gone (no test at this level yet) | integration |
| 009 | The admin's list shows a lab's phase and never its status, and offers a ready lab both start and stop whatever its nodes are doing (no test yet) | unit |
| 010 | A list from a server that sends no phase reads as `ready` labs | unit |
| 011 | The labs list loads when its page opens and again every 30 seconds (no test yet) | e2e (browser) |
| 012 | The node count is the number of the lab's own nodes, another lab's not counted | integration |

Each lab's status is `lab_status_006`. Whose labs a list holds is `access_control_011`.

## `lab_page`: a lab's page shows the lab, its nodes and what can be done with it now

**Statement:** a lab's page shows its owner, or an admin, the lab's details, its status or phase, its nodes, links and bridges, and only the actions the lab can take now.
**Relied on by:** the user, the admin. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | The page of a lab shows its name, ID, owner, management network, gateway and router, its status, and its nodes, links and bridges, each with a total (no test yet) | integration |
| 002 | The description, the IPv6 network, gateway and router, and the client host and folder are shown only for a lab that has them (no test yet) | unit |
| 003 | While the lab is not ready the header shows its phase as a large badge, and not its status | unit |
| 004 | Once the lab is ready the header shows its status | unit |
| 005 | While the lab is not ready the page offers no start or stop, and still offers destroy | unit |
| 006 | A ready lab's page offers stop while the lab is running or starting, start and stop while partial, and start otherwise (no test yet) | unit |
| 007 | The lab's files are offered for download only for a lab with nodes in a known state | unit |
| 008 | The download is not offered to an admin who does not own the lab (no test yet) | unit |
| 009 | The nodes table names each node `<name>.<lab_id>`, linked to the node's page | unit |
| 010 | A row of the nodes table gives the node's model and kind, its state as a badge (green for running, amber for building and starting, red for failed and missing, grey for stopped and unknown), its addresses or `-`, and its disks on one line or `-`; a lab with no nodes says so (no test yet) | unit |
| 011 | The links table gives each link's nodes, interfaces and type, and no state; the bridges table each bridge's name, network and nodes; a lab with none says so (no test yet) | unit |
| 012 | The nodes table is loaded again every 60 seconds without a reload of the page; the lab's own badge and its start and stop buttons stay as they were loaded (no test yet) | e2e (browser) |
| 013 | After a start, stop or redeploy the page says which nodes were taken, and each link left unattached with the reason, in red when a node or a link failed (no test yet) | e2e (browser) |
| 014 | An error page is served as HTML with its own status, 404 or 403 | unit |
| 015 | A lab that does not exist is a 404 page saying so, and any other failure to load a lab is a 404 page too, with a general message (no test yet) | integration |

That another user's lab is refused is `access_control_009`. Which actions a node's row offers is `node_state_020`.

## `node_page`: a node's page shows the node as it was built

**Statement:** a node's page shows the node as it was built: its state, its addresses, the vCPUs and memory it was given, its disks, its ZTP setting, and the settings of its image that its kind uses.
**Relied on by:** the user, the admin. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | The page names the node `<name>.<lab_id>` with its model, kind and state, and gives its management IPv4, IPv6 and MAC addresses and management interface, with `-` for one it has not (no test yet) | integration |
| 002 | The vCPUs and memory shown are the node's own, not its image's defaults | unit |
| 003 | A container with no limit shows `Not limited` for both | unit |
| 004 | A virtual machine or unikernel with none recorded shows `Unknown` (no test yet) | unit |
| 005 | A virtual machine shows its firmware, machine type and disk buses, a unikernel its machine type, a container none of them, and every kind its own disks | unit |
| 006 | The ZTP setting shown is the one the lab's stored definition sets on the node, or else its image's | unit |
| 007 | The image's model, version and kind are shown, and a container image's repository (no test yet) | unit |
| 008 | A node the lab does not have is a 404 page that names it (no test yet) | integration |

## `web_jobs`: a lab made or destroyed in the web UI is shown as a job

**Statement:** a lab created or destroyed in the web UI is shown as a job: a page that prints each step as the server reports it, and ends with what was made or removed, or why it failed.
**Relied on by:** the user. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | A status message of the operation is a progress line with its text | unit |
| 002 | A message that is not a status is not shown | unit |
| 003 | A status message with no kind is shown as information | unit |
| 004 | The page of a job that does not exist is a 404 page, and its stream ends at once, saying the job was not found or has already run | integration |
| 005 | A create job ends with the lab's name, the containers, virtual machines, unikernels, networks, bridges and interfaces made, the time taken, and a link to the lab (no test yet) | integration |
| 006 | A create that fails ends with each error and the phase it came from, and a link to try again (no test yet) | unit |
| 007 | A destroy job ends with what was removed, and one that left something behind lists each resource with its error (no test yet) | unit |
| 008 | A job runs when its stream is first opened, and once: its page opened again while it runs shows no progress, and says the job was not found or has already run (no test yet) | integration |
| 009 | Before a destroy the page asks, naming the lab and how many nodes it has and that it cannot be undone, and cancel puts the destroy button back (no test yet) | integration |

Who may see a job is `access_control_012`. That it runs as its requester only while their session lasts is `user_sessions_022`.

## `lab_create_form`: the create-lab page builds or takes a manifest

**Statement:** the create-lab page offers every model with the kinds it takes, and sends the manifest the user builds or pastes to be brought up as their lab.
**Relied on by:** the user. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | Each model is offered with its kinds and its default kind, none for `generic`, and every kind is offered | unit |
| 002 | The fields the form requires are all in the build panel, so the upload panel cannot block the submit | unit |
| 003 | A form with no manifest, with TOML that does not parse, or with no `name`, is answered with the reason in the page, and no job is made (no test yet) | integration |
| 004 | A manifest that parses starts a job and sends the browser to its page, for the lab ID `sherpa up` would give the user for that name (no test yet) | integration |
| 005 | Choosing a model offers only its kinds, with its default chosen, and the nodes and links added in the form are the manifest that is sent (no test yet) | e2e (browser) |

## `web_layout`: every page has the sidebar, and the dashboard counts what the user has

**Statement:** every page has the sidebar, which links an admin, and only an admin, to the admin pages; the dashboard counts the user's labs and the server's images and says whether Docker and libvirt answer; the pages are light or dark as the browser, or the user, chose.
**Relied on by:** the user, the admin. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | The sidebar links an admin to the admin pages: images, labs, users and tools | unit |
| 002 | The sidebar of a user who is not an admin has no link to an admin page | unit |
| 003 | The sidebar highlights the link of the page being shown and no other, and none on a page with no link | unit |
| 004 | The sidebar shows the user's name and its first letter in capitals | unit |
| 005 | The dashboard shows the number of the user's labs, the number of images, and Docker and libvirt as connected or unavailable (no test yet) | integration |
| 006 | The dashboard's lab count is the user's own labs, another user's not counted | integration |
| 007 | The dashboard's image count is every recorded image | integration |
| 008 | A new visitor gets the light or dark theme of their system | unit (JavaScript) |
| 009 | A saved choice of light or dark wins over the system's | unit (JavaScript) |
| 010 | The theme follows the system until the user toggles it, and the toggle saves the choice | unit (JavaScript) |

That the admin pages themselves are refused to others is `admin_only_004`.

## `page_safety`: a page never runs what a user typed

**Statement:** nothing a user typed is run by a page: every value is shown as text, and no page carries script of its own.
**Relied on by:** every user whose browser shows another user's lab name, description, node names or error text; the admin above all. **Priority:** P0.

| Case | What it shows | Level |
|---|---|---|
| 001 | A lab description with markup in it is shown escaped in the labs list | unit |
| 002 | No template has an inline event handler or a script block: every script is a file, which reads its arguments from `data-` attributes | unit |
| 003 | An error message with markup in it is shown escaped in an admin notification | unit |
| 004 | A lab's name, description, client host and client folder, and a node's name, with markup in them are shown escaped on the lab page, the node page, the job page and the destroy panel (no test yet) | unit |
| 005 | The message a page's script adds after an action, which may quote an error, is inserted as text (no test yet) | e2e (browser) |
| 006 | A progress line and a failure message of a job with markup in them are escaped in its stream (no test yet) | unit |

The session cookie that keeps another site's requests from acting as the user is `user_login_008`, and cross-origin access `user_sessions_025`. The login page's notice is `user_login_011`.

## `api_documents`: the server publishes what its API offers

**Statement:** the server publishes what its API offers, as its own spec and as an OpenAPI 3.1 document with a Swagger page, to anyone, and what it publishes is what it serves.
**Relied on by:** users writing clients and scripts against the REST API or the RPC. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | `GET /api/v1/spec` is answered without a login | integration |
| 002 | Every REST operation in the spec is a route the server serves | integration |
| 003 | `GET /api/v1/openapi.json` is an OpenAPI 3.1 document that lists the lab, image and login paths, and `/api/docs` is the Swagger page, both without a login | integration |
| 004 | The RPC methods the server answers are the ones the spec lists, each once, with the same access and streaming; `image.upload` is REST only, and `labs.list`, `download`, `node.secrets`, `node.host_keys` and `server.backup` are answered and not in the spec | unit |
| 005 | Every operation has a name of its own, and each request and response schema it names is in the spec | unit |
| 006 | The OpenAPI document has every REST path of the spec | unit |
| 007 | A streaming operation has a stream type, and is a `text/event-stream` response in the OpenAPI document | unit |
| 008 | An operation that needs no login has no security requirement in the OpenAPI document | unit |
| 009 | An admin operation is marked `x-admin-required` in the OpenAPI document | unit |
| 010 | A path's parameters are listed for it | unit |
| 011 | Every schema reference in the OpenAPI document points inside the document | unit |

## `server_status`: anyone can ask whether a server is up

**Statement:** `sherpa server status` and `GET /health` say whether a server is up and whether it uses TLS, without a login.
**Relied on by:** the user before logging in, the admin, monitoring. **Priority:** P2.

| Case | What it shows | Level |
|---|---|---|
| 001 | `GET /health` answers 200 without a login, with the status `ok`, the service's name, its version and whether TLS is on | integration |
| 002 | `sherpa server status` of a running server prints one row: the server's URL, `online`, and TLS `enabled` for a `wss://` URL or `disabled`; it needs no login and exits 0 (no test yet) | e2e |
| 003 | For a server that does not answer within 5 seconds it prints `offline` with `-` for TLS, and still exits 0 (no test yet) | e2e |
| 004 | A client that connects to `/ws` is first sent its connection's ID and the server's version (no test yet) | integration |

## `api_errors`: a refused or failed request is answered with what went wrong

**Statement:** a request the server refuses or fails is answered with the kind of failure, as a code a client can act on, and a message for the user, which for a request that does not parse is fixed, names at most the path of the field that is wrong and never the value sent; a failed CLI command prints it and exits 1.
**Relied on by:** the CLI, the web UI's scripts, users' scripts and clients. **Priority:** P1.

| Case | What it shows | Level |
|---|---|---|
| 001 | An RPC call of a method the server does not have is answered method not found, `-32601` | integration |
| 002 | An internal error over REST is answered with the code `INTERNAL_ERROR` and a general message, and none of its detail | unit |
| 003 | An RPC failure of no known kind is a server error with the operation's message, the error's chain of messages as its context, not its debug form, and no data | unit |
| 004 | An RPC refusal of a known kind keeps its code and its own message, and has no context | unit |
| 005 | An error code is sent as its JSON-RPC number and read back from it, and a number this version does not know is kept as it came | unit |
| 006 | Over REST each kind of failure has its status, 401, 403, 404, 400, 409, 507 for memory and 500, in an `error` object with a code and a message (no test yet; the 403, 404 and 409 of single operations are cases of `access_control` and `user_accounts`) | integration |
| 007 | A REST failure of no known kind whose text has `not found`, `Invalid` or `missing` in it is answered 404 or 400 with its whole chain of messages as the detail (no test yet) | unit |
| 008 | A CLI command the server refuses or fails prints the error with its causes and exits 1 (no test at this level yet) | e2e |
| 009 | RPC parameters that do not parse, for any method, are answered invalid params with the method's message, and the context `the request is not valid at <path>`, such as `password`, `ztp_password` or `files[0].mode`, or `the request is not valid` for a missing top-level field or a field sent with a name that is not plain; never the value sent: a password sent as a number is not in it | unit |
| 010 | A REST JSON body that does not parse is answered 400 `BAD_REQUEST`, `Invalid request`, with the details `the request is not valid at <path>`, or `the request is not valid` for a missing top-level field, never the value sent; a body that is not JSON, or not sent as JSON, says only that | unit |
| 011 | A REST form, query string or path parameter that does not parse is answered 400 `BAD_REQUEST`, `the request is not valid`, with no path and never the value sent; a form not sent as one says only that | unit |
| 012 | A WebSocket message that is not a request is answered `the request is not valid`, with the path where serde gives one, such as `type`, or that it is not JSON, never what was sent | unit |
| 013 | A request that does not parse logs nothing of what was sent, at any level: not a password sent as a number, nor a string nested in a value of the wrong type | unit |
| 014 | An up whose manifest does not parse is refused with the path in the manifest, such as `nodes[0].model`, never its value | unit |
| 015 | A redeploy whose manifest does not parse is refused with the path in the manifest, such as `nodes[0].model`, never its value | unit |

Exit status 75, for a refusal for memory or held CPUs, and what those refusals carry, are `lab_preflight_007` in [lab-lifecycle.md](lab-lifecycle.md). That a request which does not parse is answered without its secrets is `provisioner_secrecy_004`. The path in the answer to a request that does not parse is given only when each of its segments is an index or a plain name of letters, digits, `_` and `-`, since a map key or an unknown field is what the client sent; serde gives none inside an internally tagged enum, such as the WebSocket message's own fields. A JSON key sent twice is refused as `the request is not valid`.
