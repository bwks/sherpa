# Sherpa Application Server Architecture

`sherpad` is the Sherpa application server daemon. It is not just an HTTP wrapper around the old CLI path; it is the process that owns the server-side control plane for browser users, REST clients, WebSocket JSON-RPC clients, and the Sherpa CLI.

The server crate is `crates/server`. It sits above the infrastructure crates (`db`, `libvirt`, `container`, `network`, `template`, `topology`, `validate`) and coordinates them through explicit service modules under `crates/server/src/services`.

## System context

At runtime, Sherpa is a single server process with multiple public transports and several infrastructure backends.

```text
+------------------+   HTTPS + cookie / HTML + SSE   +---------------------------+
| Browser UI       | <------------------------------> |                           |
| Askama + HTMX    |                                  |                           |
+------------------+                                  |                           |
                                                      |          sherpad          |
+------------------+   WebSocket JSON-RPC             |       crates/server       |
| sherpa CLI       | <------------------------------> |                           |
| client crate     |   token in RPC params            |  Axum router             |
+------------------+                                  |  REST handlers           |
                                                      |  WebSocket RPC           |
+------------------+   REST + Bearer JWT              |  SSE progress streams    |
| API clients      | <------------------------------> |  service layer           |
+------------------+                                  |  scanner task            |
                                                      +-------------+-------------+
                                                                    |
                                                                    v
                                    +-------------------------------+-------------------------------+
                                    |                         Backend APIs                         |
                                    |                                                               |
                                    |  SurrealDB:      users, labs, nodes, links, image records     |
                                    |  libvirt/QEMU:   VM/unikernel domains, networks, disks        |
                                    |  Docker daemon:  containers, images, Docker networks          |
                                    |  Linux host:     bridges, veths, taps, impairment plumbing    |
                                    +---------------------------------------------------------------+
```

The public API surface is intentionally transport-agnostic at the data-model level. Request/response types live in `shared`, and the generated API registry in `crates/shared/src/api_spec.rs` describes how those operations map to REST, WebSocket RPC, and CLI commands. The implementation is not fully generated from the registry yet, but the registry is the intended public contract source of truth.

## High-level server layering

The server code is layered like this:

```text
+---------------------------------------------------------------------+
|                         Transport layer                              |
|                                                                     |
|  api/router.rs                                                      |
|  +- HTML routes: pages, forms, HTMX fragments                       |
|  +- REST routes: /api/v1/...                                        |
|  +- WebSocket route: /ws                                            |
|  `- static assets: rust_embed catch-all                             |
+-------------------------------+-------------------------------------+
                                |
                                v
+---------------------------------------------------------------------+
|                  Request/Auth/Response boundary                      |
|                                                                     |
|  api/handlers.rs                                                    |
|  api/extractors.rs                                                  |
|  api/websocket/rpc.rs                                               |
|  auth/{jwt,cookies,middleware,context}.rs                           |
|                                                                     |
|  Responsibilities:                                                  |
|  - parse path/query/body/form/multipart data                        |
|  - authenticate cookies, bearer tokens, or RPC params.token          |
|  - enforce admin/ownership checks                                   |
|  - construct service request structs                                |
|  - convert service results into HTML, JSON, SSE, or RPC responses    |
+-------------------------------+-------------------------------------+
                                |
                                v
+---------------------------------------------------------------------+
|                         Service layer                                |
|                                                                     |
|  services/up.rs          services/destroy.rs                        |
|  services/down.rs        services/resume.rs                         |
|  services/redeploy.rs    services/inspect.rs                        |
|  services/import.rs      services/container_pull.rs                 |
|  services/impairment.rs  services/scanner.rs                        |
|                                                                     |
|  Responsibilities:                                                  |
|  - orchestrate db/libvirt/docker/network/template/topology/validate  |
|  - emit progress updates for long-running operations                 |
|  - maintain operation-level rollback/best-effort cleanup semantics   |
|  - return shared request/response models                            |
+-------------------------------+-------------------------------------+
                                |
                                v
+---------------------------------------------------------------------+
|                    Infrastructure and domain crates                  |
|                                                                     |
|  db         SurrealDB schema and CRUD                               |
|  libvirt    QEMU/libvirt domain, network, storage operations         |
|  container  Docker/Bollard operations                               |
|  network    Linux bridges, veths, taps, impairment helpers           |
|  topology   manifest transformation                                 |
|  validate   validation of caller-provided manifest data              |
|  template   generated configs/XML/scripts                           |
|  shared     public models, constants, utilities, API spec            |
+---------------------------------------------------------------------+
```

The important boundary is between handlers and services. Handlers know about transports and authentication. Services know about orchestration. Infrastructure crates know how to talk to their backend, but should not know about HTTP, cookies, WebSocket message IDs, or HTMX fragments.

## Server startup architecture

The binary entry point is `crates/server/src/main.rs`. Its job is intentionally small: initialize rustls, detect the internal background-child mode, parse the `sherpad` CLI, and dispatch to daemon management.

```text
sherpad process start
        |
        v
main.rs
  +- install rustls ring crypto provider
  +- if argv[1] == "--background-child"
  |     `- daemon::manager::run_background_child()
  `- parse Cli from cli.rs
        |
        +- init      -> init::init(...)
        +- doctor    -> doctor::doctor(...)
        +- start     -> daemon::manager::start_daemon(foreground)
        +- stop      -> daemon::manager::stop_daemon(force)
        +- restart   -> daemon::manager::restart_daemon(foreground)
        +- status    -> daemon::manager::status_daemon()
        `- logs      -> daemon::manager::logs_daemon(follow)
```

Daemon mode is implemented in `crates/server/src/daemon/manager.rs`:

```text
sherpad start
    |
    +- ensure run/log dirs exist
    +- verify no live PID file exists
    |
    +- foreground=true
    |     +- write PID file
    |     +- run_server(true)
    |     `- remove PID file on exit
    |
    `- foreground=false
          +- spawn current executable with --background-child
          +- child writes its PID file
          +- parent verifies child is still alive
          `- parent exits
```

The real server runtime is built by `crates/server/src/daemon/server.rs`:

```text
run_server(foreground)
    |
    +- load sherpa.toml
    +- build tracing filter from RUST_LOG or default to info
    +- if [otel].enabled
    |     +- initialize OTLP trace exporter
    |     +- initialize OTLP metric exporter
    |     `- attach tracing-opentelemetry layer
    +- configure logging
    |     +- foreground: compact stdout/stderr formatter
    |     `- daemon: compact file formatter
    +- apply env overrides
    |     +- SHERPA_SERVER_IPV4
    |     +- SHERPA_SERVER_IPV6
    |     +- SHERPA_SERVER_WS_PORT
    |     `- SHERPA_SERVER_HTTP_PORT
    +- AppState::new(config, metrics)
    |     +- load/generate JWT secret
    |     +- connect SurrealDB
    |     +- apply DB schema
    |     +- optionally seed admin user
    |     +- initialize Qemu wrapper
    |     +- connect Docker client
    |     `- create WebSocket registry and pending-job map
    +- create CancellationToken for background services
    +- if scanner enabled: spawn services::scanner::run_scanner(...)
    +- build Axum router and add /ws route
    +- if TLS enabled
    |     +- compute certificate SANs if none configured
    |     +- ensure certificate/key exist
    |     +- start HTTP /cert listener
    |     +- optionally start IPv6 HTTP /cert listener
    |     +- optionally spawn IPv6 TLS listener
    |     `- serve main IPv4 TLS listener with graceful shutdown handle
    `- if TLS disabled
          +- bind plain IPv4 listener
          +- optionally spawn IPv6 listener
          `- serve with graceful shutdown future
```

Shutdown uses SIGINT/SIGTERM. On shutdown, the server cancels the background-service token. With TLS, it also asks the `axum_server::Handle` to perform graceful shutdown with a short timeout. The OpenTelemetry guard flushes traces/metrics when dropped.

## Runtime state: `AppState`

`AppState` is the dependency graph passed into Axum handlers and cloned into async tasks. It is defined in `crates/server/src/daemon/state.rs`.

```text
                         +------------------------------+
                         |           AppState            |
                         +--------------+---------------+
                                        |
        +-------------------------------+-------------------------------+
        |                               |                               |
        v                               v                               v
+----------------+              +----------------+              +----------------+
| db             |              | qemu           |              | docker         |
| Arc<Surreal>   |              | Arc<Qemu>      |              | Arc<Docker>    |
|                |              |                |              |                |
| users/labs     |              | domains        |              | containers     |
| nodes/images   |              | networks/disks |              | images/networks|
+----------------+              +----------------+              +----------------+
                                        |
                                        v
+------------------------------------------------------------------------+
| Other shared state                                                     |
|                                                                        |
| config, jwt_secret, WebSocket connections, metrics, pending SSE jobs   |
+------------------------------------------------------------------------+
```

The state object deliberately holds clients rather than opening new connections in every handler. Some libvirt operations still create short-lived libvirt connections through `Qemu::connect()` because libvirt calls are blocking FFI-style operations and are wrapped at the libvirt crate boundary.

### AppState fields

| Field | Architectural role |
|---|---|
| `connections` | Active WebSocket connection registry. Used for connection lifecycle and log/status subscriptions. |
| `db` | Shared SurrealDB remote WebSocket client. All persistent user/lab/node/image state flows through this. |
| `qemu` | Shared QEMU/libvirt wrapper. Service modules call into the `libvirt` crate for VM, unikernel, storage, and network operations. |
| `docker` | Shared Bollard Docker client. Container services use this instead of shelling out to Docker. |
| `config` | Immutable runtime configuration loaded from `sherpa.toml` after env overrides. |
| `jwt_secret` | Secret used by JWT login, cookie auth, REST bearer auth, and RPC token auth. |
| `metrics` | OTel metric instruments or no-op instruments when OTel is disabled. |
| `pending_jobs` | A small one-shot job handoff registry for HTML form submissions that redirect to a job page and then open an SSE stream. |

## Transport architecture

Sherpa has three main public transports and one browser-specific HTML flow.

```text
                             +--------------------+
                             |  build_router()     |
                             | api/router.rs       |
                             +---------+----------+
                                       |
        +------------------------------+------------------------------+
        |                              |                              |
        v                              v                              v
+----------------+             +----------------+             +------------------+
| HTML/HTMX UI   |             | REST JSON API  |             | WebSocket RPC    |
| Askama output  |             | /api/v1/...    |             | /ws              |
+-------+--------+             +-------+--------+             +--------+---------+
        |                              |                               |
        | cookie auth                  | bearer/cookie auth             | params.token auth
        v                              v                               v
+----------------+             +----------------+             +------------------+
| HTML handlers  |             | REST handlers  |             | rpc.rs dispatcher|
| templates.rs   |             | Json<T>/SSE    |             | ServerMessage    |
+-------+--------+             +-------+--------+             +--------+---------+
        |                              |                               |
        +------------------------------+-------------------------------+
                                       v
                              +-----------------+
                              | services/*.rs   |
                              +-----------------+
```

### HTML/HTMX UI

The browser UI is not a separate frontend application. It is server-rendered HTML:

- Askama template structs are defined in `crates/server/src/templates.rs`.
- Template files live in `crates/server/templates/`.
- Static assets are embedded from `crates/server/web/static/` using `rust_embed`.
- HTMX is used for form submissions, partial updates, node-table polling, lab-grid updates, and progress pages.

HTML routes generally authenticate with `AuthenticatedUserFromCookie`. When auth fails, the extractor redirects to `/login?error=session_required`.

The HTML job/progress flow looks like this:

```text
Browser submits form
        |
        v
HTML POST handler
        |
        +- authenticate cookie
        +- validate/shape input
        +- create Job { lab_name, JobType }
        +- insert into AppState.pending_jobs[job_id]
        `- redirect browser to /jobs/{job_id}
                |
                v
        Browser opens /jobs/{job_id}/stream
                |
                v
        job_stream_handler consumes pending_jobs[job_id]
                |
                +- spawn service task
                +- service writes progress to ProgressSender
                `- api/sse.rs renders HTML SSE fragments for HTMX
```

That `pending_jobs` map is intentionally a transient handoff, not persistent job storage. If the process dies, pending jobs are lost.

### REST JSON API

REST handlers live in `api/handlers.rs`. They use Axum extractors (`Path`, `Query`, `Json`, `Multipart`, `State`) and return either `Json<T>`, generic `IntoResponse`, or SSE streams.

REST auth is implemented by `AuthenticatedUser` in `api/extractors.rs`. It tries bearer auth first and then cookie auth. Admin-only REST handlers call `require_admin_auth(&auth)` inside the handler.

Important current behavior: `GET /api/v1/labs?username=...` is public in code. The handler takes no `AuthenticatedUser`. This document records the current implementation; it is not a recommendation for future authorization design.

### WebSocket JSON-RPC

The WebSocket stack lives under `api/websocket/`:

```text
GET /ws
  |
  v
handler.rs::ws_handler
  | upgrades connection
  v
handle_socket_inner
  +- split WebSocket sender/receiver
  +- create Connection { id, sender, subscribed_logs }
  +- insert into AppState.connections
  +- increment ws connection metric
  +- send ConnectedMsg
  `- loop over incoming ClientMessage
        |
        +- SubscribeLogs / UnsubscribeLogs
        +- Pong
        `- RpcRequest { id, method, params }
              |
              +- if streaming method:
              |     spawn rpc::handle_streaming_rpc_request(...)
              `- else:
                    spawn rpc::handle_rpc_request(...)
```

Streaming RPC methods currently include:

- `up`
- `destroy`
- `redeploy`
- `image.import`
- `image.pull`
- `image.download`

Regular RPC methods return one `ServerMessage::RpcResponse`. Streaming methods send zero or more `ServerMessage::Status` values, followed by one final `ServerMessage::RpcResponse`.

### SSE progress architecture

Long-running operations use a common progress channel pattern:

```text
REST/HTML handler or WebSocket RPC handler
        |
        +- create mpsc::unbounded_channel<Message>()
        +- create ProgressSender(tx)
        +- spawn service task or call service directly
        |      |
        |      `- service calls progress.send_status/send_phase(...)
        |
        `- stream receiver
              |
              +- REST JSON SSE: api/sse.rs::json_progress_stream
              +- HTML SSE:     api/sse.rs::up_progress_stream / destroy_progress_stream
              `- WebSocket:    forwarding task sends Message::Text to Connection
```

The service layer does not know if progress is ultimately going to a browser SSE stream, REST SSE stream, or WebSocket. It only sees `ProgressSender`.

## Authentication and authorization architecture

Authentication has three input paths but one shared JWT validation model.

```text
+--------------------+       +--------------------+       +--------------------+
| Browser HTML       |       | REST API           |       | WebSocket RPC      |
| Cookie: sherpa_auth|       | Bearer or cookie   |       | params.token       |
+---------+----------+       +---------+----------+       +---------+----------+
          |                            |                            |
          v                            v                            v
+--------------------+       +--------------------+       +--------------------+
| AuthenticatedUser  |       | AuthenticatedUser  |       | authenticate_req   |
| FromCookie/Admin   |       | api/extractors.rs  |       | auth/middleware.rs |
| api/extractors.rs  |       |                    |       |                    |
+---------+----------+       +---------+----------+       +---------+----------+
          |                            |                            |
          +----------------------------+----------------------------+
                                       |
                                       v
                            +----------------------+
                            | jwt::validate_token  |
                            | auth/jwt.rs          |
                            +----------+-----------+
                                       |
                                       v
                            +----------------------+
                            | AuthContext / claims |
                            | username + is_admin  |
                            +----------+-----------+
                                       |
                                       v
                            +----------------------+
                            | handler/RPC authz    |
                            | admin + ownership    |
                            +----------------------+
```

The JWT contains the subject username and admin flag. Authorization is then done in handlers/RPC dispatchers:

- Browser page access uses cookie-only extractors.
- REST API access uses bearer-or-cookie extractors.
- WebSocket RPC access requires `params.token`.
- Admin-only operations reject non-admin users.
- Lab operations check ownership unless the caller is admin.
- User password/info operations allow admins to target anyone and regular users to target themselves.
- Self-delete is rejected.
- Deleting the last admin is rejected.

The service layer is not a clean authorization boundary today. Some services, such as `destroy_lab`, still validate ownership internally from the username in the request. Others assume the handler/RPC dispatcher has already made the authorization decision. New work should prefer the explicit pattern: authenticate and authorize at the transport boundary, then pass a verified username/admin context into services.

## Service layer architecture

The services directory is the orchestration layer. Each service is built around a public async function that takes typed request data, `&AppState`, and sometimes a `ProgressSender`.

```text
+--------------------------------------------------------------------------------+
|                                services/*.rs                                    |
|                                                                                |
|  orchestration, progress reporting, rollback, best-effort cleanup, summaries    |
+-------------------------+--------------------------+---------------------------+
                          |                          |
                          |                          |
                          v                          v
+-------------------------+        +-----------------+---------------------------+
| Persistence             |        | Runtime I/O                                 |
|                         |        |                                             |
| - db crate              |        | - libvirt crate: domains, disks, networks   |
| - SurrealDB records     |        | - container crate: Docker/Bollard           |
| - schema + CRUD         |        | - network crate: bridges, veths, taps       |
+-------------------------+        +-----------------+---------------------------+
                          |                          |
                          |                          v
                          |        +---------------------------------------------+
                          |        | Transformation and generation               |
                          |        |                                             |
                          |        | - topology crate: manifest expansion        |
                          |        | - validate crate: input validation          |
                          |        | - template crate: configs/XML/scripts       |
                          |        +---------------------------------------------+
                          |
                          v
+--------------------------------------------------------------------------------+
| shared crate: public request/response models, constants, utility helpers, spec  |
+--------------------------------------------------------------------------------+
```

The services are not symmetrical. Some are short CRUD-style orchestrators, and some are large lifecycle engines.

### Service categories

```text
Lifecycle services
  +- up.rs          create a full lab and all resources
  +- destroy.rs     remove a full lab and all resources
  +- redeploy.rs    replace one node inside an existing lab
  +- down.rs        stop all nodes or one node
  `- resume.rs      start all nodes or one node

Read/model services
  +- inspect.rs     read DB + runtime data and build lab inspection output
  +- list_labs.rs   list lab summaries for a user
  `- download.rs    package saved lab files for client download

Image/admin services
  +- import.rs          image import/list/show/set-default/scan/download support
  +- container_pull.rs  Docker/OCI image pull with progress
  `- clean.rs           admin force-clean path

Network mutation services
  `- impairment.rs  update delay/jitter/loss/reorder/corrupt settings on P2P links

Background service
  `- scanner.rs     reconcile runtime state from Docker/libvirt into SurrealDB

Shared helpers
  +- node_ops.rs    common node setup/building helpers
  `- progress.rs    progress message abstraction
```

### Service call graph for lab creation

`up.rs` is the largest orchestration module. Its job is to convert a validated manifest into persistent records, generated files, runtime networks, and running nodes.

```text
up_lab(UpRequest, &AppState, ProgressSender)
    |
    +- Deserialize manifest JSON into topology::Manifest
    +- Load current user from DB
    +- Guard: lab_id must not already exist
    +- Load node image records from DB
    |
    +- Manifest validation and expansion
    |   +- validate::check_duplicate_device
    |   +- validate environment variables
    |   +- container::get_local_images
    |   +- validate::validate_and_resolve_node_versions
    |   +- process_manifest_nodes
    |   +- process_manifest_links
    |   +- process_manifest_bridges
    |   +- validate management/reserved/data interface bounds
    |   `- validate duplicate links and bridge endpoints
    |
    +- Persistent model construction
    |   +- allocate IPv4 management subnet
    |   +- allocate IPv4 loopback subnet
    |   +- allocate IPv6 management subnet
    |   +- allocate IPv6 loopback subnet
    |   +- db::create_lab
    |   +- db::create_node for each node
    |   `- db::create_link for each link
    |
    +- Filesystem/model artifacts
    |   +- create /opt/sherpa/labs/{lab_id}
    |   +- write lab info file
    |   +- save manifest for redeploy
    |   +- create node configs
    |   +- create ZTP/TFTP files where needed
    |   +- create lab certs where needed
    |   `- write SSH/config helper files
    |
    +- Runtime network creation
    |   +- libvirt NAT management network
    |   +- Docker bridge management network
    |   +- per-node isolated/reserved networks
    |   +- P2P bridge/veth/tap topology
    |   +- Docker macvlan networks for container bridge links
    |   `- Linux host interfaces via network crate
    |
    +- Runtime node creation
    |   +- create/start containers through container crate
    |   +- create/start VMs through libvirt crate
    |   +- create/start unikernels through libvirt/QEMU path
    |   +- attach P2P/eBPF/netns wiring where required
    |   `- run readiness checks unless skipped
    |
    +- On resource-phase failure
    |   `- attempt cleanup of partially-created resources
    |
    `- Return UpResponse { success, lab_info, summary, errors, total_time_secs }
```

The central architectural decision in creation is that the manifest is transformed into fully-expanded intermediate structures before resource creation. That avoids spreading manifest parsing rules across Docker/libvirt/network calls. Validation happens before resource-creating phases where possible; resource phases still need cleanup handling because external backends can fail mid-operation.

### Lab creation data flow

```text
manifest.toml / uploaded manifest JSON
        |
        v
topology::Manifest
        |
        +- topology expansion
        |      +- NodeExpanded[]
        |      +- LinkDetailed[]
        |      `- BridgeDetailed[]
        |
        +- validate crate checks caller-supplied data
        |
        +- db crate persists desired state
        |      +- lab record
        |      +- node records
        |      `- link/interface records
        |
        +- template crate generates config/domain assets
        |
        `- runtime crates realize state
               +- libvirt: VM/unikernel domains, NAT networks, disks
               +- container: Docker containers/images/networks
               `- network: bridges, veths, taps, impairment plumbing
```

### Service call graph for lab destruction

`destroy.rs` is deliberately best-effort. It should attempt every cleanup category even if an earlier category fails.

```text
destroy_lab(DestroyRequest, &AppState, ProgressSender)
    |
    +- Load user from DB
    +- Load lab from DB
    +- Verify DB lab ownership against request.username
    +- Load lab info file from /opt/sherpa/labs/{lab_id}
    |
    +- destroy_containers
    |   +- container::list_containers
    |   +- kill matching lab containers if running
    |   `- remove matching lab containers
    |
    +- destroy_vms_and_disks
    |   +- list libvirt domains
    |   +- undefine/destroy domains containing lab_id
    |   +- lookup Sherpa storage pool
    |   `- delete volumes containing lab_id
    |
    +- destroy_docker_networks
    |   +- container::list_networks
    |   `- delete networks containing lab_id
    |
    +- destroy_libvirt_networks
    |   +- list libvirt networks
    |   `- destroy/undefine networks containing lab_id
    |
    +- destroy_interfaces
    |   +- network::find_interfaces_fuzzy(lab_id)
    |   `- delete known Sherpa-created bridge/veth/tap interfaces
    |
    +- cleanup_database
    |   +- db::delete_lab_links
    |   +- db::delete_lab_nodes
    |   `- db::delete_lab
    |
    +- remove lab directory
    `- Return DestroyResponse { success, summary, errors }
```

Destruction uses name-based matching for many runtime resources because the runtime backends are external systems and the database may not be fully reliable after partial failures. This is why consistent naming with `lab_id` embedded in resource names is part of the architecture.

### Redeploy architecture

Redeploy is a controlled single-node replacement. It exists because updating node configuration in place is often less reliable than destroying and recreating that node.

```text
redeploy request
   |
   +- auth/ownership check at handler/RPC boundary
   +- load saved manifest from lab directory for web UI flow
   +- validate target lab and node
   +- stop/remove current runtime node resources
   +- regenerate node-specific config/assets from manifest
   +- recreate node runtime resource
   +- reconnect node to existing lab networks/links
   +- update DB node state/details
   `- stream progress and final RedeployResponse
```

The important boundary is that redeploy should preserve the lab-level network topology and only replace the selected node's runtime resources and generated files.

### Down/resume architecture

`down.rs` and `resume.rs` operate at two scopes:

```text
POST /api/v1/labs/{id}/down              +
POST /api/v1/labs/{id}/resume            +- node_name = None  -> all nodes in lab
POST /labs/{id}/stop                     |
POST /labs/{id}/start                    +

POST /labs/{id}/nodes/{node}/stop        +
POST /labs/{id}/nodes/{node}/start       +- node_name = Some -> one node
RPC down/resume with node_name optional  +
```

Handlers authorize ownership first, then services inspect node kind and call the right runtime backend: Docker for containers and libvirt for VMs/unikernels. The service returns a `LabNodeActionResponse` describing what changed and what failed.

### Image service architecture

Image management is split by image kind and source:

```text
Image operations
    |
    +- import.rs
    |   +- import local VM/unikernel disk image metadata/artifact
    |   +- import local container tar/archive where supported
    |   +- list image records
    |   +- show image record details
    |   +- set default image version
    |   +- scan filesystem/Docker for discoverable images
    |   `- download image from URL with progress
    |
    +- container_pull.rs
    |   `- pull OCI image through Docker/Bollard with streamed status
    |
    `- delete.rs
        `- remove imported image records/artifacts
```

Admin-only image mutations are enforced at the transport boundary. Image list/show require authentication but not admin privileges. Long-running import/pull/download paths use `ProgressSender` so REST, WebSocket, and UI callers can receive progress without service-specific transport code.

### Scanner service architecture

The scanner is the only long-running background service started by `run_server` today.

```text
run_server
  |
  +- create CancellationToken
  `- if config.scanner.enabled
        `- tokio::spawn(run_scanner(AppState, child_token))

run_scanner
  |
  +- sleep interval_secs between cycles
  +- stop immediately when cancellation token is triggered
  `- scan_cycle
        |
        +- query_libvirt_states(qemu)
        |     `- spawn_blocking because libvirt calls are blocking
        |
        +- query_docker_states(docker)
        |     `- Bollard list_containers
        |
        +- db::list_labs
        +- for each lab:
        |     +- db::list_nodes_by_lab
        |     +- batch load node images for NodeKind
        |     +- map runtime state to NodeState
        |     +- db::update_node_state when changed
        |     +- derive LabState from node states
        |     `- db::update_lab_state when changed
        `- log but do not kill server on scan failure
```

The scanner is reconciliation, not orchestration. It should not create or destroy resources. It observes runtime state and updates the database so UI/API consumers see reality when resources are externally stopped, started, removed, or crash.

## HTTP routing architecture

`api/router.rs` builds a single Axum router. It attaches CORS, HTTP tracing, embedded static asset handling, and the route table. The WebSocket route is added in `daemon/server.rs` after `build_router()` returns.

```text
build_router()
    |
    +- Public browser routes
    |   +- GET/POST /login
    |   +- GET/POST /signup
    |   `- POST /logout
    |
    +- Protected browser routes
    |   +- GET /
    |   +- GET /labs, /labs/grid, /labs/create
    |   +- GET /jobs/{job_id}, /jobs/{job_id}/stream
    |   +- GET /labs/{lab_id}
    |   +- GET /labs/{lab_id}/nodes[/node]
    |   +- POST lab/node start/stop/redeploy/destroy actions
    |   `- GET/POST profile and SSH-key routes
    |
    +- Admin browser routes
    |   +- /admin/users...
    |   +- /admin/labs
    |   +- /admin/tools...
    |   `- /admin/images...
    |
    +- Public API routes
    |   +- GET /health
    |   +- GET /cert
    |   +- GET /api/v1/spec
    |   +- GET /api/v1/openapi.json
    |   +- GET /api/docs
    |   `- POST /api/v1/auth/login
    |
    +- REST API routes
    |   +- labs: create/inspect/delete/down/resume/redeploy
    |   +- links: impairment update
    |   +- images: list/show/import/upload/delete/default/pull/download
    |   +- admin tools: clean/scan
    |   `- users: create/list/info/delete/password
    |
    +- layers
    |   +- CorsLayer mirror_request + credentials
    |   `- TraceLayer::new_for_http()
    |
    `- catch-all static asset route /{*path}
```

Auth is enforced by handler signatures and handler-local checks, not by route grouping. When reviewing a route's protection level, check the handler parameters and body rather than only the route table.

## Public API specification architecture

The generated API metadata is built from `crates/shared/src/api_spec.rs`.

```text
shared::api_spec::build_spec()
    |
    +- build_operations()
    |     `- OperationDef[]
    |          +- operation name
    |          +- auth requirement
    |          +- streaming flag
    |          +- request/response schema names
    |          `- REST/RPC/CLI bindings
    |
    +- build_schemas()
    |     `- schemars JSON schemas from shared data types
    |
    `- ApiSpec
          |
          +- GET /api/v1/spec
          `- build_openapi()
                `- GET /api/v1/openapi.json
```

`/api/docs` serves embedded Swagger UI backed by the OpenAPI endpoint.

Current implementation note: the registry marks five canonical operations as streaming (`lab.create`, `lab.destroy`, `node.redeploy`, `image.pull`, `image.download`), while the current REST/RPC handlers also stream `image.import`. That mismatch should be resolved in `api_spec.rs` if streaming import is the intended public contract.

## Lab lifecycle architecture

### Create lifecycle phases

The creation path is best understood as a staged compiler pipeline: manifest input is parsed, validated, transformed, persisted, rendered to files, and finally realized into runtime resources.

```text
                    +---------------+
                    | Manifest input |
                    +-------+-------+
                            v
                    +---------------+
                    | Deserialize   |
                    | topology model|
                    +-------+-------+
                            v
                    +---------------+
                    | Validate      |
                    | no resources  |
                    +-------+-------+
                            v
                    +---------------+
                    | Expand model  |
                    | nodes/links   |
                    +-------+-------+
                            v
                    +---------------+
                    | Persist DB    |
                    | desired state |
                    +-------+-------+
                            v
                    +---------------+
                    | Generate files|
                    | configs/certs |
                    +-------+-------+
                            v
       +--------------------+--------------------+
       v                    v                    v
+--------------+     +--------------+     +--------------+
| libvirt      |     | Docker       |     | host network |
| networks/dom |     | nets/ctrs    |     | bridge/veth  |
+------+-------+     +------+-------+     +------+-------+
       +--------------------+--------------------+
                            v
                    +---------------+
                    | Start nodes   |
                    | readiness     |
                    +-------+-------+
                            v
                    +---------------+
                    | UpResponse    |
                    +---------------+
```

Progress phases emitted by `up_lab` correspond to these stages. Fail-fast validation happens before resource creation where possible. Once resource creation starts, the service must assume partial success is possible and clean up on failures.

### Runtime resource model for a lab

A running lab is spread across several systems. The DB is the desired/known state, not the only state.

```text
Lab {lab_id}
  |
  +- SurrealDB
  |   +- user record
  |   +- lab record
  |   +- node records
  |   +- link records
  |   `- image records
  |
  +- Filesystem: /opt/sherpa/labs/{lab_id}
  |   +- lab info file
  |   +- saved manifest
  |   +- generated node configs
  |   +- ZTP/TFTP assets
  |   +- cert material where needed
  |   `- SSH/helper config files
  |
  +- libvirt/QEMU
  |   +- VM domains
  |   +- unikernel domains
  |   +- NAT/isolated/reserved networks
  |   `- cloned/resized disks in Sherpa storage pool
  |
  +- Docker
  |   +- containers
  |   +- bridge/macvlan networks
  |   `- pulled/imported container images
  |
  `- Linux host networking
      +- bridges
      +- veth pairs
      +- tap devices
      `- impairment/eBPF/netns wiring where applicable
```

Destroy and clean paths must account for all of these locations. That is why cleanup is not a single database delete.

### Destroy lifecycle phases

```text
DestroyRequest { lab_id, username }
          |
          v
  load user + lab from DB
          |
          v
  verify ownership
          |
          v
  load lab info file
          |
          v
+-----------------------------------------------------+
| best-effort cleanup sequence                         |
|                                                     |
| 1. Docker containers                                |
| 2. libvirt domains and disks                         |
| 3. Docker networks                                  |
| 4. libvirt networks                                 |
| 5. Linux host interfaces                            |
| 6. SurrealDB lab/link/node records                   |
| 7. /opt/sherpa/labs/{lab_id} directory               |
+-----------------------------------------------------+
          |
          v
DestroyResponse
  +- success = errors.is_empty()
  +- summary of each cleanup category
  `- detailed DestroyError[] for failed categories
```

The best-effort rule is important: failing to delete a disk should not prevent the server from attempting to delete Docker networks or database records. Operators need a complete summary of what was cleaned and what still requires manual repair.

## TLS/listener architecture

TLS is enabled by default. The main listener serves the UI, REST API, and WebSocket endpoint. When TLS is enabled, a separate HTTP listener is started only for certificate retrieval.

```text
TLS enabled
   |
   +- main listener
   |     +- IPv4: wss/https on config.server_ipv4:config.ws_port
   |     `- IPv6: optional wss/https on [config.server_ipv6]:config.ws_port
   |
   `- cert listener
         +- IPv4: http on config.server_ipv4:config.http_port
         `- IPv6: optional http on [config.server_ipv6]:config.http_port

TLS disabled
   |
   `- main listener only
         +- IPv4: ws/http
         `- IPv6: optional ws/http
```

Certificate SAN generation is part of server startup. If no SANs are configured, startup adds listen IPs, non-loopback interface IPs when listening on unspecified addresses, localhost, loopbacks, hostname, and FQDN when available.

The `/cert` endpoint is public by design because clients need to fetch the certificate before they can establish trust. If TLS is disabled, `/cert` returns a service-unavailable JSON response.

## Observability architecture

Logging and OTel are initialized before `AppState` so startup failures and backend connection failures are visible.

```text
run_server
  |
  +- EnvFilter from RUST_LOG or info
  +- if OTel enabled
  |     +- SdkTracerProvider + OTLP span exporter
  |     +- SdkMeterProvider + periodic OTLP metric exporter
  |     +- tracing_opentelemetry layer
  |     `- Metrics::new(meter)
  |
  +- if foreground
  |     `- tracing fmt layer to terminal
  |
  `- if daemon
        `- tracing fmt layer to sherpad log file
```

Important spans/metrics are created at these levels:

- HTTP requests through `tower_http::TraceLayer`.
- WebSocket connections through a `ws_connection` span.
- RPC calls through an `rpc_call` span.
- Public service operations through `#[instrument]` spans.
- Database, Docker, libvirt, and network helper functions where instrumented in their crates.
- Metrics for WebSocket connection count, RPC duration, service operation duration, and errors.

See [`OTEL.md`](OTEL.md) for collector configuration and exported metric names.

## Current architectural rough edges

These are not documentation features; they are real implementation seams to keep in mind when modifying the server.

1. **Auth is handler-driven, not route-group driven.** The route table alone does not tell you if a route is protected. Check handler signatures and `require_admin_auth` calls.
2. **Service authorization is inconsistent.** Some services validate ownership internally; others rely on handlers/RPC dispatchers. New work should move toward a consistent verified-auth-context pattern.
3. **The generated API spec and implementation disagree on `image.import` streaming.** The implementation streams it; the registry does not mark it streaming.
4. **`pending_jobs` is in-memory only.** HTML progress jobs do not survive process restart.
5. **Resource cleanup depends heavily on naming conventions.** Runtime resources must include `lab_id` consistently or destroy/clean paths may miss them.
6. **libvirt calls are blocking.** Code that performs broad libvirt scans should use blocking-safe patterns like the scanner does.

## Source map

| Concern | Primary file(s) |
|---|---|
| Binary dispatch | `crates/server/src/main.rs` |
| CLI shape | `crates/server/src/cli.rs` |
| Daemon/PID/log management | `crates/server/src/daemon/manager.rs`, `pidfile.rs` |
| Server runtime setup | `crates/server/src/daemon/server.rs` |
| Shared state | `crates/server/src/daemon/state.rs` |
| Metrics | `crates/server/src/daemon/metrics.rs` |
| Router | `crates/server/src/api/router.rs` |
| REST/HTML handlers | `crates/server/src/api/handlers.rs` |
| REST auth extractors | `crates/server/src/api/extractors.rs` |
| SSE conversion | `crates/server/src/api/sse.rs` |
| WebSocket lifecycle | `crates/server/src/api/websocket/handler.rs`, `connection.rs`, `messages.rs` |
| RPC dispatch | `crates/server/src/api/websocket/rpc.rs` |
| JWT/cookies/auth context | `crates/server/src/auth/` |
| Lab create | `crates/server/src/services/up.rs` |
| Lab destroy | `crates/server/src/services/destroy.rs` |
| Node/lab stop/start | `crates/server/src/services/down.rs`, `resume.rs` |
| Redeploy | `crates/server/src/services/redeploy.rs` |
| Inspect/list/download | `crates/server/src/services/inspect.rs`, `list_labs.rs`, `download.rs` |
| Image management | `crates/server/src/services/import.rs`, `container_pull.rs`, `delete.rs` |
| Link impairment | `crates/server/src/services/impairment.rs` |
| Scanner | `crates/server/src/services/scanner.rs` |
| TLS certificates | `crates/server/src/tls/` |
| Generated API registry | `crates/shared/src/api_spec.rs` |
