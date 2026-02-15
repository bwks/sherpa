pub mod handlers;
pub mod websocket;

use crate::daemon::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};
use handlers::{lab_destroy, lab_inspect, lab_up};

/// Build the Axum router with all API routes
pub fn build_router() -> Router<AppState> {
    Router::new()
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        .route("/inspect/{id}", get(lab_inspect))
}
