pub mod handlers;

use axum::{routing::{get, post}, Router};
use handlers::{lab_up, lab_destroy, lab_inspect};

/// Build the Axum router with all API routes
pub fn build_router() -> Router {
    Router::new()
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        .route("/inspect/{id}", get(lab_inspect))
}
