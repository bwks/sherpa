use axum::{
    Json, Router,
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
        // `GET /` goes to `root`
        .route("/", get(root))
        .route("/up", post(lab_up))
        .route("/destroy", post(lab_destroy))
        .route("/inspect", post(lab_inspect))
        // `POST /users` goes to `create_user`
        .route("/users", post(create_user));

    // run our app with hyper
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

// basic handler that responds with a static string
async fn lab_up(Json(payload): Json<LabId>) -> String {
    format!("Creating Lab {}", payload.id)
}
async fn lab_destroy(Json(payload): Json<LabId>) -> String {
    format!("Destroying Lab {}", payload.id)
}
async fn lab_inspect(Json(payload): Json<LabId>) -> String {
    format!("Inspecting Lab {}", payload.id)
}

async fn create_user(
    // this argument tells axum to parse the request body
    // as JSON into a `CreateUser` type
    Json(payload): Json<CreateUser>,
) -> impl IntoResponse {
    // insert your application logic here
    let user = User {
        id: 1337,
        username: payload.username,
    };

    // this will be converted into a JSON response
    // with a status code of `201 Created`
    (StatusCode::CREATED, Json(user))
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
