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
- **Existing tracing calls** — all 500+ `tracing::info!` / `error!` / `debug!`
  calls throughout the codebase automatically appear as span events under the
  OTel layer.

## Graceful shutdown

The tracer provider is held in an `OtelGuard` that flushes pending spans when
the server shuts down. No manual cleanup is required.
