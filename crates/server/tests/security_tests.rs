//! Regression tests for the server's security boundaries: what an authenticated or anonymous
//! caller must not be able to do, on every transport.

mod helpers;

use anyhow::{Context, Result, ensure};
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};
use shared::error::RpcErrorCode;
use std::time::Duration;

use helpers::test_server::{TEST_ADMIN_PASSWORD, TEST_SSH_PUBLIC_KEY, TestServer, test_config};
use helpers::ws_client::TestWsClient;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
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
