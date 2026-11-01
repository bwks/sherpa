# Sherpa Application Server Architecture

`sherpad` is the Sherpa application server daemon. It is not just an HTTP wrapper around the old CLI path; it is the process that owns the server-side control plane for browser users, REST clients, WebSocket JSON-RPC clients, and the Sherpa CLI.

The server crate is `crates/server`. It sits above the infrastructure crates (`db`, `libvirt`, `container`, `network`, `template`, `topology`) and coordinates them through explicit service modules under `crates/server/src/services`.

## System context

At runtime, Sherpa is a single server process with multiple public transports and several infrastructure backends.

```mermaid
flowchart LR
    browser["Browser UI<br/>Askama + HTMX"]
    cli["sherpa CLI<br/>client crate"]
    apiclients["API clients"]
    sherpad["sherpad<br/>crates/server<br/>Axum router<br/>REST handlers<br/>WebSocket RPC<br/>SSE progress streams<br/>service layer<br/>scanner task"]
    subgraph backends["Backend APIs"]
        surreal["SurrealDB:<br/>users, labs, nodes, links, image records"]
        libvirt["libvirt/QEMU:<br/>VM/unikernel domains, networks, disks"]
        docker["Docker daemon:<br/>containers, images, Docker networks"]
        linuxhost["Linux host:<br/>bridges, veths, taps, impairment plumbing"]
    end
    browser <-->|"HTTPS + cookie / HTML + SSE"| sherpad
    cli <-->|"WebSocket JSON-RPC<br/>token in RPC params"| sherpad
    apiclients <-->|"REST + Bearer JWT"| sherpad
    sherpad --> backends
```

The public API surface is intentionally transport-agnostic at the data-model level. Request/response types live in `shared`, and the generated API registry in `crates/shared/src/api_spec.rs` describes how those operations map to REST, WebSocket RPC, and CLI commands. The implementation is not fully generated from the registry yet, but the registry is the intended public contract source of truth.

## High-level server layering

The server code is layered like this:

```mermaid
flowchart TB
    subgraph transport["Transport layer"]
        router["api/router.rs<br/>HTML routes: pages, forms, HTMX fragments<br/>REST routes: /api/v1/...<br/>WebSocket route: /ws<br/>static assets: rust_embed catch-all"]
    end
    subgraph boundary["Request/Auth/Response boundary"]
        boundaryfiles["api/handlers.rs<br/>api/extractors.rs<br/>api/websocket/rpc.rs<br/>auth/{jwt,cookies,middleware,context}.rs"]
        boundaryduties["Responsibilities:<br/>parse path/query/body/form/multipart data<br/>authenticate cookies, bearer tokens, or RPC params.token<br/>pass the caller's AuthContext to services<br/>construct service request structs<br/>convert service results into HTML, JSON, SSE, or RPC responses"]
    end
    subgraph servicelayer["Service layer"]
        servicefiles["services/up.rs, services/destroy.rs<br/>services/down.rs, services/resume.rs<br/>services/redeploy.rs, services/inspect.rs<br/>services/import.rs, services/container_pull.rs<br/>services/impairment.rs, services/scanner.rs"]
        serviceduties["Responsibilities:<br/>orchestrate db/libvirt/docker/network/template/topology/validate<br/>emit progress updates for long-running operations<br/>maintain operation-level rollback/best-effort cleanup semantics<br/>return shared request/response models"]
    end
    subgraph infra["Infrastructure and domain crates"]
        crates["db: SurrealDB schema and CRUD<br/>libvirt: QEMU/libvirt domain, network, storage operations<br/>container: Docker/Bollard operations<br/>network: Linux bridges, veths, taps, impairment helpers<br/>topology: manifest transformation<br/>topology::validate: validation of caller-provided manifest data<br/>template: generated configs/XML/scripts<br/>shared: public models, constants, utilities, API spec"]
    end
    transport --> boundary
    boundary --> servicelayer
    servicelayer --> infra
```

The important boundary is between handlers and services. Handlers know about transports and authentication. Services know about orchestration. Infrastructure crates know how to talk to their backend, but should not know about HTTP, cookies, WebSocket message IDs, or HTMX fragments.

## Server startup architecture

The binary entry point is `crates/server/src/main.rs`. Its job is intentionally small: initialize rustls, detect the internal background-child mode, parse the `sherpad` CLI, and dispatch to daemon management.

```mermaid
flowchart TB
    start["sherpad process start"] --> mainrs["main.rs"]
    mainrs --> rustls["install rustls aws-lc-rs crypto provider"]
    rustls --> bgcheck{"argv[1] == --background-child?"}
    bgcheck -->|yes| bgchild["daemon::manager::run_background_child()"]
    bgcheck -->|no| parse["parse Cli from cli.rs"]
    parse -->|init| init["init::init(...)"]
    parse -->|doctor| doctor["doctor::doctor(...)"]
    parse -->|start| startd["daemon::manager::start_daemon(foreground)"]
    parse -->|stop| stopd["daemon::manager::stop_daemon(force)"]
    parse -->|restart| restartd["daemon::manager::restart_daemon(foreground)"]
    parse -->|status| statusd["daemon::manager::status_daemon()"]
    parse -->|logs| logsd["daemon::manager::logs_daemon(follow)"]
```

Daemon mode is implemented in `crates/server/src/daemon/manager.rs`:

```mermaid
flowchart TB
    start["sherpad start"] --> dirs["ensure run/log dirs exist"]
    dirs --> pidcheck["verify no live PID file exists"]
    pidcheck --> mode{"foreground?"}
    mode -->|"foreground=true"| writepid["write PID file"]
    writepid --> runserver["run_server(true)"]
    runserver --> removepid["remove PID file on exit"]
    mode -->|"foreground=false"| spawnchild["spawn current executable with --background-child"]
    spawnchild --> childpid["child writes its PID file"]
    childpid --> verify["parent verifies child is still alive"]
    verify --> parentexit["parent exits"]
```

The real server runtime is built by `crates/server/src/daemon/server.rs`:

```mermaid
flowchart TB
    runserver["run_server(foreground)"] --> config["load sherpa.toml"]
    config --> filter["build tracing filter from RUST_LOG or default to info"]
    filter --> otel{"[otel].enabled?"}
    subgraph otelinit["OTel"]
        traceexp["initialize OTLP trace exporter"] --> metricexp["initialize OTLP metric exporter"] --> otellayer["attach tracing-opentelemetry layer"]
    end
    otel -->|yes| traceexp
    otel -->|no| logging
    otellayer --> logging{"configure logging"}
    logging -->|foreground| stdoutfmt["compact stdout/stderr formatter"]
    logging -->|daemon| filefmt["compact file formatter"]
    stdoutfmt --> signals
    filefmt --> signals
    signals["install SIGINT and SIGTERM handlers<br/>(startup fails if registration fails)"]
    signals --> envoverrides["apply env overrides<br/>SHERPA_SERVER_IPV4<br/>SHERPA_SERVER_IPV6<br/>SHERPA_SERVER_WS_PORT<br/>SHERPA_SERVER_HTTP_PORT"]
    envoverrides --> jwt
    subgraph appstate["AppState::new(config, metrics)"]
        jwt["load/generate JWT secret"] --> surreal["connect SurrealDB"] --> schema["apply DB schema"] --> seed["optionally seed admin user"] --> storage["restrict lab storage directories<br/>(daemon::storage::secure_lab_storage)"] --> qemu["initialize Qemu wrapper"] --> docker["connect Docker client"] --> registry["create WebSocket registry and pending-job map"]
    end
    registry --> cancel["create CancellationToken for background services"]
    cancel --> scanner["if scanner enabled:<br/>spawn services::scanner::run_scanner(...)"]
    scanner --> router["build Axum router and add /ws route"]
    router --> tls{"TLS enabled?"}
    tls -->|yes| sans["compute certificate SANs if none configured"]
    sans --> ensurecert["ensure certificate/key exist"]
    ensurecert --> certlistener["start HTTP /cert listener"]
    certlistener --> certlistener6["optionally start IPv6 HTTP /cert listener"]
    certlistener6 --> tls6["optionally spawn IPv6 TLS listener"]
    tls6 --> servetls["serve main IPv4 TLS listener with graceful shutdown handle"]
    tls -->|no| bindplain["bind plain IPv4 listener"]
    bindplain --> plain6["optionally spawn IPv6 listener"]
    plain6 --> serveplain["serve with graceful shutdown future"]
```

Shutdown uses SIGINT/SIGTERM. The handlers are registered once, before the server binds any listener, so a registration failure stops startup with an error instead of leaving a server that cannot shut down gracefully. On shutdown, the server cancels the background-service token. With TLS, it also asks the `axum_server::Handle` to perform graceful shutdown with a short timeout. The OpenTelemetry guard flushes traces/metrics when dropped.

### Lab storage permissions

Lab data is kept from other server users by directory modes, not file modes, so files written by
older releases are covered too. `daemon::storage::secure_lab_storage` runs on every start and
calls `restrict_dir` on:

| Directory | Mode and group | Constants |
|---|---|---|
| `/opt/sherpa/labs` | `2750`, the `sherpa` group | `SHERPA_LABS_DIR_MODE` |
| `/opt/sherpa/libvirt/images` (storage pool) | `2750`, `kvm` | `SHERPA_STORAGE_POOL_DIR_MODE`, `SHERPA_STORAGE_POOL_GROUP` |

- QEMU runs as `libvirt-qemu` in `kvm`, so it can reach the pool. Nothing reads `/opt/sherpa/labs`
  through the directory except `sherpad`: the dnsmasq router and container nodes get lab files
  through Docker bind mounts.
- File modes inside lab directories are left as they are. dnsmasq runs as root and serves only
  world-readable TFTP files, and container nodes may read their bind-mounted configs as a
  non-root user, so a restrictive umask would break ZTP.
- New pool volumes are created `0600` (`konst::STORAGE_VOLUME_MODE`, in
  `crates/libvirt/templates/copy.xml` and `overlay.xml`); libvirt hands them to the QEMU user
  while a VM runs. Flatcar's `.ign` file is `0644` (`STORAGE_VOLUME_QEMU_ARG_MODE`), because QEMU
  opens it from a raw `-fw_cfg` argument that libvirt does not relabel
  (`libvirt::vm::volume_format_and_mode`).
- A failure is logged, not fatal. `sherpad init` runs as the installing administrator, so it
  does not create these directories: one it created would not be owned by `sherpa`. The install
  and update scripts set the same owner and modes as root, and `scripts/fix-permissions.sh` keeps
  them.

### Container secret files

Container secret files (see [`PROVISIONERS.md`](PROVISIONERS.md)) are kept in RAM, under
`/run/sherpa`, the daemon's systemd `RuntimeDirectory` (`0700 sherpa`, a tmpfs).
`RuntimeDirectoryPreserve=yes` keeps it when `sherpad` stops or restarts, so running containers
keep their secrets; a reboot clears it. The install script writes these settings into the unit,
and the update script and `dev/rebuild` add them to older units as the drop-in
`sherpad.service.d/runtime-directory.conf`.

`services::node_secrets` writes `konst::SHERPA_SECRETS_DIR/<lab>/<node>/`: the lab directory is
`0700`, and the node directory, the bind mount's root, is `0755`. Nothing under `/run/sherpa` is
readable by other server users or the `sherpa` group.

## Runtime state: `AppState`

`AppState` is the dependency graph passed into Axum handlers and cloned into async tasks. It is defined in `crates/server/src/daemon/state.rs`.

```mermaid
flowchart TB
    appstate["AppState"]
    db["db<br/>Arc#lt;Surreal#gt;<br/>users/labs<br/>nodes/images"]
    qemu["qemu<br/>Arc#lt;Qemu#gt;<br/>domains<br/>networks/disks"]
    docker["docker<br/>Arc#lt;Docker#gt;<br/>containers<br/>images/networks"]
    other["Other shared state<br/>config, jwt_secret, WebSocket connections, metrics, pending SSE jobs"]
    appstate --> db
    appstate --> qemu
    appstate --> docker
    appstate --> other
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

```mermaid
flowchart TB
    router["build_router()<br/>api/router.rs"]
    html["HTML/HTMX UI<br/>Askama output"]
    rest["REST JSON API<br/>/api/v1/..."]
    ws["WebSocket RPC<br/>/ws"]
    htmlhandlers["HTML handlers<br/>templates.rs"]
    resthandlers["REST handlers<br/>Json#lt;T#gt;/SSE"]
    rpc["rpc.rs dispatcher<br/>ServerMessage"]
    services["services/*.rs"]
    router --> html
    router --> rest
    router -->|"route added in daemon/server.rs"| ws
    html -->|cookie auth| htmlhandlers
    rest -->|"bearer/cookie auth"| resthandlers
    ws -->|"params.token auth"| rpc
    htmlhandlers --> services
    resthandlers --> services
    rpc --> services
```

### HTML/HTMX UI

The browser UI is not a separate frontend application. It is server-rendered HTML:

- Askama template structs are defined in `crates/server/src/templates.rs`.
- Template files live in `crates/server/templates/`.
- Static assets are embedded from `crates/server/web/static/` using `rust_embed`.
- HTMX is used for form submissions, partial updates, node-table polling, lab-grid updates, and progress pages.

HTML routes generally authenticate with `AuthenticatedUserFromCookie`. When auth fails, the extractor redirects to `/login?error=session_required`.

The HTML job/progress flow looks like this:

```mermaid
sequenceDiagram
    participant B as Browser
    participant H as HTML POST handler
    participant P as AppState.pending_jobs
    participant S as job_stream_handler
    participant T as service task
    participant E as api/sse.rs
    B->>H: submit form
    H->>H: authenticate cookie
    H->>H: validate/shape input
    H->>H: create Job { lab_name, JobType }
    H->>P: insert pending_jobs[job_id]
    H-->>B: redirect to /jobs/{job_id}
    B->>S: open /jobs/{job_id}/stream
    S->>P: consume pending_jobs[job_id]
    S->>T: spawn service task
    T->>E: write progress to ProgressSender
    E-->>B: HTML SSE fragments for HTMX
```

That `pending_jobs` map is intentionally a transient handoff, not persistent job storage. If the process dies, pending jobs are lost.

### REST JSON API

REST handlers live in `api/handlers.rs`. They use Axum extractors (`Path`, `Query`, `Json`, `Multipart`, `State`) and return either `Json<T>`, generic `IntoResponse`, or SSE streams.

REST auth is implemented by `AuthenticatedUser` in `api/extractors.rs`. It tries bearer auth first and then cookie auth. Admin-only REST handlers call `require_admin_auth(&auth)` inside the handler.

`GET /api/v1/labs` requires authentication and lists the caller's labs. Admins may pass `?username=` to list another user's labs; regular users get 403 for any other username.

### WebSocket JSON-RPC

The WebSocket stack lives under `api/websocket/`:

```mermaid
flowchart TB
    get["GET /ws"] --> wshandler["handler.rs::ws_handler"]
    wshandler -->|upgrades connection| inner["handle_socket_inner"]
    inner --> split["split WebSocket sender/receiver"]
    split --> conn["create Connection { id, sender, subscribed_logs }"]
    conn --> insert["insert into AppState.connections"]
    insert --> metric["increment ws connection metric"]
    metric --> connected["send ConnectedMsg"]
    connected --> msgloop["loop over incoming ClientMessage"]
    msgloop --> subs["SubscribeLogs / UnsubscribeLogs"]
    msgloop --> pong["Pong"]
    msgloop --> rpcreq["RpcRequest { id, method, params }"]
    rpcreq --> streaming{"streaming method?"}
    streaming -->|yes| streamrpc["spawn rpc::handle_streaming_rpc_request(...)"]
    streaming -->|no| plainrpc["spawn rpc::handle_rpc_request(...)"]
```

Streaming RPC methods currently include:

- `up`
- `destroy`
- `resume`
- `redeploy`
- `image.import`
- `image.pull`
- `image.download`

`node.secrets` (a regular method) rewrites a container node's secret files; its params are never
logged, and a parse error is not echoed back.

Regular RPC methods return one `ServerMessage::RpcResponse`. Streaming methods send zero or more `ServerMessage::Status` values, followed by one final `ServerMessage::RpcResponse`.

### SSE progress architecture

Long-running operations use a common progress channel pattern:

```mermaid
flowchart TB
    handler["REST/HTML handler or WebSocket RPC handler"] --> channel["create mpsc::unbounded_channel#lt;Message#gt;()"]
    channel --> sender["create ProgressSender(tx)"]
    sender --> service["spawn service task or call service directly"]
    service --> send["service calls progress.send_status/send_phase(...)"]
    send --> receiver["stream receiver"]
    receiver --> restsse["REST JSON SSE:<br/>api/sse.rs::json_progress_stream"]
    receiver --> htmlsse["HTML SSE:<br/>api/sse.rs::up_progress_stream / destroy_progress_stream"]
    receiver --> websocket["WebSocket:<br/>forwarding task sends Message::Text to Connection"]
```

The service layer does not know if progress is ultimately going to a browser SSE stream, REST SSE stream, or WebSocket. It only sees `ProgressSender`.

## Authentication and authorization architecture

Authentication has three input paths but one shared JWT validation model.

```mermaid
flowchart TB
    browser["Browser HTML<br/>Cookie: sherpa_auth"]
    rest["REST API<br/>Bearer or cookie"]
    rpc["WebSocket RPC<br/>params.token"]
    cookieext["AuthenticatedUserFromCookie / AdminUser<br/>api/extractors.rs"]
    restext["AuthenticatedUser<br/>api/extractors.rs"]
    authreq["authenticate_request<br/>auth/middleware.rs"]
    jwt["jwt::validate_token<br/>auth/jwt.rs"]
    resolve["auth::middleware::resolve_user (DB)"]
    ctx["AuthContext<br/>username + is_admin"]
    service["service<br/>auth/policy.rs check"]
    browser --> cookieext
    rest --> restext
    rpc --> authreq
    cookieext --> jwt
    restext --> jwt
    authreq --> jwt
    jwt --> resolve
    resolve --> ctx
    ctx --> service
```

### Identity comes from the database

The JWT proves who the caller is; it does not decide what they may do. After the token is validated, every extractor and the RPC `authenticate_request` look the subject up in the user table with `auth::middleware::resolve_user`:

- The `is_admin` flag in `AuthContext` comes from the user record, not the token. Demoting an admin takes effect on their next request, not when their seven-day token expires.
- A valid token for a user that no longer exists is rejected: REST returns 401, HTML routes clear the cookie and redirect with `session_expired`, and RPC returns `AuthRequired`.
- If the lookup itself fails, REST returns 500 and HTML routes return 503.

The token still carries `is_admin`, and the UI may use it for display, but no authorization decision reads it.

### Services enforce authorization

Every lab service takes the caller's `&AuthContext` and checks it with the helpers in `auth/policy.rs` before it sends progress, takes a lock, or touches a resource:

| Rule | Helper | Operations |
|---|---|---|
| Lab owner or admin | `require_lab_access` | inspect, destroy, down, resume, download, link impairment |
| Lab owner only, no admin bypass | `require_lab_owner` | redeploy (it rebuilds the node with the owner's SSH keys and account) |
| Admin only | `require_admin` | clean |
| Self or admin | `require_self_or_admin` | listing another user's labs |
| Any authenticated user | none | up (creates the caller's own lab), listing your own labs |

`require_lab_access` and `require_lab_owner` return the lab record, so services do not look it up twice. When an admin acts on another user's lab, the helper logs a `warn` line with `event = "auth.admin_lab_access"`, naming the admin, the lab and the owner. Refusals log `event = "auth.access_denied"`.

Refusals are returned as a typed `services::error::ServiceError` inside `anyhow::Error`: `Denied` (not allowed), `NotFound`, or `Conflict` (allowed, but the current state forbids it, such as deleting a user who owns labs). Transports find it with `ServiceError::find`, which works through added context:

- REST: `ApiError::from` maps `Denied` to 403, `NotFound` to 404 and `Conflict` to 409. No authorization decision is made by matching on error text.
- RPC: `service_error` in `rpc.rs` maps them to `AccessDenied`, `NotFound`, and `ServerError` with the conflict message (JSON-RPC has no conflict code); other failures stay `ServerError` with the error chain as context.
- HTML: `lab_error_page` and `lab_error_fragment` in `handlers.rs` render the 403 and 404 templates.

Request structs (`UpRequest`, `DestroyRequest`, `InspectRequest`, `RedeployRequest`, `UpdateImpairmentRequest`) do not carry a username. Identity only ever comes from the `AuthContext`. Older clients that still send `username` keep working, because unknown fields are ignored.

### When a transport checks first

A few transports must answer before the service runs, and call the same policy helper first:

- REST destroy and REST redeploy return SSE streams, so a refusal inside the service would arrive as a 200 with an error event. They call `require_lab_access` or `require_lab_owner` first, so a refused caller gets 403.
- Web redeploy checks ownership before loading the saved manifest, so a non-owner gets 403 rather than a load error.
- Web destroy checks access before queuing the job.

The service always checks again, so the rule is still enforced if a transport forgets.

### Web jobs

Web UI create and destroy run as jobs: the POST queues a `Job` in `AppState::pending_jobs` and redirects to `/jobs/{id}`. The job records its `requester` (`AuthContext`). The job page and `/jobs/{id}/stream` serve the job only to its requester or an admin; another user gets 403, and the job is left for its owner. The job runs as its requester.

### Other rules

- Browser page access uses cookie-only extractors.
- REST API access uses bearer-or-cookie extractors.
- WebSocket RPC access requires `params.token`.
- User password/info operations allow admins to target anyone and regular users to target themselves. These checks are still in the transports, not in services.
- User deletion goes through `services::user_delete::delete_user` on every transport. It is admin only and refuses self-delete, the last admin account, and any user who owns labs (`Conflict`, listing the labs). The web admin page shows a refusal in its `#user-delete-error` notification using `HX-Retarget`. The database backs up the owned-lab rule: `lab.user` is `REFERENCE ON DELETE REJECT`, because deleting only the records would leave the labs' VMs, containers and networks running unmanaged. With identity read from the database, the last-admin rule can only be hit by a race (two admins deleting each other at once); there is no transaction around the check and the delete.
- `clean_lab_resources` is the unchecked clean used internally, to roll back a failed `up` on behalf of the user who started it. User requests go through `clean_lab`.

## Service layer architecture

The services directory is the orchestration layer. Each service is built around a public async function that takes typed request data, `&AppState`, and sometimes a `ProgressSender`.

```mermaid
flowchart TB
    services["services/*.rs<br/>orchestration, progress reporting, rollback, best-effort cleanup, summaries"]
    persistence["Persistence<br/>db crate<br/>SurrealDB records<br/>schema + CRUD"]
    runtime["Runtime I/O<br/>libvirt crate: domains, disks, networks<br/>container crate: Docker/Bollard<br/>network crate: bridges, veths, taps"]
    transform["Transformation and generation<br/>topology crate: manifest expansion<br/>topology::validate: manifest checks<br/>template crate: configs/XML/scripts"]
    shared["shared crate: public request/response models, constants, utility helpers, spec"]
    services --> persistence
    services --> runtime
    runtime --> transform
    persistence --> shared
```

The services are not symmetrical. Some are short CRUD-style orchestrators, and some are large lifecycle engines.

### Service categories

| Category | Module | Purpose |
|---|---|---|
| Lifecycle services | `up.rs` | create a full lab and all resources |
| | `destroy.rs` | remove a full lab and all resources |
| | `redeploy.rs` | replace one node inside an existing lab |
| | `down.rs` | stop all nodes or one node |
| | `resume.rs` | start all nodes or one node |
| Read/model services | `inspect.rs` | read DB + runtime data and build lab inspection output |
| | `list_labs.rs` | list lab summaries for a user |
| | `download.rs` | package saved lab files for client download |
| Image/admin services | `import.rs` | image import/list/show/set-default/scan/download support |
| | `container_pull.rs` | Docker/OCI image pull with progress |
| | `clean.rs` | admin force-clean path |
| Network mutation services | `impairment.rs` | update delay/jitter/loss/reorder/corrupt settings on P2P links |
| Background service | `scanner.rs` | reconcile runtime state from Docker/libvirt into SurrealDB |
| Shared helpers | `node_ops.rs` | common node setup/building helpers |
| | `progress.rs` | progress message abstraction |

### Service call graph for lab creation

`up.rs` is the largest orchestration module. Its job is to convert a validated manifest into persistent records, generated files, runtime networks, and running nodes.

```mermaid
flowchart TB
    uplab["up_lab(&AuthContext, UpRequest, &AppState, ProgressSender)"] --> pre["Deserialize manifest JSON into topology::Manifest<br/>validate node secrets and Tailscale enrollment<br/>db::validate_lab_id<br/>lab_operations::begin"]
    pre --> setup
    subgraph phase1["1. Setup"]
        setup["Load current user from DB<br/>Guard: lab_id must not already exist<br/>Load node image records from DB"]
    end
    setup --> validation
    subgraph phase2["2. ManifestValidation"]
        validation["topology::validate checks: duplicate device, device count, Talos clusters<br/>validate environment variables<br/>container::get_local_images<br/>validate::validate_and_resolve_node_versions (server)<br/>process_manifest_nodes, then lease node images<br/>validate disk method and guest account per node<br/>process_manifest_links, process_manifest_bridges<br/>validate management/reserved/data interface bounds<br/>validate duplicate links, link and bridge endpoints"]
    end
    validation --> startguard["Guard: no disks or VMs left from an earlier attempt with this lab_id<br/>(after validation, before any resource is created)"]
    startguard --> hostnets
    subgraph phase3["3. DatabaseRecords"]
        subgraph reserve["reserve_lab_networks (retried while concurrent labs win a subnet)"]
            hostnets["read host interface networks (IPv4 and IPv6)"] --> mgmtsubnets["allocate IPv4/IPv6 management subnets<br/>(not in DB, not overlapping host)"] --> loopsubnets["allocate IPv4/IPv6 loopback subnets (not in DB)"] --> createlab["db::create_lab: unique subnet indexes make the insert the reservation"]
        end
        createlab --> createnode["for each node:<br/>db::create_node<br/>node_resources::reserve_node_resources (vCPUs, memory, VNC port, pinned CPUs)<br/>create the node's isolated and reserved libvirt networks and bridges"]
    end
    createnode --> labfiles
    subgraph phase4["4. LabNetworkSetup"]
        labfiles["create /opt/sherpa/labs/{lab_id}<br/>write lab info file<br/>save manifest.toml for redeploy"] --> mgmtnets["libvirt NAT management network<br/>Docker bridge management network"] --> tailscale["tailscale::provision when the lab joins a tailnet"]
    end
    tailscale --> links
    subgraph phase5["5. LinkCreation"]
        links["for each link: db::create_link, all P2p (and db::update_link for impairment)<br/>nothing created yet: libvirt makes VM taps, container veths come later<br/>record veths for disabled interfaces on containers with links"]
    end
    links --> ctrnets
    subgraph phase6["6. ContainerNetworks"]
        ctrnets["Docker macvlan bridge networks for disabled interfaces of containers with no link"]
    end
    ctrnets --> bridges
    subgraph phase7["7. SharedBridges"]
        bridges["network::create_bridge for each [[bridges]] entry<br/>members of containers with links: veth planned for start<br/>other container members: Docker macvlan bridge network<br/>db::create_bridge with its members"]
    end
    bridges --> ztpdirs
    subgraph phase8["8. ZtpGeneration"]
        ztpdirs["create ZTP and TFTP directories<br/>generate the lab CA certificate"] --> nodeztp["per node: assign and save management addresses, node TLS certificate<br/>containers: node_ops::generate_container_ztp<br/>Kubernetes clusters planned and their secrets generated<br/>VMs: node_ops::generate_vm_ztp, save management MAC, build domain template<br/>unikernels: node_ops::generate_unikernel_setup, build domain template"]
    end
    nodeztp --> boot
    subgraph phase9["9. BootContainers"]
        boot["create ZTP config and dnsmasq directories<br/>write dnsmasq config<br/>start the boot container (dnsmasq router)"]
    end
    boot --> disks
    subgraph phase10["10. DiskCloning"]
        disks["node_ops::clone_node_disks for VMs and unikernels"]
    end
    disks --> vms
    subgraph phase11["11. VmCreation"]
        vms["node_ops::create_vm for each VM<br/>node_ops::create_unikernel for each unikernel<br/>release the disk image leases"] --> vmebpf["11b: attach_ready_p2p_links: eBPF redirect on VM-to-VM P2p links"] --> isodown["11c: set isolated bridges down"]
    end
    isodown --> ssh
    subgraph phase12["12. SshConfig"]
        ssh["ssh_config::save_lab_ssh_files<br/>read the SSH private key for the client<br/>build container network attachments"]
    end
    ssh --> secrets
    subgraph phase13["13. NodeReadiness"]
        secrets["write container secret files to RAM"] --> startctr["start containers (node_ops::start_container_node)<br/>P2p containers: veths created into the container netns,<br/>then attach_ready_p2p_links for links now complete<br/>db::update_node_state Running"]
        startctr --> ready["poll VMs with node_ops::check_vm_ready, record host keys<br/>poll unikernels with check_unikernel_ready<br/>no wait for skip_ready_check nodes or Talos cluster workers"]
        ready --> refresh["ssh_config::refresh_lab_ssh_files with recorded host keys<br/>bootstrap Kubernetes clusters"]
    end
    refresh --> response["Return UpResponse { success, lab_info, summary, errors, total_time_secs }"]
    phase1 -.->|"failure: returned, no cleanup"| failed["error returned"]
    phase2 -.-> failed
    startguard -.-> failed
    phase3 -.->|"failure in phases 3-13"| rollback["clean::clean_lab_resources rolls back the lab<br/>(skipped when a destroy stopped up)"]
    phase13 -.-> rollback
```

The central architectural decision in creation is that the manifest is transformed into fully-expanded intermediate structures before resource creation. That avoids spreading manifest parsing rules across Docker/libvirt/network calls. Validation happens before resource-creating phases where possible; resource phases still need cleanup handling because external backends can fail mid-operation.

The management network's `brm-{lab_id}` bridge is created by libvirt from
`crates/libvirt/templates/network/nat.jinja`, with STP off. With STP on, the kernel holds every
new port for its 2 second minimum forward delay twice, listening then learning, whatever the
network XML asks for. A container's first DNS lookups were lost that way, so the FRR image's boot
`apk add` of openssh failed and the node never became reachable.

### Lab creation data flow

```mermaid
flowchart TB
    input["manifest.toml / uploaded manifest JSON"] --> manifest["topology::Manifest"]
    manifest --> expansion["topology expansion<br/>NodeExpanded[]<br/>LinkDetailed[]<br/>BridgeDetailed[]"]
    expansion --> validate["topology::validate and server validate<br/>check caller-supplied data"]
    validate --> persist["db crate persists desired state<br/>lab record<br/>node records<br/>link/interface records"]
    persist --> generate["template crate generates config/domain assets"]
    generate --> realize["runtime crates realize state"]
    realize --> libvirt["libvirt: VM/unikernel domains, NAT networks, disks"]
    realize --> container["container: Docker containers/images/networks"]
    realize --> network["network: bridges, veths, taps, impairment plumbing"]
```

### Service call graph for lab destruction

`destroy.rs` is best-effort: it attempts every cleanup category even if an earlier category
fails. A failure on one resource (a container, VM, disk, network or interface) is recorded and
destroy carries on. So is a setup failure: if `destroy_vms_and_disks` cannot connect to libvirt,
list domains, or find, refresh or list the storage pool, or `destroy_libvirt_networks` cannot
connect or list networks, that step records a `vm`, `disk` or `libvirt_network` error and the
later steps still run. A `vm` or `disk` error keeps the lab record and lab directory, so destroy
can be retried once libvirt is back.

```mermaid
flowchart TB
    destroylab["destroy_lab(&AuthContext, lab_id, &AppState, ProgressSender)"] --> access["require_lab_access: load lab, check owner or admin"]
    access --> stopops["lab_operations::stop: cancel up/redeploy/resume on this lab and wait for them to end"]
    stopops --> listcontainers
    subgraph containers["destroy_containers"]
        listcontainers["container::list_containers"] --> kill["kill matching lab containers if running"] --> removecontainers["remove matching lab containers"]
    end
    removecontainers --> secrets["destroy_lab_secrets: remove the lab's container secret files"]
    secrets --> listdomains
    subgraph vms["destroy_vms_and_disks"]
        listdomains["list libvirt domains"] --> undefine["undefine/destroy domains containing lab_id"] --> pool["lookup Sherpa storage pool"] --> volumes["delete volumes containing lab_id"]
    end
    volumes --> listnetworks
    subgraph dockernets["destroy_docker_networks"]
        listnetworks["container::list_networks"] --> deletenetworks["delete networks containing lab_id"]
    end
    deletenetworks --> listlibvirtnets
    subgraph libvirtnets["destroy_libvirt_networks"]
        listlibvirtnets["list libvirt networks"] --> undefinenets["destroy/undefine networks containing lab_id"]
    end
    undefinenets --> findifaces
    subgraph interfaces["destroy_interfaces"]
        findifaces["network::find_interfaces_fuzzy(lab_id)"] --> deleteifaces["delete known Sherpa-created bridge/veth/tap interfaces"]
    end
    deleteifaces --> deletelinks
    subgraph database["cleanup_database"]
        deletelinks["db::delete_lab_links"] --> deletenodes["db::delete_lab_nodes"] --> deletelab["db::delete_lab"]
    end
    deletelab --> removedir["remove lab directory"]
    removedir --> response["Return DestroyResponse { success, summary, errors }"]
```

Destruction uses name-based matching for many runtime resources because the runtime backends are external systems and the database may not be fully reliable after partial failures. This is why consistent naming with `lab_id` embedded in resource names is part of the architecture.

The lab name comes from the lab's database record, so destroy also works when `up` was stopped
before it wrote the lab info file.

### Operation lifecycle and stopping operations

Long-running operations (`up`, `destroy`, `redeploy`, `resume`, `down`, admin `clean`, image
operations) always run as their own tasks, on every transport, and keep running when their
client disconnects. The WebSocket and SSE handlers spawn them; the plain HTTP handlers wait on
them through `run_detached` in `api/handlers.rs`, because axum drops a handler's future when
its client disconnects and would otherwise stop the operation at an arbitrary await point.

`services/lab_operations.rs` keeps, per lab, a read/write lease and a cancellation token with
a stop reason:

- `up`, `redeploy` and `resume` call `lab_operations::begin` and hold the read lease until
  they return;
- `lab_operations::stop(lab_id, reason)` cancels the token, then waits for the write lease, so
  it returns once every operation on the lab has ended.

Operations check the token only at safe points: between `up` phases, in the readiness and
Talos bootstrap and Kubernetes waits (`LabOperation::sleep` ends early when cancelled), and
before writing lab files. They are never dropped mid-step, because disk and VM steps run in
`spawn_blocking` tasks that would carry on detached.

| Stopped by | Reason | Stopped `up` |
|---|---|---|
| `destroy_lab`, admin `clean_lab` | `Destroy`; the write lease is held while the lab is removed | Skips its auto-cleanup; the destroy removes the lab |
| `lab.cancel` (`services/cancel.rs`): RPC `cancel`, `POST /api/v1/labs/{id}/cancel`, Ctrl-C in the CLI | `Request`; the lease is released at once | Rolls back like a failed `up` |

The first reason wins if both happen. Without this, an orphaned `up` kept polling destroyed
VMs, held its image leases, and recreated the lab directory with cluster credentials after
destroy.

The CLI (`cmd/interrupt.rs`) waits for `up`, `redeploy` and `resume` while listening for
Ctrl-C. On Ctrl-C it asks whether to stop the operation on the server too, defaulting to no; a
yes opens a new connection and calls `cancel`.

`up` holds read leases on its node images so they cannot be deleted while it provisions. VM
and unikernel image leases end once every disk and domain exists (node records and overlay
backing files protect the images from then on), so a long readiness wait does not block image
pull, import or delete. Container image leases last until the containers start.

### Redeploy architecture

Redeploy is a controlled single-node replacement. It exists because updating node configuration in place is often less reliable than destroying and recreating that node.

```mermaid
flowchart TB
    request["redeploy request"] --> owner["require_lab_owner in the service<br/>(owner only, admins included)"]
    owner --> manifest["load saved manifest from lab directory for web UI flow"]
    manifest --> validate["validate target lab and node"]
    validate --> teardown["stop/remove current runtime node resources"]
    teardown --> regenerate["regenerate node-specific config/assets from manifest"]
    regenerate --> recreate["recreate node runtime resource"]
    recreate --> reconnect["reconnect node to existing lab networks/links"]
    reconnect --> updatedb["update DB node state/details"]
    updatedb --> stream["stream progress and final RedeployResponse"]
```

The important boundary is that redeploy should preserve the lab-level network topology and only replace the selected node's runtime resources and generated files.

### Down/resume architecture

`down.rs` and `resume.rs` operate at two scopes:

```mermaid
flowchart LR
    restdown["POST /api/v1/labs/{id}/down"]
    restresume["POST /api/v1/labs/{id}/resume"]
    webstop["POST /labs/{id}/stop"]
    webstart["POST /labs/{id}/start"]
    nodestop["POST /labs/{id}/nodes/{node}/stop"]
    nodestart["POST /labs/{id}/nodes/{node}/start"]
    rpc["RPC down/resume<br/>with node_name optional"]
    alllab["node_name = None<br/>all nodes in lab"]
    onenode["node_name = Some<br/>one node"]
    restdown --> alllab
    restresume --> alllab
    webstop --> alllab
    webstart --> alllab
    nodestop --> onenode
    nodestart --> onenode
    rpc -->|node_name set| onenode
    rpc -->|node_name omitted| alllab
```

Services check owner-or-admin access first, then inspect node kind and call the right runtime backend: Docker for containers and libvirt for VMs/unikernels. The service returns a `LabNodeActionResponse` describing what changed and what failed.

`down` routes each node (`shutdown_route`): VMs are asked to shut down (`domain.shutdown()`:
libvirt uses the guest agent when present, otherwise the ACPI power button); Talos cluster
nodes, whose kernel ignores the power button, get a graceful shutdown over the Talos API
(see `TALOS.md`). `down` then polls until the VM stops and forces a power-off only if it is
still running after `VM_SHUTDOWN_TIMEOUT` (60 seconds) or the request fails. Nodes stop in
parallel in two waves, Talos cluster controllers last, so a lab takes at most about two
timeouts. Unikernels and Talos nodes outside a Sherpa-built cluster are powered off directly,
and the result says why. A node is marked `Stopped` only once it has stopped. Forcing guests off without asking lost recently
written files on every VM without a guest agent, which includes Talos and the Ubuntu images.

Resume then waits for the nodes it cold-booted, as `up` does. It loads the lab's saved
manifest (`manifest.toml` in the lab directory, written by `up`) for `ready_timeout`,
`skip_ready_check` and `ready_port`; labs without a readable saved manifest use the default
timeout and SSH checks. VMs are probed on SSH, unikernels with `check_unikernel_ready`.
Containers and paused nodes are not waited for. A node that is not ready within the timeout
is reported in its result message, not as an error. The WebSocket `resume` RPC is streaming
and sends readiness status messages before its response; the REST and web handlers return
the same `LabNodeActionResponse` once the wait completes.

### Image service architecture

Image management is split by image kind and source:

```mermaid
flowchart LR
    ops["Image operations"]
    import["import.rs"]
    pull["container_pull.rs"]
    delete["delete.rs"]
    ops --> import
    ops --> pull
    ops --> delete
    import --> importvm["import local VM/unikernel disk image metadata/artifact"]
    import --> importctr["import local container tar/archive where supported"]
    import --> list["list image records"]
    import --> show["show image record details"]
    import --> setdefault["set default image version"]
    import --> scan["scan filesystem/Docker for discoverable images"]
    import --> download["download image from URL with progress"]
    pull --> pulloci["pull OCI image through Docker/Bollard with streamed status"]
    delete --> remove["remove imported image records/artifacts"]
```

Admin-only image mutations are enforced at the transport boundary. Image list/show require authentication but not admin privileges. Long-running import/pull/download paths use `ProgressSender` so REST, WebSocket, and UI callers can receive progress without service-specific transport code.

### Scanner service architecture

The scanner is the only long-running background service started by `run_server` today.

```mermaid
flowchart TB
    subgraph runserver["run_server"]
        token["create CancellationToken"] --> enabled{"config.scanner.enabled?"}
        enabled -->|yes| spawn["tokio::spawn(run_scanner(AppState, child_token))"]
    end
    subgraph runscanner["run_scanner"]
        wait{"sleep interval_secs between cycles"}
        stop["stop immediately"]
        subgraph scancycle["scan_cycle"]
            libvirtstates["query_libvirt_states(qemu)<br/>spawn_blocking because libvirt calls are blocking"]
            dockerstates["query_docker_states(docker)<br/>Bollard list_containers"]
            listlabs["db::list_labs"]
            subgraph eachlab["for each lab"]
                listnodes["db::list_nodes_by_lab"]
                images["batch load node images for NodeKind"]
                mapstate["map runtime state to NodeState"]
                nodestate["db::update_node_state when changed"]
                wantskey{"wants_host_key:<br/>running VMs with SSH that started,<br/>or still await a key since they started,<br/>or have none recorded"}
                checkready["node_ops::check_vm_ready on port 22"]
                recordkey["record_node_host_key"]
                refresh["refresh_lab_ssh_files when a key was recorded"]
                derive["derive LabState from node states"]
                labstate["db::update_lab_state when changed"]
            end
        end
    end
    spawn --> wait
    wait -->|cancellation token triggered| stop
    wait -->|interval elapsed| libvirtstates
    libvirtstates --> dockerstates --> listlabs --> listnodes
    listnodes --> images --> mapstate --> nodestate --> wantskey
    wantskey -->|yes| checkready --> recordkey --> refresh --> derive
    wantskey -->|no| derive
    derive --> labstate
    labstate -->|next cycle| wait
    scancycle -.->|"scan failure: log, do not kill server"| wait
```

A VM that changes to Running joins `awaiting_host_key`, an in-memory set kept across cycles,
because libvirt reports it running before its SSH server answers. Its key is read every
cycle until it answers or stops. A running VM with a key is not read again, so the scanner
makes no SSH connections to settled nodes; a key that changes without a state change, as on a
reboot from inside the guest, is read when a client finds it does not match.

The scanner is reconciliation, not orchestration. It should not create or destroy resources. It observes runtime state and updates the database so UI/API consumers see reality when resources are externally stopped, started, removed, or crash.

## HTTP routing architecture

`api/router.rs` builds a single Axum router. It attaches CORS, HTTP tracing, embedded static asset handling, and the route table. The WebSocket route is added in `daemon/server.rs` after `build_router()` returns.

`build_router()` contents:

| Group | Routes |
|---|---|
| Public browser routes | `GET/POST /login`, `GET/POST /signup`, `POST /logout` |
| Protected browser routes | `GET /`; `GET /labs`, `/labs/grid`, `/labs/create`; `GET /jobs/{job_id}`, `/jobs/{job_id}/stream`; `GET /labs/{lab_id}`; `GET /labs/{lab_id}/nodes[/node]`; `POST` lab/node start/stop/redeploy/destroy actions; `GET/POST` profile and SSH-key routes |
| Admin browser routes | `/admin/users...`, `/admin/labs`, `/admin/tools...`, `/admin/images...` |
| Public API routes | `GET /health`, `GET /cert`, `GET /api/v1/spec`, `GET /api/v1/openapi.json`, `GET /api/docs`, `POST /api/v1/auth/login` |
| REST API routes | labs: list/create/inspect/delete/down/resume/redeploy; links: impairment update; images: list/show/import/upload/delete/default/pull/download; admin tools: clean/scan; users: create/list/info/delete/password |
| Layers | `CorsLayer` mirror_request + credentials; `TraceLayer::new_for_http()` |
| Catch-all | static asset route `/{*path}` |

Auth is enforced by handler signatures and handler-local checks, not by route grouping. When reviewing a route's protection level, check the handler parameters and body rather than only the route table.

## Public API specification architecture

The generated API metadata is built from `crates/shared/src/api_spec.rs`.

```mermaid
flowchart TB
    buildspec["shared::api_spec::build_spec()"] --> buildops["build_operations()"]
    buildspec --> buildschemas["build_schemas()"]
    buildops --> opdefs["OperationDef[]<br/>operation name<br/>auth requirement<br/>streaming flag<br/>request/response schema names<br/>REST/RPC/CLI bindings"]
    buildschemas --> schemas["schemars JSON schemas from shared data types"]
    opdefs --> apispec["ApiSpec"]
    schemas --> apispec
    apispec --> specroute["GET /api/v1/spec"]
    apispec --> buildopenapi["build_openapi()"]
    buildopenapi --> openapiroute["GET /api/v1/openapi.json"]
```

`/api/docs` serves embedded Swagger UI backed by the OpenAPI endpoint.

Current implementation note: the registry marks five canonical operations as streaming (`lab.create`, `lab.destroy`, `node.redeploy`, `image.pull`, `image.download`), while the current REST/RPC handlers also stream `image.import`. That mismatch should be resolved in `api_spec.rs` if streaming import is the intended public contract.

## Lab lifecycle architecture

### Create lifecycle phases

The creation path validates and expands the whole manifest before it creates anything, then creates the lab's records, files and runtime resources phase by phase.

```mermaid
flowchart TB
    input["Manifest input"] --> setup["1. Setup<br/>user, lab_id unused, image records"]
    setup --> validation["2. ManifestValidation<br/>validate and expand nodes, links, bridges"]
    validation --> guard["startup guard<br/>no leftover disks or VMs"]
    subgraph created["Phases that create resources: a failure rolls back with clean_lab_resources"]
        direction TB
        db["3. DatabaseRecords<br/>lab, nodes, per-node libvirt networks"]
        labnet["4. LabNetworkSetup<br/>lab dir and files, management networks, Tailscale"]
        links["5. LinkCreation<br/>link records"]
        ctrnets["6. ContainerNetworks"]
        bridges["7. SharedBridges"]
        ztp["8. ZtpGeneration<br/>CA and node certs, ZTP, cluster secrets, domain templates"]
        boot["9. BootContainers"]
        disks["10. DiskCloning"]
        vms["11. VmCreation<br/>VMs, unikernels, VM-to-VM P2p links"]
        ssh["12. SshConfig"]
        ready["13. NodeReadiness<br/>start containers and their P2p links, wait for nodes"]
        db --> labnet --> links --> ctrnets --> bridges --> ztp --> boot --> disks --> vms --> ssh --> ready
    end
    guard --> db
    ready --> response["UpResponse"]
```

`up_lab` reports progress per `shared::data::UpPhase`, in this order. Setup, ManifestValidation
and the startup guard create nothing, so a failure there returns without cleanup. Once resource
creation starts, the service must assume partial success is possible: a failure in a later
phase cleans up what was created. The [call graph](#service-call-graph-for-lab-creation) lists
what each phase does.

### Node readiness

`up`, `redeploy` and `resume` poll each VM's readiness port (`node_ops::vm_ready_port`: the
manifest's `ready_port`, else the model's) with `node_ops::check_vm_ready`. Unikernels use
`check_unikernel_ready`, and containers are ready once running.

```mermaid
flowchart TB
    check["check_vm_ready(ip, port)"] --> port{"port"}
    port -->|22| ssh["check_node_ssh_ready"]
    ssh --> tcp["TCP connect (100 ms)"]
    tcp -->|"refused/timeout"| notready["not ready"]
    tcp -->|connected| hostkey["Jato Session::host_key<br/>(5 s, no login)"]
    hostkey -->|"SSH handshake through key exchange, then disconnect"| ready["ready, with host key"]
    hostkey -->|"non-SSH reply or timeout"| notready
    port -->|other port| portready["check_node_port_ready:<br/>TCP connect (100 ms)"]
```

An open port is not enough on SSH: network devices and some guests accept TCP connections
before their SSH server can answer. The probe uses the Jato library (`bwks/jato-lib`, a
git dependency pinned to a release tag; it is not on crates.io yet). It sends no credentials, so a
node logs no failed login. It allows legacy algorithms for older network devices. The TCP
connect comes first so that nodes that are still booting fail in 100 ms and do not hold up
the poll loop.

The probe returns the host key the node offered (`VmReadiness::Ready { host_key }`), accepted
without checking: the server is on the lab's management network and sees the node first, so
it is the party users already trust to vouch for it. Labs are short-lived, and nodes'
keys were accepted unchecked before. The key is kept on the node's database record
(`node.ssh_host_key`, `db::update_node_host_key`):

- `up` records each VM's key as it becomes ready.
- `redeploy` clears the node's key before teardown and records the new one when it is ready.
- `resume` records the keys of the nodes it cold-booted, as some network operating systems
  (Cisco IOSv, IOSvL2, IOS-XE) make a new RSA key on every boot.
- The scanner records the key of a VM that started outside these, such as from the web UI,
  and of a running VM with no key (see the scanner section).
- The `node.host_keys` RPC (`services::host_keys`) reads one node's key on request, for a
  client whose recorded key did not match.

`ssh_config::record_node_host_key` writes a key only when it differs, and logs a warning when
it replaces another. `ssh_config::refresh_lab_ssh_files` then renders the lab's SSH files
from the database (nodes' addresses, whether their image has SSH, and their keys) and saves
them with `lab-info.toml`: `sherpa_ssh_config`, in which a node with a recorded key gets
`StrictHostKeyChecking yes` and `UserKnownHostsFile sherpa_known_hosts` (others keep `no` and
`/dev/null`), and `sherpa_known_hosts` (template `ssh/known_hosts.jinja`), one line per key under
the name OpenSSH looks up: the address on port 22, else `[address]:port`. `up`'s response, the
download service and `node.host_keys` carry the known_hosts file, and the client rewrites the
relative `UserKnownHostsFile` path to an absolute one, as it does for `IdentityFile`
(`client::lab_files`). When a node's key is unknown or changed, the client fetches the lab's
keys with `node.host_keys`, warns about a changed key, and connects again (`SSH_CLIENT.md`).

### Runtime resource model for a lab

A running lab is spread across several systems. The DB is the desired/known state, not the only state.

```mermaid
flowchart LR
    lab["Lab {lab_id}"]
    surreal["SurrealDB<br/>user record<br/>lab record<br/>node records<br/>link records<br/>image records"]
    ram["RAM: /run/sherpa/secrets/{lab_id}/{node}<br/>(container secret files)"]
    fs["Filesystem: /opt/sherpa/labs/{lab_id}<br/>lab info file<br/>saved manifest<br/>generated node configs<br/>ZTP/TFTP assets<br/>cert material where needed<br/>SSH/helper config files"]
    libvirt["libvirt/QEMU<br/>VM domains<br/>unikernel domains<br/>NAT/isolated/reserved networks<br/>cloned/resized disks in Sherpa storage pool"]
    docker["Docker<br/>containers<br/>bridge/macvlan networks<br/>pulled/imported container images"]
    host["Linux host networking<br/>bridges<br/>veth pairs<br/>tap devices<br/>impairment/eBPF/netns wiring where applicable"]
    lab --> surreal
    lab --> ram
    lab --> fs
    lab --> libvirt
    lab --> docker
    lab --> host
```

Destroy and clean paths must account for all of these locations. That is why cleanup is not a single database delete.

### Node resources and VNC ports

What a node is given is stored on its database record (`DbNode`), not read back from the
hypervisor, by `services::node_resources`:

- `planned_resources`: VMs and unikernels get the manifest's `cpu_count` and `memory`, else the
  image's. Containers get none, as Sherpa does not limit them.
- `reserve_node_resources` (`up`, right after each node record is created, and `redeploy`): a
  VM also gets a VNC port, the lowest in `konst::VNC_PORT_FIRST..=VNC_PORT_LAST` that no node
  holds and nothing on the server listens on. `unique_node_vnc_port` settles a race with a
  concurrent lab, which is retried, as lab subnets are. A redeployed VM keeps its port.
- `node_ops::build_domain_template` takes the stored values, so the domain and the database
  agree. The domain has `port='N' autoport='no'`, so libvirt never moves the console, also
  across down/resume.
- `inspect` and the web UI node page read them from the record, with no domain lookup.

### VM performance tuning

A node's `[nodes.performance_tuning]` (`shared::data::PerformanceTuning`) reaches the domain as
`template::DomainTuning`, built in `node_ops::build_domain_template`. The domain then renders:
- `<cputune>` with `vcpupin`, `emulatorpin` and `iothreadpin`;
- `<numatune>`;
- `<iothreads>`;
- the CPU mode and `<topology>`;
- the disk driver's `cache`, `io` and `iothread`;
- `<driver queues>` on virtio interfaces.

The checks run in three places:
- **`topology::validate::check_performance_tuning`** (client `validate` and server `up` and
  `redeploy`): the node's own settings against its image. VMs only; topology and pinning
  match `cpu_count`; I/O threads need virtio disks; queues need virtio interfaces.
- **`topology::validate::check_cpu_pin_overlap`**: a host CPU pinned to a vCPU belongs to one
  node, and no node runs QEMU or I/O threads on it. QEMU and I/O threads may share CPUs.
- **`node_resources::check_cpu_pins_on_server`** (`up` and `redeploy`, before anything is
  created or destroyed): every CPU and the NUMA node exist on the server, from
  `libvirt::host_cpus`. The pins also must not overlap those stored on any node of any lab
  (`db::list_pinned_nodes`). A redeployed node's own stored pins are skipped.

`planned_resources` stores the pins on the node (`cpu_pinning`, and `qemu_thread_cpus` for the
QEMU and I/O threads), so later labs see them. Two labs started at the same moment could both
pass the check before either stores its pins; unlike VNC ports, pins have no unique index.

Huge pages come from the host's transparent hugepages, which QEMU requests for guest RAM.
There is no `memoryBacking` setting. `scripts/sherpa_install.sh` sets THP to `madvise` unless it
is `always`, now and on boot through `/etc/tmpfiles.d/sherpa-thp.conf`; the uninstall script
removes that rule.

### Container data interfaces

Every `[[links]]` link is a P2p (eBPF) link. A container with a link takes all its data
interfaces as veth pairs created from the host straight into its namespace after it starts,
including its private bridge members and disabled interfaces (see `P2P-ARCHITECTURE.md`).
Nothing runs in the image, so images without `ip` (such as `mongo`) or a shell work.

A container with no link gets a Docker network attachment (`ContainerNetworkAttachment`) for
each data interface, connected in index order after the container starts
(`container::run_container`). `services::container_networks` builds them for `up` and
`redeploy`. Every data interface gets one: Docker names unnamed interfaces in attach order, so
one left out would rename every interface after it. A link end among them is an error, since a
container with a link takes veths.

| Interface | Docker network | Mode | Parent |
|---|---|---|---|
| `[[bridges]]` member | `{node}-brs{interface_index}-{lab_id}` | macvlan bridge | the bridge's `brs{bridge_index}-{lab_id}` |
| disabled | `{node}-iso{interface_index}-{lab_id}` | macvlan bridge | the node's isolated `bri` bridge |

Bridge links, from labs created before every link was a P2p link, are no longer supported: see
[P2P-ARCHITECTURE.md](P2P-ARCHITECTURE.md).

Bridge member networks are created in phase 7, after their `brs` bridge, and the members of
containers with links get a veth pair there. The bridge record stores its `members`, each a
node and interface, so `redeploy` reconnects exactly those interfaces, for VMs as well as
containers.

- Docker names the interface in the container through the endpoint driver option
  `konst::DOCKER_ENDPOINT_IFNAME_OPT` when `linux_interface_name` is set.
- A disabled interface is a macvlan on the node's isolated bridge. The server sets that bridge
  down from the host (`network::set_link_down`, phase 11c and the container redeploy), as for
  VMs, so the interface has no carrier.
- An active interface that Docker renames (an SR Linux bridge member) is set promiscuous and up
  with `ip` in the container (`data_interface_command`).

### Lab management addressing

Each lab gets a management `/24` (IPv4) and `/64` (IPv6). Host offsets are split into fixed
blocks, defined in `crates/shared/src/konst.rs` and applied to both address families:

| Offsets | Block | Users |
|---|---|---|
| `1`–`10` | Sherpa infrastructure (`MANAGEMENT_INFRA_HOST_OFFSETS`) | `.1` gateway and DNS, `.2` router (dnsmasq, boot server), `.3` Tailscale gateway |
| `11`–`20` | Lab control planes (`MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS`) | `.11`–`.12` Talos cluster VIPs (`TALOS_VIP_HOST_OFFSETS`) |
| `21`–`254` | Lab nodes (`MANAGEMENT_NODE_HOST_OFFSETS`) | node `n` (1-based manifest index) at `.20 + n`; the dnsmasq DHCP range |

`util::node_management_host_offset` maps a node index to its offset and refuses indexes outside
the node block, so a lab holds at most 234 nodes; `topology::validate::check_device_count` rejects larger
manifests before anything is created. Nodes keep `dhcp-host` reservations inside the DHCP range,
so dnsmasq never leases a node's address to another client.

Up assigns addresses from the node index and saves them on the node records. Redeploy reuses the
saved addresses and only computes one for a node without them, so labs created under an earlier
layout keep the addresses their dnsmasq config and SSH config already hold. Resume reads the
saved addresses.

### Destroy lifecycle phases

```mermaid
flowchart TB
    destroy["destroy_lab(&AuthContext, lab_id)"] --> access["require_lab_access: load lab, owner or admin"]
    access --> stopops["lab_operations::stop: cancel operations on this lab"]
    stopops --> labname["lab name from the lab record"]
    labname --> containers
    subgraph cleanup["best-effort cleanup sequence"]
        containers["Step 1: Docker containers, then their secret files"]
        domains["Step 2: libvirt domains and disks"]
        dockernets["Step 3: Docker networks"]
        libvirtnets["Step 4: libvirt networks"]
        interfaces["Step 5: Linux host interfaces"]
        records["Step 6: SurrealDB lab/link/node records"]
        labdir["Step 7: /opt/sherpa/labs/{lab_id} directory"]
        containers --> domains --> dockernets --> libvirtnets --> interfaces --> records --> labdir
    end
    labdir --> response["DestroyResponse<br/>success = errors.is_empty()<br/>summary of each cleanup category<br/>detailed DestroyError[] for failed categories"]
```

The best-effort rule is important: failing to delete a disk should not prevent the server from attempting to delete Docker networks or database records. Operators need a complete summary of what was cleaned and what still requires manual repair.

## TLS/listener architecture

TLS is enabled by default. The main listener serves the UI, REST API, and WebSocket endpoint. When TLS is enabled, a separate HTTP listener is started only for certificate retrieval.

```mermaid
flowchart TB
    subgraph tlson["TLS enabled"]
        main["main listener"]
        main --> main4["IPv4: wss/https on config.server_ipv4:config.ws_port"]
        main --> main6["IPv6: optional wss/https on [config.server_ipv6]:config.ws_port"]
        cert["cert listener"]
        cert --> cert4["IPv4: http on config.server_ipv4:config.http_port"]
        cert --> cert6["IPv6: optional http on [config.server_ipv6]:config.http_port"]
    end
    subgraph tlsoff["TLS disabled"]
        mainonly["main listener only"]
        mainonly --> plain4["IPv4: ws/http"]
        mainonly --> plain6["IPv6: optional ws/http"]
    end
```

Certificate SAN generation is part of server startup. If no SANs are configured, startup adds listen IPs, non-loopback interface IPs when listening on unspecified addresses, localhost, loopbacks, hostname, and FQDN when available.

The `/cert` endpoint is public by design because clients need to fetch the certificate before they can establish trust. If TLS is disabled, `/cert` returns a service-unavailable JSON response.

## Observability architecture

Logging and OTel are initialized before `AppState` so startup failures and backend connection failures are visible.

```mermaid
flowchart TB
    runserver["run_server"] --> envfilter["EnvFilter from RUST_LOG or info"]
    envfilter --> otel{"OTel enabled?"}
    otel -->|yes| tracer
    subgraph otelinit["OTel"]
        tracer["SdkTracerProvider + OTLP span exporter"] --> meter["SdkMeterProvider + periodic OTLP metric exporter"] --> layer["tracing_opentelemetry layer"] --> metrics["Metrics::new(meter)"]
    end
    otel -->|no| mode
    metrics --> mode{"foreground or daemon?"}
    mode -->|foreground| terminal["tracing fmt layer to terminal"]
    mode -->|daemon| logfile["tracing fmt layer to sherpad log file"]
```

Important spans/metrics are created at these levels:

- HTTP requests through `tower_http::TraceLayer`.
- WebSocket connections through a `ws_connection` span.
- RPC calls through an `rpc_call` span.
- Public service operations through `#[instrument]` spans.
- Database, Docker, libvirt, and network helper functions where instrumented in their crates.
- Metrics for WebSocket connection count, RPC duration, service operation duration, and errors.

See [`OTEL.md`](OTEL.md) for collector configuration and exported metric names.

## Known architectural seams

Open implementation seams (handler-driven auth, inconsistent service authorization, spec/streaming parity, in-memory jobs, name-based resource cleanup, blocking libvirt calls) each have a proposal in [`../prds/`](../prds/).

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
