//! Regression tests for the server's security boundaries: what an authenticated or anonymous
//! caller must not be able to do, on every transport.

use anyhow::{Context, Result, ensure};
use db::DefaultRequest;
use reqwest::{Client, Method, Response, StatusCode, header};
use serde_json::{Value, json};
use shared::data::{NodeConfig, NodeModel};
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use std::time::Duration;

use crate::helpers::test_server::{
    TEST_ADMIN_PASSWORD, TEST_SSH_PUBLIC_KEY, TestServer, test_config,
};
use crate::helpers::ws_client::TestWsClient;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
/// Version of the image the access matrix seeds, so the image pages have one to show.
const MATRIX_IMAGE_VERSION: &str = "matrix-1";
/// Admin web pages that only read. An admin is served each; anyone else is refused.
const ADMIN_PAGE_READS: &[&str] = &[
    "/admin/users",
    "/admin/labs",
    "/admin/tools",
    "/admin/images",
    "/admin/images/upload",
    "/admin/images/ubuntu_linux",
    "/admin/images/ubuntu_linux/edit",
    "/admin/images/ubuntu_linux/versions",
    "/admin/images/ubuntu_linux/matrix-1",
    "/admin/images/ubuntu_linux/matrix-1/edit",
];
/// Admin web actions. Only their refusals are checked: done by an admin, they act on the host.
const ADMIN_PAGE_ACTIONS: &[&str] = &[
    "/admin/tools/clean/c0ffee99",
    "/admin/tools/scan",
    "/admin/images/upload",
    "/admin/images/ubuntu_linux/edit",
    "/admin/images/ubuntu_linux/matrix-1/edit",
];
/// Admin-only REST operations, each with a body its handler parses, so that a refusal comes
/// from the admin check rather than from the body. Only refusals are checked. The bodies name
/// nothing that exists, in case a refusal ever fails.
const ADMIN_API_ACTIONS: &[ApiAction] = &[
    ApiAction::json("POST", "/api/v1/admin/tools/labs/clean/c0ffee99", "{}"),
    ApiAction::json(
        "POST",
        "/api/v1/admin/tools/images/scan",
        r#"{"dry_run":true}"#,
    ),
    ApiAction::json(
        "POST",
        "/api/v1/images/import",
        r#"{"model":"ubuntu_linux","version":"none","src":"/nonexistent/disk.qcow2"}"#,
    ),
    ApiAction {
        method: "POST",
        path: "/api/v1/images/upload",
        content_type: "multipart/form-data; boundary=matrix",
        body: "",
    },
    ApiAction::json(
        "POST",
        "/api/v1/images/pull",
        r#"{"model":"generic_container","repo":"sherpa-test/does-not-exist","tag":"none"}"#,
    ),
    ApiAction::json(
        "POST",
        "/api/v1/images/download",
        r#"{"model":"ubuntu_linux","version":"none","url":"http://127.0.0.1:9/none"}"#,
    ),
    ApiAction::json("DELETE", "/api/v1/images/ubuntu_linux/matrix-1", ""),
    ApiAction::json("POST", "/api/v1/images/ubuntu_linux/matrix-1/default", ""),
];
/// Image reads any logged-in user may make.
const IMAGE_API_READS: &[&str] = &["/api/v1/images", "/api/v1/images/ubuntu_linux"];

/// One REST request of the access matrix.
struct ApiAction {
    method: &'static str,
    path: &'static str,
    content_type: &'static str,
    body: &'static str,
}

impl ApiAction {
    const fn json(method: &'static str, path: &'static str, body: &'static str) -> Self {
        Self {
            method,
            path,
            content_type: "application/json",
            body,
        }
    }
}
const NEW_PASSWORD: &str = "Changed-Pass-456!";
const FOREIGN_ORIGIN: &str = "https://attacker.example";

fn url(server: &TestServer, path: &str) -> String {
    format!("http://{}{path}", server.addr)
}

/// Create a user and log them in over RPC.
async fn user_token(
    server: &TestServer,
    ws: &mut TestWsClient,
    username: &str,
    is_admin: bool,
) -> Result<String> {
    db::create_user(
        &server.db,
        username.to_owned(),
        TEST_ADMIN_PASSWORD,
        is_admin,
        vec![TEST_SSH_PUBLIC_KEY.to_owned()],
    )
    .await?;
    ws.login(username, TEST_ADMIN_PASSWORD).await
}

/// Whether `username` can log in with `password`.
async fn can_log_in(ws: &mut TestWsClient, username: &str, password: &str) -> Result<bool> {
    let response = ws
        .rpc_call(
            "auth.login",
            json!({ "username": username, "password": password }),
        )
        .await?;
    Ok(response.get("error").is_none())
}

/// Seed a default Ubuntu image record, for the image pages of the access matrix.
async fn seed_matrix_image(server: &TestServer) -> Result<()> {
    let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
    MATRIX_IMAGE_VERSION.clone_into(&mut image.version);
    image.default = true;
    db::upsert_node_image(&server.db, image, DefaultRequest::Keep).await?;
    Ok(())
}

/// Send a web request, with the session cookie of `token` when there is one.
async fn web_request(
    http: &Client,
    server: &TestServer,
    method: Method,
    path: &str,
    token: Option<&str>,
) -> Result<Response> {
    let mut request = http.request(method, url(server, path));
    if let Some(token) = token {
        request = request.header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"));
    }
    Ok(request.send().await?)
}

/// A client that does not follow redirects, so a login redirect is seen as one.
fn no_redirect_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

/// Assert an admin web route sent an anonymous caller to log in.
fn ensure_login_redirect(response: &Response, label: &str) -> Result<()> {
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    ensure!(
        response.status() == StatusCode::SEE_OTHER && location == "/login?error=session_required",
        "{label}: anonymous caller got {} {location}",
        response.status()
    );
    Ok(())
}

fn error_code(response: &Value) -> &Value {
    &response["error"]["code"]
}

/// Start `up` with a one-container manifest whose node carries `extra` fields.
async fn up_container(ws: &mut TestWsClient, token: &str, extra: Value) -> Result<Value> {
    let mut node = json!({ "name": "box1", "model": "generic_container" });
    for (key, value) in extra.as_object().context("extra must be an object")? {
        node[key] = value.clone();
    }
    let manifest = json!({ "name": "host-access", "nodes": [node] });
    let (_, response) = ws
        .rpc_call_streaming(
            "up",
            json!({ "token": token, "lab_id": "hostacc1", "manifest": manifest }),
            RPC_TIMEOUT,
        )
        .await?;
    Ok(response)
}

// ── Admin clean ──

#[tokio::test]
async fn clean_refuses_a_lab_id_that_is_not_a_lab_id() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    for lab_id in ["", "../../not-a-lab", "lab/../x"] {
        let response = ws
            .rpc_call("clean", json!({ "token": token, "lab_id": lab_id }))
            .await?;
        ensure!(
            *error_code(&response) == json!(RpcErrorCode::InvalidParams),
            "clean '{lab_id}' must be refused before touching anything: {response}"
        );
    }
    Ok(())
}

// ── CORS ──

#[tokio::test]
async fn another_origin_is_never_granted_cross_origin_access() -> Result<()> {
    let server = TestServer::start().await?;
    let client = Client::new();

    let simple = client
        .get(url(&server, "/api/v1/auth/validate"))
        .header(header::ORIGIN, FOREIGN_ORIGIN)
        .send()
        .await?;
    let preflight = client
        .request(Method::OPTIONS, url(&server, "/api/v1/admin/users"))
        .header(header::ORIGIN, FOREIGN_ORIGIN)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .send()
        .await?;

    for response in [simple, preflight] {
        let headers = response.headers();
        ensure!(
            !headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                && !headers.contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
            "a foreign origin was granted access: {headers:?}"
        );
    }
    Ok(())
}

// ── Signup ──

#[tokio::test]
async fn signup_is_closed_unless_the_server_allows_it() -> Result<()> {
    let server = TestServer::start().await?;
    let client = Client::new();

    let page = client.get(url(&server, "/signup")).send().await?;
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    let posted = client
        .post(url(&server, "/signup"))
        .form(&[
            ("username", "mallory"),
            ("password", TEST_ADMIN_PASSWORD),
            ("confirm_password", TEST_ADMIN_PASSWORD),
        ])
        .send()
        .await?;
    assert_eq!(posted.status(), StatusCode::NOT_FOUND);
    assert!(db::find_user(&server.db, "mallory").await?.is_none());

    let login_page = client
        .get(url(&server, "/login"))
        .send()
        .await?
        .text()
        .await?;
    assert!(
        !login_page.contains("/signup"),
        "the login page links to a closed signup"
    );
    Ok(())
}

#[tokio::test]
async fn signup_opens_when_the_server_allows_it() -> Result<()> {
    let mut config = test_config()?;
    config.allow_signup = true;
    let server = TestServer::start_with_config(config).await?;
    let client = Client::new();

    let page = client.get(url(&server, "/signup")).send().await?;
    assert_eq!(page.status(), StatusCode::OK);
    let login_page = client
        .get(url(&server, "/login"))
        .send()
        .await?
        .text()
        .await?;
    assert!(login_page.contains("/signup"));
    Ok(())
}

// ── Password changes ──

#[tokio::test]
async fn changing_your_own_password_over_rpc_needs_the_current_one() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = user_token(&server, &mut ws, "alice", false).await?;

    for current in [None, Some("Wrong-Pass-789!")] {
        let mut params =
            json!({ "token": token, "username": "alice", "new_password": NEW_PASSWORD });
        if let Some(current) = current {
            params["current_password"] = json!(current);
        }
        let response = ws.rpc_call("user.passwd", params).await?;
        ensure!(
            response.get("error").is_some(),
            "changed without the current password ({current:?}): {response}"
        );
        ensure!(can_log_in(&mut ws, "alice", TEST_ADMIN_PASSWORD).await?);
    }

    let response = ws
        .rpc_call(
            "user.passwd",
            json!({
                "token": token,
                "username": "alice",
                "current_password": TEST_ADMIN_PASSWORD,
                "new_password": NEW_PASSWORD,
            }),
        )
        .await?;
    ensure!(response.get("error").is_none(), "{response}");
    ensure!(can_log_in(&mut ws, "alice", NEW_PASSWORD).await?);
    Ok(())
}

#[tokio::test]
async fn changing_your_own_password_over_rest_needs_the_current_one() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = user_token(&server, &mut ws, "alice", false).await?;
    let client = Client::new();
    let endpoint = url(&server, "/api/v1/admin/users/alice/password");

    let refused = client
        .post(&endpoint)
        .bearer_auth(&token)
        .json(&json!({ "token": token, "username": "alice", "new_password": NEW_PASSWORD }))
        .send()
        .await?;
    ensure!(
        !refused.status().is_success(),
        "REST changed the password without the current one"
    );
    ensure!(can_log_in(&mut ws, "alice", TEST_ADMIN_PASSWORD).await?);

    let accepted = client
        .post(&endpoint)
        .bearer_auth(&token)
        .json(&json!({
            "token": token,
            "username": "alice",
            "current_password": TEST_ADMIN_PASSWORD,
            "new_password": NEW_PASSWORD,
        }))
        .send()
        .await?;
    ensure!(accepted.status().is_success(), "{}", accepted.status());
    ensure!(can_log_in(&mut ws, "alice", NEW_PASSWORD).await?);
    Ok(())
}

#[tokio::test]
async fn an_admin_resets_another_users_password_without_theirs() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    user_token(&server, &mut ws, "alice", false).await?;
    let dana = user_token(&server, &mut ws, "dana", true).await?;

    let reset = ws
        .rpc_call(
            "user.passwd",
            json!({ "token": dana, "username": "alice", "new_password": NEW_PASSWORD }),
        )
        .await?;
    ensure!(reset.get("error").is_none(), "{reset}");
    ensure!(can_log_in(&mut ws, "alice", NEW_PASSWORD).await?);

    let own = ws
        .rpc_call(
            "user.passwd",
            json!({ "token": dana, "username": "dana", "new_password": NEW_PASSWORD }),
        )
        .await?;
    ensure!(
        own.get("error").is_some(),
        "an admin changed their own password without the current one: {own}"
    );
    Ok(())
}

// ── Host access from manifests ──

#[tokio::test]
async fn only_admins_may_give_a_container_host_access() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = user_token(&server, &mut ws, "alice", false).await?;

    for extra in [
        json!({ "privileged": true }),
        json!({ "volumes": [{ "src": "/", "dst": "/host" }] }),
    ] {
        let response = up_container(&mut ws, &token, extra.clone()).await?;
        ensure!(
            *error_code(&response) == json!(RpcErrorCode::AccessDenied),
            "a non-admin started a container with {extra}: {response}"
        );
    }
    ensure!(db::find_lab(&server.db, "hostacc1").await?.is_none());
    Ok(())
}

// ── Manifest file paths ──

#[tokio::test]
async fn the_server_never_reads_a_file_a_manifest_names() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = user_token(&server, &mut ws, "alice", false).await?;

    for extra in [
        json!({ "ssh_authorized_key_files": [{ "source": "/etc/ssh/ssh_host_ed25519_key.pub" }] }),
        json!({ "systemd_units": [{ "name": "x.service", "source": "/etc/passwd", "enabled": true }] }),
        json!({ "binary_files": [{ "source": "/etc/shadow" }] }),
    ] {
        let response = up_container(&mut ws, &token, extra.clone()).await?;
        let error = response["error"].to_string();
        ensure!(
            error.contains("sherpa up"),
            "a manifest file path reached the server unread ({extra}): {response}"
        );
    }
    Ok(())
}

// ── Impairment requests ──

#[tokio::test]
async fn out_of_range_impairment_numbers_are_refused_not_wrapped() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = user_token(&server, &mut ws, "alice", false).await?;

    for (field, value) in [
        ("link_index", json!(70_000)),
        ("delay", json!(5_000_000_000_u64)),
    ] {
        let mut params = json!({ "token": token, "lab_id": "abc12345", "link_index": 1 });
        params[field] = value;
        let response = ws.rpc_call("link.update_impairment", params).await?;
        ensure!(
            *error_code(&response) == json!(RpcErrorCode::InvalidParams),
            "{field} out of range was not refused: {response}"
        );
    }
    Ok(())
}

// ── Admin route access matrix ──

#[tokio::test]
async fn admin_pages_are_refused_to_others_and_served_to_admins() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    seed_matrix_image(&server).await?;
    let admin = ws.login_admin().await?;
    let user = user_token(&server, &mut ws, "alice", false).await?;
    let http = no_redirect_client()?;

    for path in ADMIN_PAGE_READS {
        let response = web_request(&http, &server, Method::GET, path, None).await?;
        ensure_login_redirect(&response, &format!("GET {path}"))?;
        let response = web_request(&http, &server, Method::GET, path, Some(&user)).await?;
        ensure!(
            response.status() == StatusCode::FORBIDDEN,
            "GET {path}: regular user got {}",
            response.status()
        );
        let response = web_request(&http, &server, Method::GET, path, Some(&admin)).await?;
        ensure!(
            response.status() == StatusCode::OK,
            "GET {path}: admin got {}",
            response.status()
        );
    }
    for path in ADMIN_PAGE_ACTIONS {
        let response = web_request(&http, &server, Method::POST, path, None).await?;
        ensure_login_redirect(&response, &format!("POST {path}"))?;
        let response = web_request(&http, &server, Method::POST, path, Some(&user)).await?;
        ensure!(
            response.status() == StatusCode::FORBIDDEN,
            "POST {path}: regular user got {}",
            response.status()
        );
    }
    Ok(())
}

#[tokio::test]
async fn admin_api_operations_are_refused_to_others() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    seed_matrix_image(&server).await?;
    let user = user_token(&server, &mut ws, "alice", false).await?;
    let http = no_redirect_client()?;

    for action in ADMIN_API_ACTIONS {
        let method = Method::from_bytes(action.method.as_bytes())?;
        let label = format!("{} {}", action.method, action.path);
        for (token, expected) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some(&user), StatusCode::FORBIDDEN),
        ] {
            let mut request = http
                .request(method.clone(), url(&server, action.path))
                .header(header::CONTENT_TYPE, action.content_type)
                .body(action.body);
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().await?;
            ensure!(
                response.status() == expected,
                "{label}: expected {expected}, got {}",
                response.status()
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn image_reads_need_a_login_but_not_admin() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    seed_matrix_image(&server).await?;
    let user = user_token(&server, &mut ws, "alice", false).await?;
    let http = no_redirect_client()?;

    for path in IMAGE_API_READS {
        let response = http.get(url(&server, path)).send().await?;
        ensure!(
            response.status() == StatusCode::UNAUTHORIZED,
            "GET {path}: anonymous caller got {}",
            response.status()
        );
        let response = http
            .get(url(&server, path))
            .bearer_auth(&user)
            .send()
            .await?;
        ensure!(
            response.status() == StatusCode::OK,
            "GET {path}: regular user got {}",
            response.status()
        );
        let body: Value = response.json().await?;
        ensure!(
            body.to_string().contains(MATRIX_IMAGE_VERSION),
            "GET {path} did not list the seeded image: {body}"
        );
    }
    Ok(())
}
