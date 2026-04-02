use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use shared::konst::{
    OTEL_METRIC_ERROR_COUNT, OTEL_METRIC_OPERATION_DURATION, OTEL_METRIC_RPC_DURATION,
    OTEL_METRIC_WS_CONNECTIONS,
};

/// Holds all OpenTelemetry metric instruments.
///
/// All instruments are internally reference-counted, so this struct is cheap
/// to clone and safe to share across tasks.  When OTel is disabled a noop
/// meter is used — every recording call becomes a no-op with zero overhead.
#[derive(Clone)]
pub struct Metrics {
    /// Active WebSocket connection count (increment on connect, decrement on disconnect).
    pub ws_connections: UpDownCounter<i64>,
    /// RPC call duration in seconds, keyed by `rpc.method`.
    pub rpc_duration: Histogram<f64>,
    /// Service-level operation duration in seconds, keyed by `operation.type`.
    pub operation_duration: Histogram<f64>,
    /// Error counter, keyed by `operation.type` and `error.type`.
    pub error_count: Counter<u64>,
}

impl Metrics {
    /// Create instruments from an active meter (used when OTel is enabled).
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

    /// Create noop instruments (used when OTel is disabled).
    pub fn noop() -> Self {
        let meter = opentelemetry::global::meter("noop");
        Self::new(&meter)
    }
}
