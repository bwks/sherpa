# Sherpa Application Server Architecture

`sherpad` (`crates/server`) serves the web UI, REST, WebSocket JSON-RPC and the CLI. Handlers know
transports and authentication, services (`services/`) know orchestration, and the infrastructure
crates know their backend and nothing of HTTP or RPC. The operations, their access and transports
are one registry, `shared::api_spec`, published at `/api/v1/spec`; the user's view is sherpa-docs
`api-reference.md`. Server-only constants live in the narrowest module that uses them, else in
`crates/server/src/konst.rs`, so editing one rebuilds `sherpad` alone.

## Server startup architecture

`daemon::server::run_server` installs the signal handlers before any listener binds, so a server
that could not shut down cleanly does not start. `AppState::new` loads the JWT secret, opens and
migrates the database, loads its key, requires the built-in `admin` (created only from
`SHERPA_ADMIN_PASSWORD`), restricts storage and moves renamed models' image folders. A secret or key
that is damaged stops startup and is never replaced: a new secret would end every session, a new
key would open nothing sealed. Before anything takes requests, `lab_lifecycle::recover` marks what
the last run left mid-way, shared CPU pins are logged and stopped domains' consoles move to their
sockets; then the scanner and the session sweeper start.

### Lab and database storage permissions

Lab data and the database are kept from other users by directory modes, not file modes
(`daemon::storage::secure_storage`, every start), so files of any age are covered. File modes in
lab folders are left alone: dnsmasq runs as root and serves only world-readable files, and
containers may read their bind-mounted configs as non-root. Pool volumes are `0600`, and libvirt
hands them to QEMU while a VM runs. The modes are `install-and-update.md`, `server_file_modes`.

### Who owns `/opt/sherpa`

`/opt/sherpa`, `bin`, the binaries and `libvirt` are root's and `0755`: systemd runs
`bin/sherpad` as `sherpa`, and the scripts run it as root, so neither `sherpa` nor its group may
change it. `sherpa` owns only the folders sherpad writes (config, env, logs, `.secret`, `.certs`,
db, labs, the storage pool, images, containers, bins, run). sherpad cannot create a folder in
`/opt/sherpa`, so the install and update scripts make each of them (`secure_server_folders`, in
the `# BEGIN server folders` section both scripts carry), changing the folders only, never what
is in them. The `sherpa` group reads the lab folders and the log, and writes only the images
folder, where admins stage node images; nothing there is run by systemd or written by root.

Root changes nothing through a symlink: the scripts refuse a path with a symlink in any
component, set owners with `chown -h`, install binaries and env files with `install`, which
replaces a link, and write the blank disks and set file modes as `sherpa`. The update script
runs an installed binary for its version only when the binary is root's and no other account
can write it; any other is replaced without being run. When a new release does not start, the
update moves back only the binaries it set aside, keeping their owners, so it never makes a
binary another account wrote root's. The modes are `install-and-update.md`,
`server_file_modes`.

### Config, env and log file permissions

`sherpa.toml` and the env files can hold secrets, so only `sherpa` reads them, not its group.
`sherpad init` runs as `sherpa` and refuses any other account, root included. The update script
changes owner and mode only, and skips symlinks, so root never follows one planted by `sherpa`.

## Transport architecture

The web UI is server-rendered Askama with HTMX. Templates only render: handlers decide from enums
and pass view types, and `dev/check-template-text-decisions.sh` rejects text decisions in templates.

`api/websocket/rpc/dispatch.rs` holds `METHODS`, the one table of RPC methods with each method's
access and whether it streams; `dispatch` authenticates as it says, and a test holds it to the
API spec. Long operations report through a `ProgressSender`, so a service never knows whether its
progress goes to SSE, the web UI or a WebSocket.

Every request body and query string is read through `api/request_parse.rs`: handlers take
`JsonBody`, `FormBody` and `QueryParams` instead of axum's `Json`, `Form` and `Query`, RPC
parameters go through `dispatch::parse`, and the WebSocket message and the manifests of `up` and
`redeploy` through `parse_json` and `parse_value`. serde's and axum's messages quote the value
they failed on, a password or a secret file among them, so their text is never read, logged or
kept. A JSON body or WebSocket message is deserialized straight into its type with
`serde_path_to_error` over `serde_json::Deserializer::from_slice`, and a `serde_json::Value` (RPC
parameters, a manifest) with `serde_path_to_error` over the value itself. A form body, taken as
bytes by axum's `RawForm`, and a query string, taken from the URI, are deserialized the same way
over `serde_urlencoded::Deserializer` on `form_urlencoded::parse`. The error keeps only the
path. The answer is `400` with `the request is not valid at <path>`, or `the request is not
valid` when the path is empty (a missing top-level field) or has a segment that is not an index
or a plain name, since map keys and unknown fields are what the client sent. Only a body that is
not JSON, the wrong content type and a body that cannot be read keep their own fixed messages. A
JSON body must be sent as `application/json` or `application/*+json`, which stops a cross-site
form post: the `Content-Type` header is parsed as a `mime::Mime` and its type, subtype and suffix
compared. A form must be sent as `application/x-www-form-urlencoded`, which `RawForm` checks.
Path parameters are read by axum's own `Path`: they come from the URL, which is not secret.

## Authentication and authorization architecture

### Sessions and identity

Cookies, bearer tokens and the RPC `params.token` carry the same JWT. Each names a login session,
and identity comes from the session's row, not the token: `auth::session::lookup` reads the session and its user in one query, so a token works only while
its session is active, a demotion applies on the next request, and a deleted user's token never
works for a new account of the same name. No authorization decision reads the token's own claims.
What ends a session is `tests/specs/accounts-and-auth.md`, `user_sessions`.

RPC, REST and web logins all check passwords through `auth::login::check_credentials`, which first
asks `auth::login_limit::LoginLimiter` (in `AppState`, in memory, bounded) to let the attempt
through. It keeps three tables: username and peer address together (5 failures), peer address
(20) and username from any address (50), each locking for 15 minutes. The pair limit is the
tight one so that failures from one address cannot lock a user out from another. A successful
login clears only its own pair, not other addresses' pairs or the username count. The limits are
`user_login` in `tests/specs/accounts-and-auth.md`.
An IPv4 peer is counted by its address and an IPv6 peer by its /64 prefix (`AddressKey`).
Each table holds at most 100,000 keys. A new key in a full table drops expired entries, then the
oldest count that is not a lock; a lock is never dropped, and when every entry is one the attempt
is let through uncounted in that table. A full table never refuses a login.

### Services enforce authorization

Every lab service checks the caller's `&AuthContext` with `auth/policy.rs` before it sends
progress, takes a lock or touches a resource; requests carry no username. Refusals are a typed
`ServiceError` each transport maps to its status, never decided by error text. Who may do what is
`tests/specs/accounts-and-auth.md`, `access_control`.

#### The admin boundary

Admins see and manage every lab, but only its owner connects to its nodes: the owner is their only
login, and download, host keys, consoles and redeploy are owner-only. What remains is by design: an
admin who resets a password can sign in as that user (logged, ending their sessions), and an admin
may start privileged containers and host mounts, which is root. The boundary is the admin role
through Sherpa's interfaces, with an audit trail, not a defence against root.

### Device passwords

Only the account's own user sets one (`services::user_device_password`), admins included, as
whoever sets it knows it, and it takes their login password, so a stolen session cannot. Nodes'
use of it is in [`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md#device-password).

### When a transport checks first

REST destroy and redeploy answer with SSE, where a service refusal would arrive as a 200 with an
error event, so they call the policy helper first, as do web redeploy, download and destroy; the
service checks again. A web job presents its requester's session again when it starts.

## Service layer architecture

`node_lifecycle` alone writes node states and `lab_lifecycle` lab phases; see
`tests/specs/nodes.md` and `lab-lifecycle.md`.

### Service call graph for lab creation

`up_lab` checks the request, takes the lab's lease and runs `shared::data::UpPhase` in order.
Setup (1) checks the nodes against their images before libvirt or Docker is touched, so a missing
image gets the same answer whatever the hypervisor's state, and ends by reserving the lab
(`up::networks::reserve_lab`), whose unique subnet indexes make the insert the reservation.
ManifestValidation (2) expands the whole manifest, so no parsing rule spreads into backend calls,
leases images and runs admission; a failure gives the reservation back. From phase 3 on, a failure
rolls the lab back with `clean::clean_lab_resources`, unless a destroy stopped it.

### Service call graph for lab destruction

`destroy::tear_down_lab` serves destroy, admin clean and `up`'s rollback alike, under the write
lease, and tries every step even when one fails. Resources are matched by their whole name,
`<name>-<lab_id>` (`shared::util::is_lab_resource`), since the database may lag the backends after
a partial failure and a substring match would take another lab's; a new disk kind needs its suffix
in `LAB_VOLUME_SUFFIXES`. When anything is left, the record stays and the lab is `DestroyFailed`;
otherwise the folder goes before the record, so a stuck folder still has a record to destroy.

### Operation lifecycle and stopping operations

Long operations run as their own tasks and outlive their client (`run_detached` for plain HTTP, as
axum drops a handler's future on disconnect). `services::lab_operations` gives each lab a lease and
a cancellation token: operations share the read lease, so the lab's phase, not the lease, keeps them
apart, and `REDEPLOY_LOCKS` serialises only a lab's redeploys with each other; destroy, clean, a requested stop and the scanner take the write lease. The token is checked
only at safe points, as disk and VM steps run in `spawn_blocking` tasks that a dropped future would
leave running. VM image leases end once every disk and domain exists, so a long readiness wait
does not block image changes.

### Admission: memory and CPU pins

`services::admission` admits the nodes an operation will start, one request at a time, before
anything is created: nothing else serializes operations across labs. Memory is compared with what
is committed, not with free memory, as QEMU allocates guest memory only once touched: every running
domain's and limited container's maximum, Sherpa's and others', plus the pending, against the total
less hugepages and `host_memory.reserve_mib`, times `overcommit_percent`; a VM bound strictly to a
NUMA node is checked against that node too. Pending memory is held in memory until libvirt counts it.

### CPU pins in admission

Admission checks pins against the `cpu_pin` table (see [`DATABASE.md`](DATABASE.md#cpu-pins)) to
refuse naming the holder, to admins and its owner only, then claims them; the trigger catches a
race. A redeploying node holds old and new pins until it keeps the old ones (while its old domain
may run) or the new (from `store_node_resources` on). Overlaps stored before the triggers are only
warned about, as an existing lab is never refused for its pins.

### Redeploy architecture

Redeploy replaces one node and keeps the lab's topology; redeploys of one lab serialize
(`REDEPLOY_LOCKS`), as each updates the shared definition. The node gets the image `up` would give
it. The web UI has only the stored definition, without file contents or resolved values, so it
refuses a node that needs them (`validate::check_web_redeployable`), naming the CLI command.

### Down/resume architecture

Resume acts on each node from its runtime alone (`start_step`), and its `StartOutcome`, not message
text, decides which nodes get their links back and are waited for. `down` asks VMs to shut down
before it forces them off, as forcing lost recent writes on VMs without a guest agent.

### Image service architecture

`delete_image` holds the image's lock through its checks and the delete, and asks libvirt for disks'
backing files, as sherpad cannot read disks handed to QEMU. A container image is `{repo}:{tag}` from
pull to create, never parsed: Docker resolves it, and the scan uses Docker's `reference` filter.

### Scanner service architecture

The scanner moves nodes only through `node_lifecycle::observe`, a compare-and-set on the state it
read, and only in labs no operation holds; it never creates or destroys anything. It probes before
taking the lab (`try_observe`), so a slow node never delays an operation, then queries the runtimes
again under the hold to see what an operation just did. A runtime it cannot query keeps its nodes'
states, as empty would mark them `Missing`. Under the hold it also repairs link ends made again
outside Sherpa and sets carrier (see [`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md#repair-by-the-scanner)).

## HTTP routing architecture

Auth is enforced by handler signatures and checks, not by route grouping in `api/router.rs`, so
review a route's protection in its handler. `build_router`'s doc comment says why there is no CORS.

## Lab lifecycle architecture

### Node readiness

A node's readiness check is stored on its record when it is built, and every probe reads it there.
On SSH an open port is not enough, as devices accept TCP before SSH answers, so `check_vm_ready`
completes key exchange through Jato, sending no credentials. The host key offered is recorded
unchecked: the server sees the node first, on its management network, and is the party users
already trust to vouch for it. The server keeps no SSH files; `ssh_config::lab_ssh_files` renders
them from the records on request.

### Lab management addressing

Each lab's subnets are split into fixed offset blocks in `shared::konst`: 1–10 infrastructure, 11–20
cluster control planes, 21–254 nodes in manifest order. Addresses are saved on the node records and
reused by redeploy and resume. The cases are `tests/specs/lab-lifecycle.md`, `lab_addressing`.

## Node consoles

Every serial and VNC console is a UNIX socket only sherpad reaches, under
`/opt/sherpa/run/console/<lab-id>/`, relayed to the owner over a WebSocket. The folder is writable
by `kvm`, as QEMU binds the VNC socket, and is on disk so defined domains still start after a
reboot. The routes take only the bearer token, as a browser sends its cookie with a WebSocket
upgrade from any site. Every refusal comes before the upgrade, from the same `check_console` that
`?check=true` runs. QEMU's VNC asks no password, so the folder and the owner-only route guard it.
TCP consoles of older domains move to sockets when stopped (`libvirt::move_consoles_to_sockets`).
See `tests/specs/consoles.md`.

## TLS/listener architecture

With TLS on, a separate plain listener serves only `/cert`, public by design: a client needs the
certificate before it can trust the server. Logging and OTel start before `AppState`, so backend
failures are logged; `[otel]` is in sherpa-docs `configuration/sherpa.md`.
