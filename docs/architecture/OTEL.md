# OpenTelemetry

Sherpa supports exporting distributed traces via OpenTelemetry (OTLP). This is
useful for visualising request flows, debugging latency, and understanding how
lab lifecycle operations (up, destroy, redeploy, etc.) propagate through the
system.

## Configuration

OTel is always compiled into sherpad but disabled by default. To enable it, add
an `[otel]` section to `sherpa.toml`:

```toml
[otel]
enabled = true
endpoint = "http://localhost:4317"   # OTLP gRPC collector
protocol = "grpc"                    # "grpc" (default) or "http"
service_name = "sherpad"             # reported to the collector
sample_ratio = 1.0                   # 0.0–1.0 (1.0 = sample everything)
```

All fields are optional — omitted fields use the defaults shown above. The
entire `[otel]` section can be left out and the server will start without
tracing export.

### Environment variable overrides

Standard OTel environment variables take precedence over `sherpa.toml`:

| Variable                        | Overrides          |
|---------------------------------|--------------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT`  | `endpoint`         |
| `OTEL_SERVICE_NAME`             | `service_name`     |

## Running a local collector

The quickest way to see traces is with Jaeger all-in-one:

```bash
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  jaegertracing/all-in-one:latest
```

- **4317** — OTLP gRPC receiver (where sherpad sends traces)
- **16686** — Jaeger UI

Start sherpad with OTel enabled, make some API or WebSocket calls, then open
`http://localhost:16686` and select the `sherpad` service.

## What gets traced

- **HTTP requests** — every request through the Axum router gets a span with
  method, path, and status code (via `tower-http` `TraceLayer`).
- **WebSocket connections** — each connection gets a `ws_connection` span
  tagged with `connection_id`.
- **RPC calls** — each WebSocket RPC method dispatch gets an `rpc_call` span
  tagged with `rpc.method`, `rpc.id`, and `connection_id`.
- **Service operations** — top-level service functions (`up_lab`, `destroy_lab`,
  `shutdown_lab_nodes`, `start_lab_nodes`, `clean_lab`, `inspect_lab`,
  `redeploy_node`, `import_image`, `pull_container_image`, etc.) each create
  an `#[instrument]` span with relevant fields like `lab_id` or `node_name`.
- **Database calls** — every public DB function gets a debug-level span with
  `#[instrument(skip(db), level = "debug")]`.
- **Docker operations** — container create/start/stop/exec/network functions
  get debug-level spans.
- **libvirt operations** — VM clone/create/resize/delete and network functions
  get debug-level spans.
- **Existing tracing calls** — all 500+ `tracing::info!` / `error!` / `debug!`
  calls throughout the codebase automatically appear as span events under the
  OTel layer.

## Metrics

When OTel is enabled, the following metrics are exported via OTLP alongside
traces (same endpoint):

| Metric name                  | Type          | Description                              | Attributes          |
|------------------------------|---------------|------------------------------------------|----------------------|
| `sherpad.ws.connections`     | UpDownCounter | Active WebSocket connections             | —                    |
| `sherpad.rpc.duration`       | Histogram (s) | RPC call duration                        | `rpc.method`         |
| `sherpad.operation.duration` | Histogram (s) | Service operation duration               | `operation.type`     |
| `sherpad.errors`             | Counter       | Error count by operation type            | `operation.type`     |

Metrics are exported every 60 seconds by default. When OTel is disabled, all
metric instruments are no-ops with zero overhead.

## Graceful shutdown

The tracer and meter providers are held in an `OtelGuard` that flushes pending
spans and metrics when the server shuts down. No manual cleanup is required.
