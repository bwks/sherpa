mod inspect;

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

#[tokio::main]
async fn main() {
    // initialize tracing
    tracing_subscriber::fmt::init();

    // build our application with a route
    let app = Router::new()
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        .route("/inspect/{id}", get(lab_inspect));

    // run our app with hyper
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}
async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}
async fn lab_inspect(id: Path<String>) -> String {
    format!("Inspecting Lab {}", id.0)
}

// the input to our `create_user` handler
#[derive(Deserialize)]
struct CreateUser {
    username: String,
}

#[derive(Deserialize)]
struct LabId {
    id: String,
}

// the output to our `create_user` handler
#[derive(Serialize)]
struct User {
    id: u64,
    username: String,
}
