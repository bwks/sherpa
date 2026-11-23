use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode, Uri};
use axum::{Json, Router};
use bollard::Docker;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    Gateway,
    Volume,
    Logout,
}

pub(crate) struct DockerMock {
    pub(crate) docker: Docker,
    pub(crate) requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl DockerMock {
    pub(crate) async fn new(containers: Value, fault: Option<Fault>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let router = Router::new().fallback(move |method: Method, uri: Uri| {
            let captured = Arc::clone(&captured);
            let containers = containers.clone();
            async move {
                captured.lock().unwrap().push(format!("{method} {uri}"));
                let path = uri.path();
                let (status, body) = if path.ends_with("/containers/json") {
                    (StatusCode::OK, containers)
                } else if method == Method::GET
                    && (path.ends_with("/containers/sherpa-tailnet-test1234/json")
                        || path.ends_with("/containers/gateway-id/json"))
                    && matches!(fault, Some(Fault::Gateway | Fault::Logout))
                {
                    (
                        StatusCode::OK,
                        json!({"Id":"gateway-id",
                            "State":{"Running":fault == Some(Fault::Logout)},
                            "Config":{"Labels":{
                            "sh.erpa.tailscale.lab":"test1234",
                            "sh.erpa.role":"tailnet-gateway"
                        }}}),
                    )
                } else if method == Method::GET
                    && path.ends_with("/volumes/sherpa-tailnet-state-test1234")
                    && matches!(fault, Some(Fault::Volume | Fault::Logout))
                {
                    (
                        StatusCode::OK,
                        json!({"Name":"sherpa-tailnet-state-test1234",
                            "Driver":"local","Mountpoint":"/mock/volume",
                            "Options":{},"Scope":"local","Labels":{
                            "sh.erpa.tailscale.lab":"test1234",
                            "sh.erpa.role":"tailnet-gateway"
                        }}),
                    )
                } else if method == Method::DELETE
                    && ((path.ends_with("/containers/gateway-id") && fault == Some(Fault::Gateway))
                        || (path.ends_with("/volumes/sherpa-tailnet-state-test1234")
                            && fault == Some(Fault::Volume)))
                {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        json!({"message":"cleanup failed"}),
                    )
                } else if method == Method::DELETE
                    || path.ends_with("/kill")
                    || path.ends_with("/stop")
                {
                    (StatusCode::NO_CONTENT, Value::Null)
                } else {
                    (StatusCode::NOT_FOUND, json!({"message":"missing"}))
                };
                (status, Json(body))
            }
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let docker = Docker::connect_with_http(
            &format!("http://{address}"),
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .unwrap();
        Self {
            docker,
            requests,
            task,
        }
    }
}

impl Drop for DockerMock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
