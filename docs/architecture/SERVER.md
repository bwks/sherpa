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
        database["Embedded Turso database<br/>(in-process, /opt/sherpa/db/sherpa.sqlite):<br/>users, labs, nodes, links, image records"]
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
        boundaryfiles["api/handlers/<br/>api/extractors.rs<br/>api/websocket/rpc/<br/>auth/{jwt,cookies,middleware,context,session}.rs"]
        boundaryduties["Responsibilities:<br/>parse path/query/body/form/multipart data<br/>authenticate cookies, bearer tokens, or RPC params.token<br/>pass the caller's AuthContext to services<br/>construct service request structs<br/>convert service results into HTML, JSON, SSE, or RPC responses"]
    end
    subgraph servicelayer["Service layer"]
        servicefiles["services/up/, services/destroy.rs<br/>services/down.rs, services/resume.rs<br/>services/redeploy/, services/inspect.rs<br/>services/import.rs, services/container_pull.rs<br/>services/impairment.rs, services/scanner.rs"]
        serviceduties["Responsibilities:<br/>orchestrate db/libvirt/docker/network/template/topology/validate<br/>emit progress updates for long-running operations<br/>maintain operation-level rollback/best-effort cleanup semantics<br/>return shared request/response models"]
    end
    subgraph infra["Infrastructure and domain crates"]
        crates["db: embedded Turso schema and CRUD<br/>libvirt: QEMU/libvirt domain, network, storage operations<br/>container: Docker/Bollard operations<br/>network: Linux bridges, veths, taps, impairment helpers<br/>topology: manifest transformation<br/>topology::validate: validation of caller-provided manifest data<br/>template: generated configs/XML/scripts<br/>shared: public models, constants, utilities, API spec"]
    end
    transport --> boundary
    boundary --> servicelayer
    servicelayer --> infra
```

The important boundary is between handlers and services. Handlers know about transports and authentication. Services know about orchestration. Infrastructure crates know how to talk to their backend, but should not know about HTTP, cookies, WebSocket message IDs, or HTMX fragments.

Constants only the server uses are not in `shared::konst`. One used by a single module is
private in that module. One used by a single service and its submodules, or by one module tree
such as `daemon`, lives in that service or tree: sherpad's pid and log file paths are
`pub(super)` in `daemon::pidfile`. One used by several services or trees is `pub(crate)` in
`crates/server/src/konst.rs`.
An edit to them rebuilds `sherpad` alone, not every crate that depends on `shared` (#663).
`shared::konst` keeps what other crates, `shared` itself, the `sherpad` binary or the
integration tests name.

## Server startup architecture

The binary entry point is `crates/server/src/main.rs`. Its job is intentionally small: initialize rustls, detect the internal background-child mode, parse the `sherpad` CLI, and dispatch to daemon management.

`sherpad` is also a library, for `main.rs` and the integration tests (`crates/server/tests/integration/`, one test binary). It exports only what they use: `cli`, `init`, `doctor`, `check_database`, `daemon::{manager, metrics, state}`, `api::build_router`, `api::websocket::{connection, handler}` and `auth::cookies`. Everything else is `pub(crate)`, so the compiler reports unused code and the pedantic API lints apply only to that surface.

```mermaid
flowchart TB
    start["sherpad process start"] --> mainrs["main.rs"]
    mainrs --> rustls["install rustls aws-lc-rs crypto provider"]
    rustls --> bgcheck{"argv[1] == --background-child?"}
    bgcheck -->|yes| bgchild["daemon::manager::run_background_child()"]
    bgcheck -->|no| parse["parse Cli from cli.rs"]
    parse -->|init| init["init::init(...)"]
    parse -->|doctor| doctor["doctor::doctor(...)"]
    parse -->|check-database| checkdb["check_database::check_database(path)"]
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
    filefmt --> removed
    removed["warn when sherpa.toml still has<br/>the removed [ztp_server] table"]
    removed --> signals
    signals["install SIGINT and SIGTERM handlers<br/>(startup fails if registration fails)"]
    signals --> envoverrides["apply env overrides<br/>SHERPA_SERVER_IPV4<br/>SHERPA_SERVER_IPV6<br/>SHERPA_SERVER_WS_PORT<br/>SHERPA_SERVER_HTTP_PORT"]
    envoverrides --> jwt
    subgraph appstate["AppState::new(config, metrics)"]
        jwt["load JWT secret, or generate it if missing<br/>(a new one written 0600 in a 0700 directory;<br/>a file of the wrong length stops startup)"] --> database["open the database file"] --> schema["apply DB schema"] --> dbkey["load the database key, or generate it if missing<br/>and the database has no key check<br/>(0600 in /opt/sherpa/.secret); verify_secret_key"] --> seed["optionally seed admin user"] --> storage["restrict lab and database storage<br/>(daemon::storage::secure_storage)"] --> qemu["initialize Qemu wrapper"] --> docker["connect Docker client"] --> registry["create WebSocket registry and pending-job map"]
    end
    registry --> recover["services::lab_lifecycle::recover:<br/>labs left Creating become Interrupted,<br/>left Destroying become DestroyFailed,<br/>nodes left Building become Failed"]
    recover --> cancel["create CancellationToken for background services"]
    cancel --> scanner["if scanner enabled:<br/>spawn services::scanner::run_scanner(...)"]
    scanner --> sweeper["spawn auth::session::run_sweeper<br/>(deletes expired sessions at startup, then hourly)"]
    sweeper --> router
    router["build Axum router and add /ws route"]
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

In `daemon/server.rs`, `run_server` calls `init_telemetry` (filter, OTel, logging), warns about removed settings ([below](#removed-settings)), `apply_env_overrides`, builds `AppState`, recovers the labs and nodes the operations running when the server last stopped left mid-way (`lab_lifecycle::recover`, before anything takes requests; see [`STATE_MACHINES.md`](STATE_MACHINES.md#2-lab-phase-and-status-labphase-and-labstate)), starts the scanner and the session sweeper, then `serve_tls` or `serve_plain` with the bound `Listening` addresses; `serve_cert_endpoint` serves `/cert` on each address family.

#### Removed settings

`shared::data::Config` does not deny unknown fields, so a `sherpa.toml` written by an older
release loads with settings sherpad no longer has. One of them is the server-wide `[ztp_server]`
table (`enable`, `username`, `password`), which nothing ever read; releases up to 0.6.5 wrote it
with the built-in node password. Once logging is set up,
`daemon::removed_settings::warn_on_removed_settings` reads the file again and logs one warning,
naming the file, when it has a top-level `[ztp_server]` table: it is ignored and can be deleted.
Neither sherpad nor the update script edits the file. A failure to read or parse it here is
logged, not fatal, since the config has already loaded. A lab's own `[ztp_server]` in its
manifest is a different setting and is unaffected.

`auth::jwt::load_or_generate_secret` generates the JWT secret only when the file is missing. A file that exists with a length other than 32 bytes stops startup with an error naming the file: replacing it would silently end every session. Deleting the file generates a new secret, which logs every user out.

`daemon::db_key::load_or_create_db_key` loads the database key from `SHERPA_DB_KEY_PATH`
(`/opt/sherpa/.secret/db.key`, beside the JWT secret in the `0700` `sherpa` directory), the key
that seals the database's secret values (see
[`DATABASE.md`](DATABASE.md#secret-values)). It generates one, 32 random bytes written `0600`,
only when the file is missing and the database has no key check yet; then `db::verify_secret_key`
records the check. A missing file for a database that has a check, a file of the wrong length,
and a key that does not open the check each stop startup with an error naming the problem; the
file is never replaced. The key must be backed up alongside every `server.backup`, but
separately from it: a backup never contains it.

#### Files older releases left in lab folders

Releases 0.6.4 and 0.6.5 converted, at every start, what older releases kept in each lab's
server folder, `/opt/sherpa/labs/<lab-id>`. Between them they imported the saved `manifest.toml` (or legacy
`manifest.json`) as the lab's definition and marked its nodes, copied the SSH accounts of
`lab-info.toml` into the node records, stored each Talos cluster's `clusters/<cluster>/cluster.toml`
as its plan, and deleted those files with `sherpa_ssh_config`, `sherpa_known_hosts`, the empty
`ztp/dnsmasq/dnsmasq.leases`, unikernel certificates and each VM's old boot folder, `<node>/`.
0.7.0 removed those steps, so a server upgrades through 0.6.5, and starts it once, before
0.7.0.

Nothing in sherpad lists a lab folder or reads those file names, so one left behind, by a server
that skipped 0.6.5 or copied in by hand, is ignored: sherpad starts and serves as usual, and the
file stays until `destroy` removes the lab's folder. A lab that skipped the import has no
definition: the readers use their defaults (see [`DATABASE.md`](DATABASE.md#lab-definitions)),
and the web UI refuses to redeploy its nodes until `sherpa redeploy` stores one. A Talos cluster
that skipped it has no plan, so `down` powers its nodes off, a worker `redeploy` is refused and
`download` leaves it out. An old VM boot folder is removed when the node is redeployed.

Shutdown uses SIGINT/SIGTERM. The handlers are registered once, before the server binds any listener, so a registration failure stops startup with an error instead of leaving a server that cannot shut down gracefully. On shutdown, the server cancels the background-service token. With TLS, it also asks the `axum_server::Handle` to perform graceful shutdown with a short timeout. The OpenTelemetry guard flushes traces/metrics when dropped.

### Lab and database storage permissions

Lab data and the database are kept from other server users by directory modes, not file modes,
so files written by older releases are covered too. `daemon::storage::secure_storage` runs on
every start and calls `restrict_dir` on:

| Directory | Mode and group | Constants |
|---|---|---|
| `/opt/sherpa/labs` | `2750`, the `sherpa` group | `SHERPA_LABS_DIR_MODE` |
| `/opt/sherpa/libvirt/images` (storage pool) | `2750`, `kvm` | `SHERPA_STORAGE_POOL_DIR_MODE`, `SHERPA_STORAGE_POOL_GROUP` |
| `/opt/sherpa/db` | `750` | `SHERPA_DB_DIR_MODE` |
| `/opt/sherpa/db/backups` | `700`; the backup service restricts it again before each backup | `SHERPA_DB_BACKUP_DIR_MODE` |

- QEMU runs as `libvirt-qemu` in `kvm`, so it can reach the pool. Nothing reads `/opt/sherpa/labs`
  through the directory except `sherpad`: the dnsmasq router and container nodes get lab files
  through Docker bind mounts.
- File modes inside lab directories are left as they are. dnsmasq runs as root and serves only
  world-readable TFTP files, and container nodes may read their bind-mounted configs as a
  non-root user, so a restrictive umask would break ZTP.
- New pool volumes are created `0600` (`konst::STORAGE_VOLUME_MODE`, in
  `crates/libvirt/templates/copy.xml` and `overlay.xml`); libvirt hands them to the QEMU user
  while a VM runs (`libvirt::vm::copy_volume_xml`). Flatcar's `.ign` file is no exception: the
  domain names it in a `<sysinfo type='fwcfg'>` entry, `opt/org.flatcar-linux/config`, so libvirt
  hands it over, labels it and adds it to the VM's AppArmor profile like a disk. A raw `-fw_cfg`
  QEMU argument gets none of that, and AppArmor denies QEMU the file.
- A failure is logged, not fatal. `sherpad init` does not create these directories, and only the
  install and update scripts, as root, set their owner and modes besides `sherpad`.

### Image folders of renamed models

A VM or unikernel image is `<images_dir>/<model>/<version>/<file>`, so the model renames of 0.7.0
rename folders. `daemon::image_folders::move_renamed_model_folders` runs on every start, after
`secure_storage` and before the server takes requests, for each old name in
`shared::data::REMOVED_MODELS`:

- **A folder at the old name** is renamed to the new one. When the new folder exists, because two
  old models became one (`generic_vm` and `generic_unikernel` into `generic`, or the empty
  `arista_ceos` folder into `arista_eos`), its entries are moved in one by one, version folders
  merged the same way. An entry the new folder already has is left in the old folder and logged,
  never replaced; the old folder is removed only once it is empty. Only `std::fs` renames, and
  `remove_dir` of an empty folder: no image file is deleted.
- **A link at the old name.** A VM overlay records its base by path, and libvirt and QEMU open the
  base by that path at the next start, so an overlay made before the move still names
  `arista_veos/4.33/virtioa.qcow2`. While a disk in the storage pool has a base under the old
  path, the old path is a relative symlink to the new folder (`arista_veos -> arista_eos`).
  `libvirt::pool_backed_volumes` asks libvirt for each pool volume's `<backingStore>` path, as
  the disk records it, without resolving it: libvirtd reads every volume, while sherpad cannot
  read the disks libvirt has handed to QEMU (`0600`, owned by `libvirt-qemu` or `root`). A later start removes the link once no disk names it, that is
  once the last node built on it is redeployed or destroyed. When the pool cannot be read, every
  link is kept and every moved folder gets one.
- Unikernel kernels and disks, the Aboot ISO and other CD-ROMs are copied into the storage pool
  when a node is built, so only overlays keep an old path.
- A failure to read, move or link a folder stops the start, naming the folder.

The link is state on disk for old labs, not a name anything takes: an old name is refused in a
manifest, the CLI and the API, and the image scan skips the folder because its name is not a
model.

### Config, env and log file permissions

`sherpa.toml` and the env files can hold secrets: the removed `[ztp_server]` password a file
from an older release may still have, and anything an operator adds to `sherpa.env`, such as
`OTEL_EXPORTER_OTLP_HEADERS`. So only `sherpa` may read
them, which a member of the `sherpa` group cannot. `sherpad` reads `sherpa.toml` as `sherpa`;
systemd reads `sherpa.env` as root (`EnvironmentFile`).

| Path | Mode, all `sherpa:sherpa` | Set by |
|---|---|---|
| `/opt/sherpa/config` | `750` | install, update |
| `/opt/sherpa/config/sherpa.toml` | `600` (`SHERPA_CONFIG_FILE_MODE`) | `sherpad init`, install re-run, update |
| `/opt/sherpa/env` | `750` | install, update |
| `/opt/sherpa/env/sherpa.env`, `sherpa.env.example` | `600` | install, update |
| `/opt/sherpa/logs` | `750` (`SHERPA_LOG_DIR_MODE`) | install, update, `sherpad` when it creates it |
| `/opt/sherpa/logs/sherpad.log` and rotated copies | `640` (`SHERPAD_LOG_FILE_MODE`) | `sherpad` on each open, update |

- `sherpad init` runs as `sherpa` (`sudo -u sherpa -H sherpad init`), so the files it writes are
  `sherpa`'s. It refuses any other account, root included, before it writes anything:
  `validate::validate_init_account` compares the name of the effective uid's account with
  `konst::SHERPA_USERNAME`. It writes `sherpa.toml` through
  `shared::util::write_file_with_permissions`: the mode is set on a temporary file, whatever the
  umask, before it is renamed into place. Releases
  before this wrote it with the caller's umask, often `0664`, owned by whoever ran `init`.
- The example env file gets `sherpa.env`'s mode because people copy it into place.
- The log is not meant to hold secrets. `daemon::pidfile::open_log_file` creates it `0640` and
  narrows a wider file on every open; under systemd `sherpad` logs to the journal and writes no
  file.
- `sherpa_update.sh`'s `secure_config_files` runs on every update, including one with nothing to
  install. It changes owner and mode only, never contents, and skips a path that is a symlink, so
  root never follows one planted in a folder `sherpa` can write. A missing path is skipped.
- The `sherpa` group can read the log and enter `config` and `env`, but not read the two files.
  Admins edit them with `sudo -u sherpa` or `sudoedit`.

### Container secret files

Container secret files (see [`PROVISIONERS.md`](PROVISIONERS.md)) are kept in RAM, under
`/run/sherpa`, the daemon's systemd `RuntimeDirectory` (`0700 sherpa`, a tmpfs).
`RuntimeDirectoryPreserve=yes` keeps it when `sherpad` stops or restarts, so running containers
keep their secrets; a reboot clears it. The install script writes these settings into the unit,
and the update script and `dev/rebuild` add them to older units as the drop-in
`sherpad.service.d/runtime-directory.conf`.

`services::node_secrets` writes `server::konst::SHERPA_SECRETS_DIR/<lab>/<node>/`: the lab directory is
`0700`, and the node directory, the bind mount's root, is `0755`. Nothing under `/run/sherpa` is
readable by other server users or the `sherpa` group.

## Runtime state: `AppState`

`AppState` is the dependency graph passed into Axum handlers and cloned into async tasks. It is defined in `crates/server/src/daemon/state.rs`.

```mermaid
flowchart TB
    appstate["AppState"]
    db["db<br/>db::Database<br/>users/labs<br/>nodes/images"]
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
| `db` | Shared handle to the embedded database (`db::Database`). Each operation takes its own connection, and writes queue on its write lock (`DATABASE.md`). All persistent user/lab/node/image state flows through this. |
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
    rpc["rpc::dispatch<br/>ServerMessage"]
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

- Askama template structs are defined in `crates/server/src/templates.rs`. Each derives `askama_web::WebTemplate`, so a handler returns it as the response: a template that fails to render is logged and answered with a 500. The error pages wrap themselves in `WebTemplate` to keep their 404 or 403 status.
- A page that contains a partial also swapped on its own by HTMX (the SSH key lists) holds the partial's template as a field and renders it in place, so the partial's render errors fail the page.
- SSE progress streams (`api/sse.rs`) render fragments with `render_fragment`. The stream has already started, so a fragment that fails to render is logged and sent empty.
- Template files live in `crates/server/templates/`.
- Static assets are embedded from `crates/server/web/static/` using `rust_embed`.
- HTMX is used for form submissions, partial updates, node-table polling, lab-grid updates, and progress pages.
- Templates only render. The handler, or a view type it builds, decides from enums with exhaustive `match`es and passes the result: a `templates::Badge` (label, tone, size) drawn by `snippets/badge.html.jinja`, a `KindBadge` drawn by `snippets/kind-badge.html.jinja`, the `LabActions` or `NodeActions` a lab or node offers, `FormChoice` rows (value, label, selected) for every dropdown of the image edit form, the sidebar's `Nav` built from a `NavPage`, and the login page's `LoginError`. The view types are in `crates/server/src/templates/`, each with a test over every variant. A template does not compare an enum, its text, or call a method that decides; `templates::tests::templates_decide_nothing_from_an_enums_text` rejects `to_string()` and comparisons with string literals in template tags. Class names stay in the templates, because Tailwind only scans `templates/**/*.jinja`.
- Templates carry no inline JavaScript: no `on*=` handlers and no `<script>` blocks without `src`. Buttons name their action and target in `data-` attributes, and the scripts in `web/static/js` listen for them. HTML escaping does not make interpolation into a handler safe, because the browser decodes the escapes before running it. `templates::tests::templates_carry_no_inline_javascript` enforces this.

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

HTTP handlers live in `api/handlers/`, one file per resource and transport: `auth`, `profile`,
`system`, `labs_api`, `labs_web`, `admin_web`, `images_api`, `images_web` and `users_api`. They use Axum extractors (`Path`, `Query`, `Json`, `Multipart`, `State`) and return either `Json<T>`, generic `IntoResponse`, or SSE streams.

REST auth is implemented by `AuthenticatedUser` in `api/extractors.rs`. It tries bearer auth first and then cookie auth. Admin-only REST handlers call `require_admin_auth(&auth)` inside the handler.

`GET /api/v1/labs` requires authentication and lists the caller's labs. Admins may pass `?username=` to list another user's labs; regular users get 403 for any other username.

### WebSocket JSON-RPC

The WebSocket stack lives under `api/websocket/`:

```mermaid
flowchart TB
    get["GET /ws"] --> wshandler["handler.rs::ws_handler"]
    wshandler -->|upgrades connection| inner["handle_socket_inner"]
    inner --> split["split WebSocket sender/receiver"]
    split --> conn["create Connection { id, sender }"]
    conn --> insert["insert into AppState.connections"]
    insert --> metric["increment ws connection metric"]
    metric --> connected["send ConnectedMsg"]
    connected --> msgloop["loop over incoming ClientMessage"]
    msgloop --> rpcreq["RpcRequest { id, method, params }"]
    rpcreq --> dispatch["spawn rpc::dispatch(...)"]
    dispatch --> lookup["find the method in rpc::METHODS"]
    lookup --> authn["authenticate once: Public, User or Admin"]
    authn --> streaming{"streaming?"}
    streaming -->|yes| forward["forward ProgressSender messages to the connection"]
    streaming -->|no| call["call the resource module"]
    forward --> call
    call --> respond["send RpcResponse, record rpc_duration"]
```

`api/websocket/rpc/dispatch.rs` holds `METHODS`, the one table of RPC methods: each method's
access (`Public`, any authenticated `User`, or `Admin` with its refusal message) and whether it
streams status messages before its response. `dispatch` authenticates the caller as the table
says, so the method functions in `auth.rs`, `labs.rs`, `images.rs`, `users.rs` and `server.rs`
only parse their params, call the service and return `Result<Value, RpcError>`; `parse` and
`respond` turn parse failures and service results into RPC errors and results. A test checks
`METHODS` against the published API spec (`shared::api_spec`): every RPC method's access and
streaming must agree, and the spec's `transports.rpc.streaming` says which methods stream.

The streaming methods are `up`, `destroy`, `resume`, `redeploy`, `image.import`, `image.pull`
and `image.download`.

`node.secrets` (a regular method) rewrites a container node's secret files; its params are never
logged, and a parse error is not echoed back. `up` and `redeploy` carry secret files too, so
all three parse with `parse_quietly`, which drops serde's message because it can quote values.

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
    resolve["auth::session::lookup<br/>session row + its user (DB)"]
    ctx["AuthContext<br/>username + is_admin + session_id"]
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

### Sessions and identity

Every token names a login session, and identity comes from the session's row, not from the
token. The JWT carries `sid`, the key of a row in the `session` table, besides `sub`, `iat`,
`exp` and `is_admin`. After the signature and expiry check, every extractor, the RPC
`authenticate_request` and `auth.validate` present the session with `auth::session::lookup`,
which reads the session and its user in one query:

- A token is accepted only while its session is active: the row exists and its `expires_at` has
  not passed. Otherwise REST returns 401, HTML routes clear the cookie and redirect with
  `session_expired`, RPC returns `AuthRequired`, and `auth.validate` answers `valid: false`.
- `username` and `is_admin` in `AuthContext` come from the user row. Demoting an admin takes
  effect on their next request, not when their token expires.
- Sessions belong to a user's id, not their name, so a token of a deleted user never works for a
  new account with the same name.
- If the lookup itself fails, REST returns 500 and HTML routes return 503.

The token still carries `sub` and `is_admin`, and the UI and CLI may use them for display, but no
authorization decision reads them.

The session states (`Active`, `Expired`, `Ended`), the events that move them and the code behind
each are defined in one place, `auth/session.rs`, with a table test. What ends a session:

| Event | Effect | Code |
|---|---|---|
| Logout: web `POST /logout`, REST `POST /api/v1/auth/logout`, RPC `auth.logout` (`sherpa logout`) | ends that session only | `session::logout` |
| Users change their own password | ends their other sessions, keeps the one they used | `db::change_user_password`, from `services::user_password` |
| An admin resets another user's password | ends all of that user's sessions | the same |
| The user is deleted | ends all their sessions | `session.user` is `ON DELETE CASCADE` |
| `expires_at` passes | refused at once; the row is deleted by the next sweep | `session::run_sweeper`, at startup and every `SESSION_SWEEP_INTERVAL_SECS` (an hour) |

A password change writes the new hash and deletes the sessions in one transaction. Web logout
ends the cookie's session before it clears the cookie; if the session cannot be ended it returns
503 and keeps the cookie, so the user is not shown as logged out while the token still works.
A session lasts `JWT_TOKEN_EXPIRY_SECONDS` (7 days), or 30 days for a web login with "remember me";
the token's `exp` and the row's `expires_at` are the same second.

### Services enforce authorization

Every lab service takes the caller's `&AuthContext` and checks it with the helpers in `auth/policy.rs` before it sends progress, takes a lock, or touches a resource:

| Rule | Helper | Operations |
|---|---|---|
| Lab owner or admin | `require_lab_access` | inspect, destroy, down, resume, cancel, link impairment |
| Lab owner only, no admin bypass | `require_lab_owner` | redeploy and `node.secrets` (they rebuild or write into the owner's nodes); `download` and `node.host_keys` (they serve connecting to the nodes) |
| Admin only | `require_admin` | clean |
| Self only, admins included | `require_self` | adding an SSH key, setting a device password |
| Self or admin | `require_self_or_admin` | listing another user's labs, removing an SSH key |
| Any authenticated user | none | up (creates the caller's own lab), listing your own labs |

`require_lab_access` and `require_lab_owner` return the lab record, so services do not look it up twice. When an admin acts on another user's lab, the helper logs a `warn` line with `event = "auth.admin_lab_access"`, naming the admin, the lab and the owner. Refusals log `event = "auth.access_denied"`.

#### The admin boundary

Admins see and manage every lab, but only a lab's owner connects to its nodes (#649). The owner
is the nodes' only login, with the owner's keys and device password, so nothing an admin can get
through Sherpa lets them log in:

- **Files.** `download` (RPC, and the web lab zip; REST has none) and `node.host_keys` are
  `require_lab_owner`. The lab page offers the download link only to the owner
  (`labs_web::lab_offers_download`).
- **Consoles.** `inspect` stays owner or admin, but `inspect::shown_to` blanks each node's
  `console_address` and `vnc_port` for anyone but the owner, using `check_lab_rule` with
  `LabRule::OwnerOnly`, and logs `event = "auth.owner_only_hidden"`. RPC, REST, the CLI and the
  web lab and node pages all read the same response.
- **Keys.** `user.ssh_key.add` is `require_self`, so an admin cannot put a key on another
  account for its owner's next `up` or `redeploy` to install; the admin user page has no add
  form. `user.create` takes no `ssh_keys`, and `CreateUserRequest` refuses unknown fields so an
  old request with keys fails instead of creating a keyless user. Removing a key stays self or
  admin. `sherpad init`, run as root on the host, still takes the first user's key.
- **Client.** `sherpa ssh`, `sherpa console` and `sherpa provision` call
  `lab_identity::require_lab_owner` before anything connects: when the folder's
  `lab-info.toml` names an owner other than the logged-in user they stop with `Only the owner of
  lab '<id>' can connect to its nodes`. This is a courtesy; the server rules above are the
  boundary.

What remains: an admin who resets a user's Sherpa password can sign in as them (logged, and it
ends the user's sessions). An admin may start privileged containers and host mounts, which is
root on the host, and root reaches the database key, disks, serial consoles (`127.127.x.y:23`)
and VNC (`localhost:59xx`); host-level isolation is #674 and #683. The boundary is for the admin
role through Sherpa's interfaces, with an audit trail, not a defence against root.

Refusals are returned as a typed `services::error::ServiceError` inside `anyhow::Error`: `Denied` (not allowed), `NotFound`, or `Conflict` (allowed, but the current state forbids it, such as deleting a user who owns labs). Transports find it with `ServiceError::find`, which works through added context:

- REST: `ApiError::from` maps `Denied` to 403, `NotFound` to 404 and `Conflict` to 409. No authorization decision is made by matching on error text.
- RPC: `service_error` in `rpc/dispatch.rs` maps them to `AccessDenied`, `NotFound`, `InvalidParams`, and `ServerError` with the conflict message (JSON-RPC has no conflict code); other failures stay `ServerError` with the error's message chain (`{:#}`, never its debug form) as context. Authentication failures carry no context.
- HTML: `lab_error_page` and `lab_error_fragment` in `handlers/labs_web.rs` render the 403 and 404 templates.

Request structs (`UpRequest`, `DestroyRequest`, `InspectRequest`, `RedeployRequest`, `UpdateImpairmentRequest`) do not carry a username. Identity only ever comes from the `AuthContext`. Older clients that still send `username` keep working, because unknown fields are ignored.

### Device passwords

A user's device password is their login password on lab nodes (#649). It is stored sealed (see
[`DATABASE.md`](DATABASE.md#device-passwords)) and set through
`services::user_device_password::set_device_password`, which the RPC method
`user.device_password.set`, `POST /api/v1/admin/users/{username}/device-password` and the
Profile page's form (`POST /profile/device-password`) all call:

- **Self only.** `auth::policy::require_self`: only the account's own user sets it, admins
  included, because whoever sets it knows it. There is no read and no clear. The web form names
  no user and refuses a form that does (`deny_unknown_fields`).
- **Not the built-in `admin`.** `require_lab_starter`: that account cannot start labs, so it has
  no node login.
- **Login password.** The request carries the caller's Sherpa login password, checked against
  the stored hash, so a stolen session cannot set one (`Denied`).
- **Rule.** `validate::validate_device_password`: one rule for every node model, with a message
  for each part (`Invalid`). It must also differ from the login password, checked with the
  stored hash (`Invalid`).

`sherpa server user device-password` prompts for the login password and the new device password
twice; none is ever a flag. `user.info`, `user.list`, the CLI and the Profile page show only
`device_password_set`. `up` and `redeploy` require it and
write it into the node configurations as the password of the owner's login
(`guest_account::require_device_password`, see
[`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md#device-password)); `resume`, `down`, `destroy` and
`inspect` do not. The refusal names `sherpa server user device-password` and the Profile page.

### When a transport checks first

A few transports must answer before the service runs, and call the same policy helper first:

- REST destroy and REST redeploy return SSE streams, so a refusal inside the service would arrive as a 200 with an error event. They call `require_lab_access` or `require_lab_owner` first, so a refused caller gets 403.
- Web redeploy checks ownership before reading the lab's stored definition, so a non-owner gets 403 rather than a refusal.
- The web lab download checks ownership first, since it needs the lab's name for the zip.
- Web destroy checks access before queuing the job.

The service always checks again, so the rule is still enforced if a transport forgets.

### Web jobs

Web UI create and destroy run as jobs: the POST queues a `Job` in `AppState::pending_jobs` and redirects to `/jobs/{id}`. The job records its `requester` (`AuthContext`, with the session it was queued through). The job page and `/jobs/{id}/stream` serve the job only to its requester or an admin; another user gets 403, and the job is left for its owner. When the stream starts the job, `Job::resolve_requester` presents the requester's session again: if it has ended or expired since the job was queued (logout, a password change, the user deleted), the job does not run and the stream reports why; otherwise it runs as the requester with their current admin flag.

### Other rules

- Browser page access uses cookie-only extractors.
- REST API access uses bearer-or-cookie extractors.
- WebSocket RPC access requires `params.token`.
- Password changes go through `services::user_password::change_password` on every transport (web profile, web admin reset, REST, RPC). Users change only their own password and admins may reset anyone's. Changing your own password, as an admin too, needs the current password, so a stolen token cannot lock the owner out; `sherpa server user passwd` prompts for it. An admin resetting another user's password does not. A change ends the user's other sessions (see [Sessions and identity](#sessions-and-identity)).
- Device passwords go through `services::user_device_password::set_device_password` on every transport; see [Device passwords](#device-passwords).
- Edits of a user row are compare-and-set: `db::update_user` writes only if the row's `updated_at` is still the one it read, and otherwise fails with `db::UserChanged`. `ServiceError::from_user_write` turns that into a `Conflict` ("reload and try again"), so of two concurrent SSH-key edits the second is refused instead of overwriting the first. A password change writes only the hash (`db::change_user_password`), so it cannot undo a concurrent key edit.
- User info operations allow admins to target anyone and regular users to target themselves. These checks are still in the transports, not in services. Every transport builds the `UserInfo` it returns with `services::user_device_password::user_info`, which adds `device_password_set`.
- Logins on every route go through `auth::login::check_credentials`. An unknown username is checked against a dummy hash, so it takes as long as a wrong password and response times do not show which usernames exist.
- `auth::policy::require_host_access` refuses `up` and `redeploy` manifests from non-admins whose nodes are `container.privileged` or bind-mount host paths (`container.volumes`). Either is root on the host.
- The server never opens a path a manifest names. The CLI reads `text_files`, `binary_files`, `systemd_units`, `ssh.authorized_key_files` and the scripts on the client and sends their contents in the matching `*_data` fields; `topology::validate::check_client_read_files` refuses a manifest that lists files without them, and binary file and unit names that are not plain file names.
- Signup is closed unless `allow_signup = true` in `sherpa.toml`. With it off, `GET` and `POST /signup` return 404 and the login page does not link to it.
- User deletion goes through `services::user_delete::delete_user` on every transport. It is admin only and refuses self-delete, the last admin account, and any user who owns labs (`Conflict`, listing the labs). The web admin page shows a refusal in its `#user-delete-error` notification using `HX-Retarget`. The database backs up the owned-lab rule: `lab.user` is `REFERENCE ON DELETE REJECT`, because deleting only the records would leave the labs' VMs, containers and networks running unmanaged. With identity read from the database, the last-admin rule can only be hit by a race (two admins deleting each other at once); there is no transaction around the check and the delete.
- `clean_lab_resources` is the unchecked clean used internally, to roll back a failed `up` on behalf of the user who started it. User requests go through `clean_lab`. Both validate the lab ID first (`Invalid`, a 400 or `InvalidParams`): resources are matched by it and the lab folder is built from it, so an empty or path-like ID would reach every lab.

## Service layer architecture

The services directory is the orchestration layer. Each service is built around a public async function that takes typed request data, `&AppState`, and sometimes a `ProgressSender`.

```mermaid
flowchart TB
    services["services/*.rs<br/>orchestration, progress reporting, rollback, best-effort cleanup, summaries"]
    persistence["Persistence<br/>db crate<br/>embedded Turso records<br/>schema + CRUD"]
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
| Lifecycle services | `up/` | create a full lab and all resources |
| | `destroy.rs` | remove a full lab and all resources |
| | `redeploy/` | replace one node inside an existing lab |
| | `down.rs` | stop all nodes or one node |
| | `resume.rs` | start all nodes or one node |
| | `lab_nodes.rs` | the lab's nodes and images that `down` and `resume` act on |
| Read/model services | `inspect.rs` | read DB + runtime data and build lab inspection output |
| | `list_labs.rs` | list lab summaries for a user |
| | `download.rs` | build the lab's files for the client from the database, with its key and credentials |
| Image/admin services | `import.rs` | image import/list/show/set-default/scan/download support |
| | `container_pull.rs` | Docker/OCI image pull with progress |
| | `clean.rs` | admin force-clean path |
| Network mutation services | `impairment.rs` | update delay/jitter/loss/reorder/corrupt settings on P2P links |
| Background service | `scanner.rs` | observe Docker/libvirt runtime state into the node lifecycle, and check and repair P2p links, for labs no operation holds |
| Node lifecycle | `node_lifecycle.rs` | the only writer of node states: `record` for operations, `observe` for the scanner, the runtime mapping table |
| Lab lifecycle | `lab_lifecycle.rs` | the only writer of lab phases (`record`, under the lab's lease), `require_ready` for operations on nodes, and startup `recover` |
| Shared helpers | `node_ops.rs` | common node setup/building helpers |
| | `progress.rs` | progress message abstraction |

### Node operations

`node_ops.rs` holds what `up`, `redeploy` and `resume` share: generating a node's configuration,
building its domain, and starting it. Arguments that travel together are grouped:

- `LabZtp`: the lab's part of generating any node's configuration (lab ID and directory, images
  directory, the sherpa user, DNS, management network, progress, the owner's account).
  `generate_vm_ztp` and `generate_container_ztp` take it with the node, its image and its
  addresses. Each ZTP method's generator takes the private per-node `NodeZtp`.
- `DomainParts`: the disks, interfaces and network names a VM or unikernel domain is built from,
  for `build_domain_template` and `build_unikernel_domain_template`.
- `container::ContainerSpec`: what `start_container_node` starts, as the `container` crate's
  `run_container` takes it.

### Service call graph for lab creation

`up` is the largest orchestration module. Its job is to convert a validated manifest into persistent records, generated files, runtime networks, and running nodes. `up_lab` runs the phases in turn; each phase is a function in its own file, taking the shared `LabBuild` (lab ID and directory, connections, config, progress) and returning what the next phases need:

| File | What it holds |
|---|---|
| `lab.rs` | `up_lab`: the request checks, setup (phase 1) and planning (phase 2); `LabUp`, whose methods run phases 3 to 13 in groups (records, lab network, links, nodes, readiness) and build the response; rolling back a failed `up` |
| `context.rs` | `LabBuild` |
| `plan.rs` | phase 2: validation against the server, and the `LabPlan` |
| `records.rs` | phase 3: the lab and node records, interfaces, per-node networks |
| `networks.rs` | subnet reservation and management networks (4), container data networks (6), shared bridges (7) |
| `links.rs` | phase 5, and attaching P2p eBPF redirects |
| `ztp.rs` | phases 8 and 9: addresses, certificates, node configurations and domains, the boot container |
| `runtime.rs` | phases 10 and 11: disks, VM and unikernel domains |
| `ssh.rs` | phase 12 |
| `readiness.rs` | phase 13: starting containers and waiting for nodes |
| `services.rs` | Kubernetes clusters, Keycloak realms and Vaults once nodes are ready |
| `nodes.rs` | manifest node expansion and image lookup, shared with `redeploy` |

```mermaid
flowchart TB
    uplab["up_lab(&AuthContext, UpRequest, &AppState, ProgressSender)"] --> pre["Deserialize manifest JSON into topology::Manifest<br/>validate node secrets and Tailscale enrollment<br/>db::validate_lab_id<br/>lab_operations::begin"]
    pre --> setup
    subgraph phase1["1. Setup"]
        setup["Load current user from DB<br/>Guard: lab_id must not already exist<br/>Load node image records from DB"]
    end
    setup --> validation
    subgraph phase2["2. ManifestValidation"]
        validation["topology::validate checks: duplicate device, device count, Talos clusters<br/>validate environment variables<br/>validate::validate_and_resolve_node_versions (server): versions, container image repos<br/>process_manifest_nodes, then lease node images<br/>validate disk method and guest account per node<br/>check_container_image: Docker has each container image<br/>topology::process_manifest_links, process_manifest_bridges (unknown nodes fail)<br/>validate management/reserved/data interface bounds<br/>admission::admit_lab_nodes: CPU pins and memory against the server<br/>validate duplicate links"]
    end
    validation --> startguard["Guard: no disks or VMs left from an earlier attempt with this lab_id<br/>(after validation, before any resource is created)"]
    startguard --> hostnets
    subgraph phase3["3. DatabaseRecords"]
        subgraph reserve["reserve_lab_networks (retried while concurrent labs win a subnet)"]
            hostnets["read host interface networks (IPv4 and IPv6)"] --> mgmtsubnets["allocate IPv4/IPv6 management subnets<br/>(not in DB, not overlapping host)"] --> loopsubnets["allocate IPv4/IPv6 loopback subnets (not in DB)"] --> createlab["db::create_lab: unique subnet indexes make the insert the reservation"]
        end
        createlab --> createnode["for each node:<br/>db::create_node<br/>node_resources::reserve_node_resources (vCPUs, memory, VNC port)<br/>create the node's isolated and reserved libvirt networks and bridges"]
    end
    createnode --> labfiles
    subgraph phase4["4. LabNetworkSetup"]
        labfiles["create /opt/sherpa/labs/{lab_id}<br/>store_ssh_usernames: each node's SSH account on its record<br/>(no lab info, SSH config or manifest file: the database holds them)"] --> mgmtnets["libvirt NAT management network<br/>Docker bridge management network"] --> tailscale["tailscale::provision when the lab joins a tailnet"]
    end
    tailscale --> links
    subgraph phase5["5. LinkCreation"]
        links["for each link: db::create_link, all P2p, with the manifest's impairment<br/>nothing created yet: libvirt makes VM taps, container veths come later<br/>record veths for disabled interfaces on containers with links"]
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
        ssh["build container network attachments<br/>(the SSH config is rendered once the nodes are ready)"]
    end
    ssh --> secrets
    subgraph phase13["13. NodeReadiness"]
        secrets["write container secret files to RAM"] --> startctr["start containers (node_ops::start_container_node)<br/>P2p containers: veths created into the container netns,<br/>then attach_ready_p2p_links for links now complete<br/>node_lifecycle::record Launched"]
        startctr --> ready["containers: kept running CONTAINER_SETTLE, then their ready_check.port with check_vm_ready<br/>poll VMs with node_ops::check_vm_ready, record host keys<br/>poll unikernels with check_unikernel_ready<br/>no wait for ready_check.skip nodes or Talos cluster workers"]
        ready --> refresh["ssh_config::lab_ssh_files from the database, with recorded host keys<br/>bootstrap Kubernetes clusters"]
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
    stopops --> reread{"lab still recorded?"}
    reread -->|no: another destroy removed it| already["Return success: already destroyed"]
    reread -->|yes| started["lab_lifecycle::record DestroyStarted: the lab is Destroying"]
    started --> listcontainers
    subgraph containers["destroy_containers"]
        listcontainers["container::list_containers"] --> kill["kill matching lab containers if running"] --> removecontainers["remove matching lab containers"]
    end
    removecontainers --> secrets["destroy_lab_secrets: remove the lab's container secret files"]
    secrets --> listdomains
    subgraph vms["destroy_vms_and_disks"]
        listdomains["list libvirt domains"] --> undefine["undefine/destroy domains named node-lab_id"] --> pool["lookup Sherpa storage pool"] --> volumes["delete volumes named node-lab_id plus a disk suffix"]
    end
    volumes --> listnetworks
    subgraph dockernets["destroy_docker_networks"]
        listnetworks["container::list_networks"] --> deletenetworks["delete networks named name-lab_id"]
    end
    deletenetworks --> listlibvirtnets
    subgraph libvirtnets["destroy_libvirt_networks"]
        listlibvirtnets["list libvirt networks"] --> undefinenets["destroy/undefine networks named name-lab_id"]
    end
    undefinenets --> findifaces
    subgraph interfaces["destroy_interfaces"]
        findifaces["network::list_interface_names"] --> deleteifaces["delete the lab's Sherpa-created bridge/veth/tap interfaces"]
    end
    deleteifaces --> anyerror{"any resource error?"}
    anyerror -->|no| removedir["remove lab directory"]
    removedir -->|removed| deletelab["lab_lifecycle::record Destroyed:<br/>db::delete_lab_from, the schema deletes its nodes, links and bridges"]
    removedir -->|failed| failed
    anyerror -->|yes| failed["lab_lifecycle::record DestroyFailed:<br/>record and folder kept for a retry"]
    deletelab --> response["Return DestroyResponse { success, summary, errors }"]
    failed --> response
```

Destruction uses name-based matching for many runtime resources because the runtime backends are external systems and the database may not be fully reliable after partial failures. This is why consistent naming with `lab_id` embedded in resource names is part of the architecture.

Every resource is matched by its whole name. Everything Sherpa creates for a lab is named
`<name>-<lab_id>` (domains and containers, Docker and libvirt networks, bridges, taps and veths),
and `shared::util::is_lab_resource` checks that form; `libvirt::is_lab_volume` adds one of
`shared::konst::LAB_VOLUME_SUFFIXES` for disks. A substring match would also take another lab's
resources when one of its names contains this lab's ID. Redeploy removes a container node's
data networks with `container_networks::is_node_network` (`<node>-brs<n>-<lab_id>` or
`<node>-iso<n>-<lab_id>`), so node `r1` never takes the networks of node `r1-x`. A new kind of disk needs its suffix in
`LAB_VOLUME_SUFFIXES`, or destroy leaves it behind. The startup guard in `up` and the disks
`inspect` lists use the same rules.

The lab name comes from the lab's database record, so destroy also works when `up` was stopped
before it wrote the lab info file.

`destroy::tear_down_lab` runs these steps for destroy, admin clean and the rollback of a failed
`up`, under the lab's lease, and moves the lab's phase through `Destroying` (see
[`STATE_MACHINES.md`](STATE_MACHINES.md#7-destroy-and-clean)). When any resource fails to go,
`cleanup_lab_metadata` keeps the lab's database record and folder, and the lab is left
`DestroyFailed`, so its owner can destroy it again. Otherwise the folder goes first and the record
last, so a folder that cannot be removed still has a record to destroy it through. A destroy that
waited behind another and finds the lab gone reports success. Admin clean also works on a lab with
no record: it removes the resources and the folder named for the lab ID.

The five teardown steps are `destroy::destroy_lab_resources` (`destroy_compute`: containers,
secret files, VMs and disks; then `destroy_networking`: Docker networks, libvirt networks,
interfaces). `clean` (admin clean, and the rollback of a failed `up`) runs the same function
without a progress sender, so both remove exactly the same resources.

### Operation lifecycle and stopping operations

Long-running operations (`up`, `destroy`, `redeploy`, `resume`, `down`, admin `clean`, image
operations) always run as their own tasks, on every transport, and keep running when their
client disconnects. The WebSocket and SSE handlers spawn them; the plain HTTP handlers wait on
them through `run_detached` in `api/handlers/labs_api.rs`, because axum drops a handler's future when
its client disconnects and would otherwise stop the operation at an arbitrary await point.

`services/lab_operations.rs` keeps, per lab, a read/write lease and a cancellation token with
a stop reason:

- `up`, `redeploy`, `resume` and `down` call `lab_operations::begin` and hold the read lease
  until they return;
- the scanner calls `lab_operations::try_observe`, which takes the write lease without waiting
  and without cancelling anything; a lab an operation holds is skipped that cycle, and an
  operation that starts meanwhile waits while the scanner holds it: a few milliseconds, or a
  few netlink calls and two program loads per link when it repairs P2p links, and a netlink call
  or a libvirt device update per end whose carrier changes;
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

A requested stop removes the lab's token when it releases the lease, so an operation queued
behind it would start with a fresh token ahead of a destroy waiting behind both. `stop` counts
pending destroys per lab, and `begin` cancels a new operation with `Destroy` while one is
pending. `redeploy` and `resume` check access before taking the lease, and again once they hold
it, so a lab destroyed while they waited is not rebuilt.

The CLI (`cmd/interrupt.rs`) waits for `up`, `redeploy` and `resume` while listening for
Ctrl-C. On Ctrl-C it asks whether to stop the operation on the server too, defaulting to no; a
yes opens a new connection and calls `cancel`.

`up` holds read leases on its node images so they cannot be deleted while it provisions. VM
and unikernel image leases end once every disk and domain exists (node records and overlay
backing files protect the images from then on), so a long readiness wait does not block image
pull, import or delete. Container image leases last until the containers start.

### Admission: memory and CPU pins

Nothing else serializes operations across labs, so `services/admission.rs` admits the nodes an
operation will start, one request at a time, before anything is created:

- `up` calls `admit_lab_nodes` in phase 2 (`up/plan.rs`), and holds the `Admission` in its
  plan until it returns;
- `redeploy` calls it in stage 1 (`check_fits_server` in `redeploy/context.rs`), before the
  node is torn down, and holds it in `Redeploy`. The node being replaced does not count
  against itself;
- `resume` and node start call `admit_stopped_nodes` (`resume.rs`) for the target VMs,
  unikernels and memory-limited containers that are stopped, before the Tailscale gateway is
  enrolled or any node starts. Paused and running ones already hold their memory, and stopped
  nodes keep their pins in the database, so nothing is claimed.

Requests wait on a Tokio mutex, which grants the lock in arrival order. Under it, the request's
CPU pins are checked (`node_resources::check_cpu_pins_on_server`) against the `cpu_pin` table
(see [CPU pins in admission](#cpu-pins-in-admission)), its containers' CPU quotas against the
server's CPUs (`validate::check_container_cpus`; containers take no part in pinning), and its
memory against the server (`validate::check_memory_fits`). Once all of them pass, the nodes'
pins are claimed in the table (`db::claim_cpu_pins`, one transaction), whose trigger refuses a
CPU another node holds; a refusal there is mapped to the same `ServiceError::CpuPinsHeld` and
claims nothing. The lab's record must exist: `up` reserves it in phase 1. Admitted nodes'
memory goes into the ledger, keyed by domain name (`<node>-<lab id>`), so the next request
counts it before libvirt shows them running. Dropping the `Admission` when the operation ends
releases the memory; by then libvirt counts the ones that run. Pins are not in the ledger: they
stay claimed in the database until the lab is deleted or a redeploy keeps one set. A domain
that is running and pending counts once, as pending. The ledger is in memory: a restart ends
the operations it held, and leaves their pins claimed.

Memory is compared with what is committed, not with free memory. QEMU allocates guest memory
as the guest touches it, so free memory at start overstates the room a lab has. The server's
guest memory is its total (`virNodeGetInfo`) less its hugepage pools and
`host_memory.reserve_mib`, times `host_memory.overcommit_percent` percent (both in
`sherpa.toml`, 4096 MiB and 100 by default; startup refuses a percentage below 1). The limit is
computed in whole MiB with saturating integer arithmetic, rounded down. Against it count every running or
paused domain's maximum memory, Sherpa's and others', the pending nodes, and the request. A VM
held strictly on a NUMA node (`numa.node`, with `numa.memory_mode` `strict` or left out;
`NodeNuma::strict_node`) is also checked against that node: its memory (from the
capabilities XML) less its hugepages and its share of the reserve, against the domains held
strictly on it (`numatune` mode `strict` or `restrictive` with one node). Memory that is not
bound to one node, including a VM with `numa.memory_mode = "preferred"`, whose memory falls
back to other nodes when its node is full, counts only against the whole server, both as a
request and once it runs. A container with a memory limit counts like a domain: its limit
when it is requested, and, read from Docker (`container::list_container_memory`), while it
runs or is paused, Sherpa's and others'; its memory is not bound to a NUMA node. A container
without a limit counts as nothing; the reserve covers those and QEMU's own memory. A refusal names the numbers that fell short
and the two settings. `libvirt::host_memory` and `libvirt::list_domain_memory` read the server
(`crates/libvirt/src/host.rs`). The capabilities XML is read with `quick-xml`'s serde
deserialiser into private structs that hold only `<host><topology><cells>`: each cell's id,
`<memory>` and `<pages>` pools, with every other element skipped. The numbers stay text in the
structs and are parsed afterwards, so a missing or bad value is reported by name. See `docs/investigations/host-memory-oom.md` for why.

### CPU pins in admission

The database holds every node's pins, in the `cpu_pin` table (see
[`DATABASE.md`](DATABASE.md#cpu-pins)), and its trigger enforces the pinning rules on every
insert. Admission checks first, to give a refusal that names the holder, and then claims.

`node_resources::check_cpu_pins_free` decides whether the requested nodes' pins are free. It
reads every node's rows (`db::list_cpu_pins`), its lab and the lab's owner. The holders are
every node in the table except the requested node itself: its own rows are its own, since it
is being replaced. A lab still being created (`LabPhase::Creating`) holds its pins from `up`'s
phase 2, before its node rows exist.

A node another operation redeploys holds both its old and its new pins until that redeploy
keeps one set. Admission claims the new pins beside the old ones (`claim_cpu_pins` keeps the
rows a node holds; the trigger lets a node overlap only itself), and `Admission` returns the
pins the node held before (`take_pins_before`). The node keeps one set at one of two points:

| The redeploy stops at | Domain | The node's rows while it runs | Kept after (`retain_cpu_pins`) |
| --- | --- | --- | --- |
| Stage 1, refused | old, running | old | old (nothing was claimed) |
| `RebuildStarted`, host key, teardown (`destroy_node`) | old, or partly torn down | old and new | old (`keep_old_pins`) |
| VM networks (`recreate_vm_networks`) | none | old and new | old (`keep_old_pins`) |
| `reserve_node_resources` | none | old and new | new: `rebuild` keeps them right after it |
| Certificate, domain or container, readiness | new, or none | new | new |
| Success | new | new | new |

Each row's "kept after" covers the domain that may run: a failure before the old domain is
gone keeps the old pins, which it may still use, and from `reserve_node_resources` on the node
keeps the new ones. No other request can take either set while the redeploy runs, so no two
nodes end up on one vCPU CPU. A server restart in the middle leaves the node holding both sets,
a superset, until it is redeployed or destroyed; a failure to give the new pins back is logged
and leaves the same.

A requested node's overlap with a holder is one of: its vCPU CPU on the holder's vCPUs or
QEMU threads, or its QEMU thread CPU on the holder's vCPUs; QEMU threads may share. A new
overlap is refused with `ServiceError::CpuPinsHeld` (see [`API.md`](API.md#typed-refusals)),
naming the CPUs and roles in its `data`. The message names the holding node and lab
(`node 'n1' of lab 'name' (lab id)`, with ", which another operation is starting" for a lab
being created) to admins and to the holding lab's owner, and says "a node of another lab" to
everyone else. Who is asking comes from the request's `AuthContext`. Should the trigger refuse
a claim the check passed, `db::CpuPinTaken` becomes the same refusal
(`ServiceError::from_cpu_pin_claim`), saying "another node".

An overlap the requested node's own rows already had with the same holder's rows was stored
before the database enforced the rules, and copied as it was by migration 23. It is carried:
the node starts, and the overlap is logged (`tracing::warn!`) and sent to the user as a
`Warning:` status. `redeploy` gets these from the same check; `resume` and node start
(`admit_stopped_nodes`) compare the stopped nodes' rows with every other node's and only warn,
as a lab that exists is never refused for its pins. A new lab has no rows, so its `up` never
warns. sherpad also logs each such pair at every start (`node_resources::warn_shared_cpu_pins`).

Pins are held while a lab is stopped. Destroying a lab deletes its record, and the foreign key
deletes its rows; a lab left `DestroyFailed` keeps them until a destroy succeeds, as its domains
may still exist. A refused or failed `up` gives its pins back with its lab record.

### Redeploy architecture

Redeploy is a controlled single-node replacement. It exists because updating node configuration in place is often less reliable than destroying and recreating that node.

The CLI sends the manifest it resolved in the lab folder, with every file's contents and every
`$VAR`'s value, and the parts as written beside it. The web UI has neither: it rebuilds the node
from the lab's stored definition (`lab_definition::web_redeploy_manifest`), which holds no file
contents or resolved values. So it refuses, with a `ServiceError::Conflict` (409), a lab with no
stored definition, and a node whose definition needs any of them
(`validate::check_web_redeployable`): a `ztp.config`, `text_files`, `binary_files`,
`systemd_units`, `startup_scripts`, `user_scripts`, `ssh.authorized_key_files`,
`environment_variables` with a `$VAR` value, and provisioners, which only the CLI runs. The
error names the node and every missing part, and says to run `sherpa redeploy -n <node>` from
the lab folder, for example:

> Node 'dev01' cannot be redeployed from the web UI: it needs ztp.config, text_files, which the
> server does not keep. Run `sherpa redeploy -n dev01` from the lab folder

A node whose definition 0.6.4 or 0.6.5 imported from a saved manifest, and that has not been
redeployed from the CLI since, has no `ztp.config`, environment variables or provisioners in it
(see [`DATABASE.md`](DATABASE.md#imported-definitions)). Those releases marked such nodes and
refused them here; 0.7.0 dropped the mark, so nothing tells them apart and the web UI rebuilds
one without those parts. The 0.7.0 upgrade notes say to redeploy them from the CLI, or destroy
their lab, before upgrading.

The node is rebuilt from the image `up` would give it: the manifest's `version` of its model and
kind, or else their default (`redeploy/context.rs`, `redeploy_image`). So a node whose manifest
names another version, such as a container node moved to a new tag, is rebuilt from that
version, not from the image it was built from before. Its model and kind cannot change: the
stored image's are checked against the manifest's. Once the old node is torn down, the node's
record is pointed at the new image (`db::update_node_image_ref`), so the old image is free to
delete and the new one is protected.

A node that needs none of them is redeployed from the definition narrowed to it: the other
nodes are left out, so their files do not refuse it, and the redeploy stores only that node's
definition and the lab-level settings, as from the CLI. Its secret files and parts as written are
the ones it has. `redeploy_node` (`node.rs`) runs the stages; each is in its own file under
`services/redeploy/`:

```mermaid
flowchart TB
    request["redeploy_node (node.rs)"] --> owner["require_lab_owner in the service<br/>(owner only, admins included), owner has an SSH key<br/>lab_operations::begin, require_lab_owner again<br/>the lab's redeploy lock"]
    owner --> load
    subgraph stage1["1. load_redeploy (context.rs): nothing is changed"]
        load["manifest checks: host access, client files, provisioners, secrets<br/>node and image from the DB; same model; unikernels refused<br/>resolve and check the container image<br/>lease the image, check the disk method<br/>lab info, management network, DNS, lab CA, Talos worker config<br/>the lab's SSH key pair (still made; no node account uses it)<br/>saved management addresses, owner account (SSH key, device password) and node login<br/>node settings by kind, VM tuning, platform<br/>admission: CPU pins and memory"]
    end
    load --> starting["clear the host key, node Starting"]
    starting --> destroy
    subgraph rebuild["rebuild (node.rs): a failure records the node Failed"]
        destroy["2. destroy_node (teardown.rs): the VM or container, the node's ZTP directory"] --> vmnets["3. recreate_vm_networks (teardown.rs): isolated and reserved networks, VMs only"]
        vmnets --> resources["reserve_node_resources (keeps the VNC port)<br/>reissue the node certificate"]
        resources --> ctr["container.rs: generate_container_ztp<br/>no links: Docker macvlan data networks; P2p: a veth per data interface<br/>start_container_node (Starting), veths into the netns, reattach_p2p_redirects<br/>Vault unsealed, Keycloak realm made again<br/>ready_check.port answers (ready.rs), if set, then Running"]
        resources --> vm["vm.rs: generate_vm_ztp, clone_node_disks<br/>vm_interfaces from the DB links and bridges<br/>build_domain_template, create_vm (Starting), reattach_p2p_redirects<br/>isolated bridge down; wait for the ready port, host key, Running"]
    end
    ctr --> ssh["db::update_node_ssh_username: the account the node was rebuilt with"]
    vm --> ssh
    ssh --> stream["RedeployResponse"]
```

A refused redeploy leaves the node, its state and its host key as they were; a node another
operation is building is refused before teardown. Teardown records it `Building`, the started
node `Starting`, and a ready one `Running`: a container only once Vault and Keycloak are
restored. Once the node is torn down, a failure records it `Failed`, which the scanner never
overwrites. A VM that is not ready within the ready timeout is left `Starting`, the redeploy
succeeds, and the scanner records it `Running` once it answers on its stored ready port.
Before the node starts, redeploy stores the new manifest node's readiness check on its record,
and its own wait reads it from there, as `resume` and the scanner do. It also stores the node's
definition as written and the lab-level settings a redeploy applies (`db::store_redeployed_node`;
see [`DATABASE.md`](DATABASE.md#lab-definitions)), from the parts as written the CLI sends
beside the manifest; a request without them, from the web UI, keeps the node's stored
`ztp.config`, environment variables and provisioners. No file content or resolved value is
stored. The node lifecycle is
in [`STATE_MACHINES.md`](STATE_MACHINES.md#1-node-lifecycle-nodestate).

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

After the access checks, and before starting anything, resume admits the stopped VMs,
unikernels and memory-limited containers against the server's memory (see [Admission](#admission-memory-and-cpu-pins)); a
refusal fails the whole call.

Both hold the lab's operation lease (`lab_operations::begin`), so a destroy waits for them and
the scanner leaves the lab alone while they run. Both move nodes only through the node
lifecycle (`node_lifecycle::record`): a node another operation is building (`Building`), or one
with no domain or container (`Missing`), is refused before anything is done to it.

Both are idempotent per node. A VM already stopped or running is reported as such. Resume
reads the node's runtime through the one mapping table (`node_lifecycle::libvirt_runtime` and
`container_runtime`) and decides with `start_step`: it boots a shut-off domain or an exited or
created container, resumes or unpauses a paused one, leaves a running one, and refuses a
crashed domain, a dead container, one between states and one that does not exist. Its result
is a `StartOutcome` (`ColdBooted`, `Resumed`, `AlreadyRunning`, `Failed`), and that, not the
message text, decides which nodes get their links re-attached, with their carrier set to follow
each link's state, and are waited for. A cold-booted
node is recorded `Starting`, then `Running` once ready; a resumed one `Running`; a running one
is left as it is. `down` pauses only a running container.

`down` routes each node (`shutdown_route`): VMs are asked to shut down (`domain.shutdown()`:
libvirt uses the guest agent when present, otherwise the ACPI power button); Talos cluster
nodes, whose kernel ignores the power button, get a graceful shutdown over the Talos API
(see `TALOS.md`). `down` then polls until the VM stops and forces a power-off only if it is
still running after `VM_SHUTDOWN_TIMEOUT` (60 seconds) or the request fails. Nodes stop in
parallel in two waves, Talos cluster controllers last, so a lab takes at most about two
timeouts. Unikernels and Talos nodes outside a Sherpa-built cluster are powered off directly,
and the result says why. A node is marked `Stopped` only once it has stopped. Forcing guests off without asking lost recently
written files on every VM without a guest agent, which includes Talos and the Ubuntu images.

Resume then waits for the nodes it cold-booted, as `up` does, on the readiness check stored
with each node (`DbNode::ready_check`). It reads the lab's stored definition
(`lab_definition::stored_definition`) only for `ready_timeout`, and for a cold-booted
container's data interface count; a lab without one uses the default timeout and the image's
count, logged at warn. VMs are probed on their stored port, unikernels with
`check_unikernel_ready` and their stored port, if any, and containers with a stored port on
that port. Other containers, nodes whose check is skipped, and paused nodes are not waited
for. A node that is not ready within the timeout
is reported in its result message, not as an error, and stays `Starting` until the scanner's
probe answers. The WebSocket `resume` RPC is streaming
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

#### Deleting an image

`services::delete::delete_image` holds the image's lifecycle lock (`lock_image`) through its
checks and the deletion, so no node is built on the image in between:

- **A VM or unikernel image** is refused while a volume of the storage pool, running or stopped,
  is built on a file in its version folder. `image_lifecycle::ensure_image_has_no_overlays` asks
  libvirt (`libvirt::pool_backed_volumes`, through the `BackedVolumes` trait that `Qemu`
  implements) for each volume's backing file, because sherpad cannot read the disks libvirt has
  handed to QEMU: they are `0600` and owned by `libvirt-qemu`, or by `root` once the VM stops.
  `libvirt::ensure_no_overlay_dependencies` resolves each backing file, links included, so a
  base reached through a renamed model's link is protected. A disk built on the image refuses the
  delete with a `ServiceError::Conflict` naming the disk; a pool libvirt cannot read, or a
  backing file that cannot be resolved, fails it. Disks on other images do not.
- **A container image** is refused while a running container runs it:
  `container::running_containers_of_image` resolves the recorded reference, its repo at its
  version, to Docker's image ID and lists the running containers with that ID, and refuses with a
  `ServiceError::Conflict` naming them. Once the row is
  deleted, `container::remove_image` removes the reference through Bollard, forced so a stopped
  container does not keep it; another tag of the same image, such as the one it was pulled as,
  is left. A container image without a repo has no Docker image to remove.
- **Any image a node records** is refused by the database: `node.image` is `ON DELETE
  RESTRICT`.

The row is deleted first, then the version folder or the Docker image; a failure to remove them
is logged and reported as `image_deleted: false`, the row being gone.

Admin-only image mutations are enforced at the transport boundary. Image list/show require authentication but not admin privileges. Long-running import/pull/download paths use `ProgressSender` so REST, WebSocket, and UI callers can receive progress without service-specific transport code.

#### Image routes name the model and its kind

Images are keyed by model, kind and version, so every route that names one image names all
three; there is no path without the kind and no alias for the paths of 0.6.

| Route | Handler |
|---|---|
| `GET /api/v1/images/{model}/{kind}` (`?version=`, else the default version) | `images_api::show_image_json` |
| `DELETE /api/v1/images/{model}/{kind}/{version}` | `images_api::delete_image_json` |
| `POST /api/v1/images/{model}/{kind}/{version}/default` | `images_api::set_default_image_json` |
| `GET /admin/images/{model}/{kind}[/{version}]` | `images_web::admin_image_detail_handler` |
| `GET/POST /admin/images/{model}/{kind}[/{version}]/edit` | `images_web::admin_image_edit_page_handler`, `admin_image_update_handler` |
| `GET /admin/images/{model}/{kind}/versions` | `images_web::admin_image_versions_handler` |

`images_api::path_model_kind` reads the two segments for all of them: an unknown model (an old
0.6 name gets `NodeModel::from_str`'s message naming its replacement), an unknown kind, or a
kind the model does not take (`ModelKind::new`) is a 400. The bodies of `import`, `pull` and
`download` and the `upload` multipart form carry an optional `kind`; absent, or empty in the
form, it is the model's default kind, and `generic`, which has none, is refused without one.

`/admin/images` lists one row per model and kind that has an image, showing its default
version. The upload page and the lab create page get each model as a `templates::ModelChoice`
(its kinds and its default kind as data attributes) and every kind from `kind_choices`; their
scripts (`model-filter.js`, `lab-create.js`) offer only the chosen model's kinds, set to its
default. The lab create page shows the kind only for a model with more than one, and writes
`kind` into the manifest only when it is not the model's default, so always for `generic`.

#### Container image reference

A container node runs one image reference from pull to create, `container::image_reference(repo,
tag)`, which is `{repo}:{tag}` with the repo kept as written:

- **Pull** (`container_pull.rs`) pulls `repo:tag` and records `repo` on the version's
  `node_image` row. A tar import records the model's built-in repo (`ModelKind::node_config`).
  An image scan records the repo it matched the tag by: the model's built-in repo, or the
  request's `repo`.
- **Resolve** (`validate::validate_and_resolve_node_versions`) sets the node's `image` to
  `NodeConfig::container_repo`: the manifest's `image`, else the row's repo. Models with a built-in
  repo follow the same rule; the built-in repo is only what their rows start with. A node with
  neither fails, naming both fixes.
- **Check** (`image_lifecycle::check_container_image`) asks Docker for the reference with Bollard
  `inspect_image`. Docker resolves it, so `docker.io/library/nats:2.11`, which Docker tags
  `nats:2.11`, is found. Sherpa does not parse or rewrite references. `up` checks after leasing
  the images, and a redeploy checks in stage 1, before the node is torn down.
- **Create** (`up/readiness.rs`, `redeploy/container.rs`) creates the container from the same
  reference. The container ZTP start settings in `node_ops.rs` set a model's environment, user,
  command and mounts, never its image.

#### Container image scan

`import::scan_images` records local Docker tags the database does not have.
`container_scan::plan_container_scan`, which is pure, decides which repos to ask Docker for:

- With no target, each container model with a built-in repo, and the models with none
  (`generic`, `mysql_db`, `postgresql_db`) come back as `SkippedModel`s, which the
  response lists so the CLI can say why they were not matched.
- With a target, the request's `model` and `repo`, only that repo, as versions of that model,
  and the disk scan is not run. `validate::validate_image_scan` checks the pair first: both or
  neither, a container model, a repo without a tag, and no other kind.

Docker matches each repo to its images: `container::get_local_images` lists images with the
Docker API's `reference` filter, as `docker images <repo>` does. Sherpa does not rewrite or
compare names. Docker matches a Docker Hub image by its short name (`mysql`) or its full name
(`docker.io/library/mysql`), but not by `docker.io/mysql` or `index.docker.io/...`, which find
nothing, and never by a prefix (`nats` does not match `natsio/nats-box` or `nats-streaming`).
Docker returns plain `repo:tag` strings, under its own name for the repo (`mysql:8.4` for
`docker.io/library/mysql`), so `container_scan::tagged_versions` reads the tag after the last
`:`, skips digest entries (`nats@sha256:...`, how the containerd image store lists an untagged
image) and leaves out tags that are not valid image versions. The repo is recorded as given.
Each version is recorded through `scan_version`, with `DefaultRequest::Keep`, so only a model's
first version becomes its default and an existing record, repo included, is left as it is.

### Scanner service architecture

The scanner is the background service `run_server` starts to keep node states in line with
libvirt and Docker. It never creates or destroys resources, and it never writes a state an
operation owns: it observes a lab only while no operation holds it, and moves nodes only
through `node_lifecycle::observe`, which applies `NodeState::on_observation`.

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
            snapshot["query_runtimes: libvirt (spawn_blocking) and Docker, at once"]
            listlabs["db::list_labs"]
            subgraph eachlab["scan_lab, for each lab"]
                listnodes["db::list_nodes_by_lab, node images for kind and SSH"]
                wantsprobe{"wants_probe: a VM with SSH that the snapshot shows running,<br/>not yet Running or with no host key"}
                probe["probe_node: check_vm_ready on port 22,<br/>record_node_host_key"]
                hold{"lab_operations::try_observe:<br/>does an operation hold the lab?"}
                fresh["under the hold: query_runtimes again,<br/>read the nodes again"]
                observe["node_lifecycle::observe each node:<br/>runtime + probe result, compare-and-set"]
            end
        end
    end
    spawn --> wait
    wait -->|cancellation token triggered| stop
    wait -->|interval elapsed| snapshot
    snapshot --> listlabs --> listnodes --> wantsprobe
    wantsprobe -->|yes| probe --> hold
    wantsprobe -->|no| hold
    hold -->|no| fresh --> observe
    hold -->|yes: skip this cycle| wait
    observe -->|next lab, next cycle| wait
    scancycle -.->|"scan failure: log, do not kill server"| wait
```

What the scanner may do to each state is in
[`STATE_MACHINES.md`](STATE_MACHINES.md#1-node-lifecycle-nodestate). In short: a node gone
from libvirt or Docker is `Missing`, a crashed domain or dead container `Failed`, a stopped,
paused or exited one `Stopped`; a running one is `Running` once ready. `Building` and `Failed`
nodes are left to the operations.

The probes run before the hold, so a slow node does not delay an operation that wants
the lab. The runtimes are queried again under the hold, so what an operation did just before
it released the lab is what the scanner sees. Each write is a compare-and-set on the state
the scanner read, so it never overwrites a newer write.

A runtime that cannot be queried in a cycle (libvirtd or Docker restarting) is unknown for that
cycle: its nodes keep their recorded state. Treating it as empty would mark every node
`Missing`.

Each node is probed on the readiness check stored with it (`DbNode::ready_check`, see
[Node readiness](#node-readiness)), never the lab's definition. A running node with a stored
port and a check that is not skipped is probed on that port while it is not yet `Running`, as
after a readiness wait that timed out or a start outside Sherpa: a virtual machine on its
model's port or its `ready_check.port`, a container or unikernel on its `ready_check.port`. A
probe that answers lets the node be recorded `Running`; one that does not leaves it
`Starting`, so a container whose port never opens stays `Starting` scan after scan. A
running node whose stored port is SSH and has no host key is probed too, skipped or not, and
the key it offers is recorded; on any other port no key is read. A `Running` node with a key,
or on a port that is not SSH, is not probed again, so the scanner makes no connections to
settled nodes; a key that changes without a state change, as on a reboot from inside the
guest, is read when a client finds it does not match. A node with no stored port, or a
skipped check, has no probe: running counts as ready.

Still under the hold, in a `Ready` lab, the scanner first turns transmit checksum offload off
on the management veth of each running IOL container where Docker made it again with offload
on, as after a `docker start` outside Sherpa (`repair_management_checksums`). Offload is read
from the kernel first, so a veth that is already right is not changed. This runs whether or not
the lab has links. See
[`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md#transmit-checksum-offload).

Then it checks the lab's P2p links in the kernel and repairs the ends of running nodes that were made again outside Sherpa (`repair_links`): a VM
started with `virsh`, a container started or restarted with `docker`. Then it makes the carrier
of each link's ends follow the link's state (`p2p_links::follow_carrier`): the surviving end of
a link whose peer is stopped or paused loses carrier, both ends of a failed link do, and an
attached link's ends get it back. Carrier is set only where a link's state changed since the
states it last recorded, or where an end was made again, so an end a user took down by hand
stays down. It keeps each link's state in `AppState::link_states`, which `inspect` reports and
the next scan compares with. The check, the repair and the carrier are in
[`STATE_MACHINES.md`, section 25](STATE_MACHINES.md#25-p2p-link-attachment) and
[`P2P-ARCHITECTURE.md`](P2P-ARCHITECTURE.md#repair-by-the-scanner). With `scanner.enabled =
false` nothing repairs them or follows a stop with carrier, and `inspect` shows links as not
checked until a resume or redeploy re-attaches them, which sets their carrier too.

## HTTP routing architecture

`api/router.rs` builds a single Axum router. It attaches HTTP tracing, embedded static asset handling, and the route table. It has no CORS layer; see `API.md`. The WebSocket route is added in `daemon/server.rs` after `build_router()` returns.

`build_router()` merges one router per area (`auth_page_routes`, `user_page_routes`, `admin_page_routes`, `public_api_routes`, `api_routes`), then adds the trace layer and, after it, the static asset route:

| Group | Routes |
|---|---|
| Public browser routes | `GET/POST /login`, `GET/POST /signup` (404 unless `allow_signup`), `POST /logout` |
| Protected browser routes | `GET /`; `GET /labs`, `/labs/grid`, `/labs/create`; `GET /jobs/{job_id}`, `/jobs/{job_id}/stream`; `GET /labs/{lab_id}`; `GET /labs/{lab_id}/nodes[/node]`; `POST` lab/node start/stop/redeploy/destroy actions; `GET/POST` profile, device-password and SSH-key routes |
| Admin browser routes | `/admin/users...`, `/admin/labs`, `/admin/tools...`, `/admin/images...` |
| Public API routes | `GET /health`, `GET /cert`, `GET /api/v1/spec`, `GET /api/v1/openapi.json`, `GET /api/docs`, `POST /api/v1/auth/login` |
| REST API routes | labs: list/create/inspect/delete/down/resume/redeploy; links: impairment update; images: list/show/import/upload/delete/default/pull/download; admin tools: clean/scan; users: create/list/info/delete/password/device-password |
| Layers | `TraceLayer::new_for_http()` |
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
        db["3. DatabaseRecords<br/>lab, nodes, per-node libvirt networks, the lab's definition"]
        labnet["4. LabNetworkSetup<br/>lab dir and SSH key, management networks, Tailscale"]
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

Each node's readiness check is stored on its record (`node.ready_port` and
`node.skip_ready_check`, read as `DbNode::ready_check`). `up` writes it when it creates the
node's record, and `redeploy` replaces it from the new manifest node before the node starts,
both with `node_ops::stored_ready_check`: the manifest's `ready_check.port`, else the node's
model and kind's port (`ModelKind::ready_port`: SSH, or the Talos API for Talos Linux, for a
VM); a container or unikernel without a `ready_check.port` has none. `resume`, `redeploy`'s wait, `inspect` and
the scanner read the stored check, not the manifest, so every readiness path agrees;
`node_ops::probed_ready_port` gives the port it is probed on, none when it is skipped.

`up`, `redeploy` and `resume` poll each VM's ready port with `node_ops::check_vm_ready`.
Unikernels use `check_unikernel_ready`. A container is ready once it keeps running
(`CONTAINER_SETTLE` in `up`); one with a ready port (`node_ops::container_ready_port` in
`up`, none when the check is skipped) must then also answer on it, probed with
`check_vm_ready` on its management IPv4 in all three. One that does not answer in time stays
`Starting` with the non-critical "did not become ready" error a VM gets, and the scanner
records it `Running` only once its stored port answers.

A container that is not running right after it starts, or that exits while it settles or waits
for its port, is reported as a failed node, a critical `NodeReadiness` error, and is not
started again (that would fail on its name and roll back the whole lab). Once readiness ends,
`up` checks every started container again, so one that exited since is reported too and its
node marked `Failed`. VM and unikernel creation
tasks are all awaited before a failure is reported, so none is still creating a domain while
the rollback removes the lab.

```mermaid
flowchart TB
    check["check_vm_ready(ip, port, sshd_marker)"] --> port{"port"}
    port -->|"22, or the stored check's sshd_marker"| ssh["check_node_ssh_ready"]
    ssh --> tcp["TCP connect (100 ms)"]
    tcp -->|"refused/timeout"| notready["not ready"]
    tcp -->|connected| hostkey["Jato Session::host_key<br/>(5 s, no login)"]
    hostkey -->|"non-SSH reply or timeout"| notready
    hostkey -->|"SSH handshake through key exchange, then disconnect"| marker{"sshd_marker?"}
    marker -->|false| ready["ready, with host key"]
    marker -->|"Windows, cloud-init"| ident["ssh_identification:<br/>read the SSH-2.0 line (5 s)"]
    ident -->|"shows the marker"| ready
    ident -->|"no marker yet"| notready
    port -->|other port| portready["check_node_port_ready:<br/>TCP connect (100 ms)"]
```

A Windows node's OpenSSH server starts at boot, before cloudbase-init has created the owner's
account, written their keys file and Sherpa's `sshd_config`, and restarted it, so a handshake
alone reported Windows nodes ready minutes before the owner could log in. Sherpa's Windows
`sshd_config` sets `VersionAddendum sherpa-ready` (`SHERPA_SSHD_READY_MARKER`), which the server
adds to the identification line it sends on every connection, before key exchange.
Whether a node waits for it is decided in one place, `ModelKind::waits_for_sshd_ready_marker`:
`windows_server` and `devbox_windows` with cloud-init ZTP (with ZTP off Sherpa writes no
`sshd_config`). `node_ops::stored_ready_check` stores the answer with the node's readiness check
(`ReadyCheck::sshd_marker`, `node.ready_sshd_marker`) when the check is on SSH, and every probe
passes the stored flag to `check_vm_ready`: `up` (the same `stored_ready_check` it stores),
`redeploy`, `resume`, the scanner and `node.host_keys`. With it the probe also reads the
identification line over a plain TCP connection (`ssh_identification`) and is ready only once it
shows the marker. cloudbase-init handles `write_files` and `users` before `runcmd`, and the `sshd`
restart is in `runcmd` after the key file's ACLs, so the marker appears only once the owner can
log in by key. No credentials are sent. So a Windows node whose `up` stopped waiting early, or
timed out, stays `Starting` until the scanner sees the marker. `node.host_keys` on a Windows node
that does not show it yet keeps the recorded key and reports no change: until then the SSH server
runs the image's config, not the one the owner logs in with, and the readiness probe records the
key once the node is ready. Nodes built before 0.7.0 stored no flag
(`0032_node_ready_sshd_marker.sql` leaves them `0`): their `sshd` never shows the marker.

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
- The scanner records the key of a running node whose stored ready port is SSH and that is
  not yet known ready, such as one whose readiness wait timed out or that started outside
  Sherpa, or has no key (see the scanner section).
- The `node.host_keys` RPC (`services::host_keys`) reads one node's key on request, for a
  client whose recorded key did not match.

`ssh_config::record_node_host_key` writes a key only when it differs, and logs a warning when
it replaces another. The server keeps no SSH files: when a client is given them, by `up`'s
response, `download` or `node.host_keys`, `ssh_config::lab_ssh_files` renders them in memory
from the database (nodes' addresses, whether their image has SSH, their keys, and the account
each logs in with, `node.ssh_username`, through `lab_info::load_lab_info`). They are
`sherpa_ssh_config`, in which a node with a recorded key gets
`StrictHostKeyChecking yes` and `UserKnownHostsFile sherpa_known_hosts` (others keep `no` and
`/dev/null`), and `sherpa_known_hosts` (template `ssh/known_hosts.jinja`), one line per key under
the name OpenSSH looks up: the address on port 22, else `[address]:port`. `up`'s response, the
download service and `node.host_keys` carry the known_hosts file, and the client saves it as
`.sherpa/ssh/known_hosts` and rewrites the relative `UserKnownHostsFile` path to that absolute
one (`client::lab_files`, see [`LAB_FOLDER.md`](LAB_FOLDER.md)). Each node's `User` is its recorded login, else the lab owner's name, and the config names no `IdentityFile`: the client's own keys and agent are used. When a node's key is unknown or changed, the client fetches the lab's
keys with `node.host_keys`, warns about a changed key, and connects again (`SSH_CLIENT.md`).

### No lab SSH key

Sherpa keeps no SSH key of its own for nodes: every node with a generated login has the lab
owner's account with the owner's registered keys (see
[`GUEST_PROVISIONING.md`](GUEST_PROVISIONING.md#guest-login)), and Sherpa itself never logs in to
a node. 0.7.0 removed the per-lab key pair:

- `up` makes no key, and `up`'s response and `download` carry no `ssh_private_key`; the client
  writes no `.sherpa/ssh/key` (see [`LAB_FOLDER.md`](LAB_FOLDER.md)).
- The MikroTik config disk carries only the owner's key files, no `sherpa_ssh_key.pub`.
- Nothing about a key is in the database: `lab.ssh_key` went in migration 17.
- **Labs from before 0.7.0** keep `{lab_dir}/sherpa_ssh_key` and `.pub` in their folder, and
  their nodes keep trusting it for `sherpa`, until each node is redeployed or the lab destroyed.
  `redeploy` no longer reads it, and `destroy` and `clean` delete it with the lab folder
  (`cleanup_lab_metadata`).
- **No server key.** Releases before 0.6.0 had a shared key pair in `/opt/sherpa/ssh/`, made by
  `sherpad init`, which labs created before lab keys kept (`lab.ssh_key = 'server'`). 0.6.0
  removed it: `sherpad` and the update script refuse to take over a database that still has
  such labs, naming them ([`DATABASE.md`](DATABASE.md#labs-on-the-servers-ssh-key)), and the
  update script deletes `/opt/sherpa/ssh/sherpa_ssh_key*` once the new release has started.

### Runtime resource model for a lab

A running lab is spread across several systems. The DB is the desired/known state, not the only state.

```mermaid
flowchart LR
    lab["Lab {lab_id}"]
    database["Database<br/>user record<br/>lab record<br/>node records<br/>link records<br/>image records"]
    ram["RAM: /run/sherpa/secrets/{lab_id}/{node}<br/>(container secret files)"]
    fs["Filesystem: /opt/sherpa/labs/{lab_id}<br/>lab info file<br/>generated node configs<br/>ZTP/TFTP assets<br/>cert material where needed<br/>SSH/helper config files"]
    libvirt["libvirt/QEMU<br/>VM domains<br/>unikernel domains<br/>NAT/isolated/reserved networks<br/>cloned/resized disks in Sherpa storage pool"]
    docker["Docker<br/>containers<br/>bridge/macvlan networks<br/>pulled/imported container images"]
    host["Linux host networking<br/>bridges<br/>veth pairs<br/>tap devices<br/>impairment/eBPF/netns wiring where applicable"]
    lab --> database
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

- `planned_resources`: VMs and unikernels get the manifest's `cpu.count` and `memory`, else the
  image's. Containers get the manifest's, which become their Docker limits, and nothing for
  what it leaves out; a `juniper_evolved` container gets its EVO VM's memory plus its container overhead,
  and no CPU count, and `cisco_iol` and `cisco_ioll2` get IOL's memory plus the launcher's
  overhead (see `NODE_SETTINGS.md`). `up` (`readiness.rs`) and redeploy
  (`redeploy/container.rs`) create the container with the stored values, so Docker and the
  database agree; resume starts the existing container, which keeps its limits.
- `reserve_node_resources` (`up`, right after each node record is created, and `redeploy`): a
  VM also gets a VNC port, the lowest in `node_resources::VNC_PORT_FIRST..=VNC_PORT_LAST` that no node
  holds and nothing on the server listens on. `unique_node_vnc_port` settles a race with a
  concurrent lab, which is retried, as lab subnets are. A redeployed VM keeps its port.
- `node_ops::build_domain_template` takes the stored values, so the domain and the database
  agree. The domain has `port='N' autoport='no'`, so libvirt never moves the console, also
  across down/resume.
- `inspect` and the web UI node page read them from the record, with no domain lookup.

### VM performance tuning

A VM node's tuning in its `cpu`, `numa`, `boot_disk` and `interfaces` groups
(`shared::data::NodeCpu`, `NodeNuma`, `NodeBootDisk`, `NodeInterfaces`; see
[`NODE_SETTINGS.md`](NODE_SETTINGS.md)) reaches the domain as `template::DomainTuning`, built by
`DomainTuning::from_node_settings` in `node_ops::build_domain_template`. The domain then renders:
- `<cputune>` with `vcpupin`, `emulatorpin` and `iothreadpin`;
- `<numatune>`, with `<memory mode>` from `numa.memory_mode` (`strict` by default, or
  `preferred`);
- `<iothreads>`;
- the CPU mode and `<topology>`;
- the disk driver's `cache`, `io` and `iothread`;
- `<driver queues>` on virtio interfaces.

The checks run in three places:
- **`topology::validate::check_vm_tuning`** (client `validate` and server `up` and
  `redeploy`): the node's own settings against its image. `check_node_kind_settings` refuses
  these settings on other kinds first; `cpu.topology` and `cpu.pinning` match `cpu.count`; I/O
  threads need virtio disks; `interfaces.queues` needs virtio interfaces; `numa.memory_mode`
  needs `numa.node`.
- **`topology::validate::check_cpu_pin_overlap`**: a host CPU pinned to a vCPU belongs to one
  node, and no node runs QEMU or I/O threads on it. QEMU and I/O threads may share CPUs.
- **`node_resources::check_cpu_pins_on_server`** (`up` and `redeploy`, under admission, before
  anything is created or destroyed): every CPU and the NUMA node exist on the server, from
  `libvirt::host_cpus`, and `check_cpu_pins_free` finds the pins free of every other node's
  in the `cpu_pin` table (see [Admission](#admission-memory-and-cpu-pins)).

Admission then claims the pins in the `cpu_pin` table, so later labs see them. Two labs started
at the same moment cannot both pass: admission serves one request at a time, and the second
sees the first's rows. The table's trigger enforces the same rules for any write (see
[`DATABASE.md`](DATABASE.md#cpu-pins)).

Huge pages come from the host's transparent hugepages, which QEMU requests for guest RAM.
There is no `memoryBacking` setting. `scripts/sherpa_install.sh` sets THP to `madvise` unless it
is `always`, now and on boot through `/etc/tmpfiles.d/sherpa-thp.conf`; the uninstall script
removes that rule.

### VM hardware watchdog

A VM has an i6300esb watchdog only when its node sets `vm.watchdog` (`shared::data::WatchdogAction`,
libvirt's action names). `node_ops::build_domain_template` copies it to
`DomainTemplate::watchdog`, and the domain renders `<watchdog model='i6300esb' action='...'/>`
for it. `topology::validate::check_node_kind_settings` (client `validate`, server `up` and
`redeploy`) refuses the `vm` group on containers and unikernels. Nothing is stored: the domain keeps it across down and
resume, and `redeploy` reads it from the manifest again.

libvirt (10.0 on Ubuntu 24.04) also lists the q35 chipset's iTCO watchdog on every q35 domain,
`<watchdog model='itco'>`, with the same action, which QEMU applies to all watchdogs: `reset`
without `watchdog`. Sherpa does not add or remove it; it acts only if the guest arms it.

VMs once all had a `reset` watchdog. Cisco IOS arms it at boot and feeds it only after an init
phase that, under nested KVM, outlasts the timeout, so those VMs reset in a loop; it was removed
from the default domain for that reason.

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

`run_container` takes a `container::ContainerSpec`. A container it creates but cannot start or
attach is removed again, so a retried `up` or `redeploy` can create it under the same name. An
active, renamed data interface is set up with a script that gets the interface name as `$1`,
never in the script text.

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
blocks, defined in `crates/shared/src/konst.rs` and applied to both address families. Nothing
in production reads the two reserved blocks as ranges, so `MANAGEMENT_INFRA_HOST_OFFSETS` and
`MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS` live in that file's test module, whose tests check that
the blocks are disjoint and hold the offsets placed in them:

| Offsets | Block | Users |
|---|---|---|
| `1`–`10` | Sherpa infrastructure (`MANAGEMENT_INFRA_HOST_OFFSETS`) | `.1` gateway and DNS, `.2` router (dnsmasq, boot server), `.3` Tailscale gateway |
| `11`–`20` | Lab control planes (`MANAGEMENT_CONTROL_PLANE_HOST_OFFSETS`) | `.11`–`.12` Talos cluster VIPs (`TALOS_VIP_HOST_OFFSETS`); `.13`–`.20` Cilium cluster load balancer addresses, four per cluster (`TALOS_LOAD_BALANCER_HOST_OFFSETS`) |
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
        records["Step 6: database lab/link/node records"]
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
| REST/HTML handlers | `crates/server/src/api/handlers/` |
| REST auth extractors | `crates/server/src/api/extractors.rs` |
| SSE conversion | `crates/server/src/api/sse.rs` |
| WebSocket lifecycle | `crates/server/src/api/websocket/handler.rs`, `connection.rs`, `messages.rs` |
| RPC dispatch | `crates/server/src/api/websocket/rpc/dispatch.rs` |
| JWT/cookies/auth context | `crates/server/src/auth/` |
| Lab create | `crates/server/src/services/up/` |
| Lab destroy | `crates/server/src/services/destroy.rs` |
| Node/lab stop/start | `crates/server/src/services/down.rs`, `resume.rs` |
| Redeploy | `crates/server/src/services/redeploy/` |
| Inspect/list/download | `crates/server/src/services/inspect.rs`, `list_labs.rs`, `download.rs` |
| Image management | `crates/server/src/services/import.rs`, `container_pull.rs`, `delete.rs` |
| Link impairment | `crates/server/src/services/impairment.rs` |
| Scanner | `crates/server/src/services/scanner.rs` |
| Node lifecycle | `crates/shared/src/data/node_lifecycle.rs` (transitions), `crates/server/src/services/node_lifecycle.rs` (writes) |
| TLS certificates | `crates/server/src/tls/` |
| Generated API registry | `crates/shared/src/api_spec.rs` |
