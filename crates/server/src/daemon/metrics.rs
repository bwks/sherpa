use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};

const OTEL_METRIC_WS_CONNECTIONS: &str = "sherpad.ws.connections";
const OTEL_METRIC_RPC_DURATION: &str = "sherpad.rpc.duration";
const OTEL_METRIC_OPERATION_DURATION: &str = "sherpad.operation.duration";
const OTEL_METRIC_ERROR_COUNT: &str = "sherpad.errors";

/// Holds all OpenTelemetry metric instruments.
///
/// All instruments are internally reference-counted, so this struct is cheap
/// to clone and safe to share across tasks.  When `OTel` is disabled a noop
/// meter is used — every recording call becomes a no-op with zero overhead.
#[derive(Clone)]
pub struct Metrics {
    pub ws_connections: UpDownCounter<i64>,
    pub rpc_duration: Histogram<f64>,
    pub operation_duration: Histogram<f64>,
    pub error_count: Counter<u64>,
}

impl Metrics {
    /// Create instruments from an active meter (used when `OTel` is enabled).
    #[must_use]
    pub fn new(meter: &Meter) -> Self {
        let ws_connections = meter
            .i64_up_down_counter(OTEL_METRIC_WS_CONNECTIONS)
            .with_description("Active WebSocket connections")
            .with_unit("connections")
            .build();

        let rpc_duration = meter
            .f64_histogram(OTEL_METRIC_RPC_DURATION)
            .with_description("RPC call duration")
            .with_unit("s")
            .build();

        let operation_duration = meter
            .f64_histogram(OTEL_METRIC_OPERATION_DURATION)
            .with_description("Service operation duration")
            .with_unit("s")
            .build();

        let error_count = meter
            .u64_counter(OTEL_METRIC_ERROR_COUNT)
            .with_description("Error count by operation and error type")
            .build();

        Self {
            ws_connections,
            rpc_duration,
            operation_duration,
            error_count,
        }
    }

    /// Create noop instruments (used when `OTel` is disabled).
    #[must_use]
    pub fn noop() -> Self {
        let meter = global::meter("noop");
        Self::new(&meter)
    }
}
