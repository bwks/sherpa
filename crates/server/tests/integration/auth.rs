use anyhow::{Result, ensure};
use reqwest::header::HeaderName;
use reqwest::{Method, Response, StatusCode, header};
use serde_json::json;
use shared::api_spec::{HttpMethod, build_spec};
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;

use crate::helpers::http_client::TestHttpClient;
use crate::helpers::test_server::{TEST_ADMIN_PASSWORD, TestServer};
use crate::helpers::ws_client::TestWsClient;

/// Operations the spec gives a REST binding the server does not route yet.
const KNOWN_UNROUTED: &[&str] = &[];

// ── JWT Authentication Tests ──

#[tokio::test]
async fn test_auth_login_valid_credentials() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "admin",
                "password": TEST_ADMIN_PASSWORD,
            }),
        )
        .await?;

    let result = response.get("result").expect("should have result");
    assert!(result.get("token").is_some(), "should have token");
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );
    assert_eq!(
        result.get("is_admin").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(result.get("expires_at").is_some(), "should have expires_at");

    Ok(())
}

#[tokio::test]
async fn test_auth_login_invalid_password() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "admin",
                "password": "WrongPassword123!",
            }),
        )
        .await?;

    assert!(response.get("error").is_some(), "should have error");
    assert!(response.get("result").is_none(), "should not have result");

    Ok(())
}

#[tokio::test]
async fn test_auth_login_nonexistent_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "nobody",
                "password": "SomePass123!",
            }),
        )
        .await?;

    assert!(response.get("error").is_some());

    Ok(())
}

#[tokio::test]
async fn test_auth_login_missing_params() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("auth.login", json!({})).await?;

    let error = response.get("error").expect("should have error");
    assert_eq!(
        error.get("code").and_then(serde_json::Value::as_i64),
        Some(-32602)
    ); // InvalidParams

    Ok(())
}

#[tokio::test]
async fn test_auth_validate_valid_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": token }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(
        result.get("valid").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );

    Ok(())
}

#[tokio::test]
async fn a_token_validates_over_rest_as_over_rpc() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    let client = TestHttpClient::new(server.addr);

    let valid: serde_json::Value = client
        .post_json("/api/v1/auth/validate", &json!({ "token": token }))
        .await?
        .json()
        .await?;
    assert_eq!(valid["valid"], true, "{valid}");
    assert_eq!(valid["username"], "admin", "{valid}");

    let invalid: serde_json::Value = client
        .post_json("/api/v1/auth/validate", &json!({ "token": "not-a-token" }))
        .await?
        .json()
        .await?;
    assert_eq!(invalid["valid"], false, "{invalid}");
    Ok(())
}

#[tokio::test]
async fn test_auth_validate_invalid_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": "invalid.jwt.token" }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(
        result.get("valid").and_then(serde_json::Value::as_bool),
        Some(false)
    );

    Ok(())
}

#[tokio::test]
async fn test_auth_validate_missing_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("auth.validate", json!({})).await?;

    let error = response.get("error").expect("should have error");
    assert_eq!(
        error.get("code").and_then(serde_json::Value::as_i64),
        Some(-32602)
    );

    Ok(())
}

// ── Authenticated Request Tests ──

#[tokio::test]
async fn test_rpc_requires_auth_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    // Try inspect without token
    let response = ws
        .rpc_call("inspect", json!({ "lab_id": "nonexistent" }))
        .await?;

    let error = response.get("error").expect("should have error");
    // AuthRequired = -32002
    assert_eq!(
        error.get("code").and_then(serde_json::Value::as_i64),
        Some(-32002)
    );

    Ok(())
}

#[tokio::test]
async fn test_admin_rpc_denied_for_non_admin() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    // First create a non-admin user
    let admin_token = ws.login_admin().await?;
    let _create_resp = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "regular",
                "password": "RegularPass123!",
                "is_admin": false,
            }),
        )
        .await?;

    // Login as non-admin
    let user_token = ws.login("regular", "RegularPass123!").await?;

    // Try admin-only operation
    let response = ws
        .rpc_call("user.list", json!({ "token": user_token }))
        .await?;

    let error = response.get("error").expect("should have error");
    // AccessDenied = -32003
    assert_eq!(
        error.get("code").and_then(serde_json::Value::as_i64),
        Some(-32003)
    );

    Ok(())
}

// ── Cookie Session Tests ──

#[tokio::test]
async fn test_cookie_login_valid() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let resp = client
        .post_form(
            "/login",
            &[("username", "admin"), ("password", TEST_ADMIN_PASSWORD)],
        )
        .await?;

    // Login uses HTMX pattern: 200 OK with hx-redirect header (not a 3xx redirect)
    assert_eq!(
        resp.status().as_u16(),
        200,
        "Expected 200 OK for HTMX login, got {}",
        resp.status()
    );
    assert!(
        resp.headers().contains_key("hx-redirect"),
        "Expected hx-redirect header in login response"
    );

    Ok(())
}

#[tokio::test]
async fn test_cookie_login_invalid() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let status = client.login_form("admin", "WrongPass123!").await?;

    // Should return 200 with login page showing error, or redirect back to login
    assert!(
        status.as_u16() == 200 || status.is_redirection(),
        "Expected 200 or redirect for invalid login, got {status}"
    );

    Ok(())
}

// ── HTTP Route Tests ──

#[tokio::test]
async fn test_health_check() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.get("/health").await?;
    assert_eq!(response.status().as_u16(), 200);

    Ok(())
}

#[tokio::test]
async fn test_login_page_renders() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.get("/login").await?;
    assert_eq!(response.status().as_u16(), 200);

    let body = response.text().await?;
    assert!(
        body.contains("login") || body.contains("Login"),
        "Login page should contain login form"
    );

    Ok(())
}

#[tokio::test]
async fn test_protected_route_redirects_unauthenticated() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.get("/").await?;

    // Should redirect to login
    assert!(
        response.status().is_redirection() || response.status().as_u16() == 401,
        "Expected redirect or 401, got {}",
        response.status()
    );

    Ok(())
}

#[tokio::test]
async fn test_api_spec_returns_json() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.get("/api/v1/spec").await?;
    assert_eq!(response.status().as_u16(), 200);

    Ok(())
}

// ── HTTP REST API Tests ──

#[tokio::test]
async fn test_http_bearer_auth_list_users() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token);

    let response = client.get("/api/v1/admin/users").await?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "GET /api/v1/admin/users with bearer token should return 200"
    );

    Ok(())
}

#[tokio::test]
async fn test_http_bearer_auth_create_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token.clone());

    let response = client
        .post_json(
            "/api/v1/admin/users",
            &serde_json::json!({
                "username": "httprestuser",
                "password": "HttpPass123!",
                "is_admin": false,
                "token": token,
            }),
        )
        .await?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "POST /api/v1/admin/users should return 200, got: {}",
        response.status()
    );

    Ok(())
}

#[tokio::test]
async fn test_http_bearer_auth_delete_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create a user to delete
    ws.rpc_call(
        "user.create",
        serde_json::json!({
            "token": token.clone(),
            "username": "httpdel",
            "password": "DelPass123!",
            "is_admin": false,
        }),
    )
    .await?;

    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token);

    let response = client.delete("/api/v1/admin/users/httpdel").await?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "DELETE /api/v1/admin/users/httpdel should return 200, got: {}",
        response.status()
    );

    Ok(())
}

#[tokio::test]
async fn test_stale_cookie_login_recovery_and_duplicate_replacement() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let stale_response = client
        .get_with_cookie("/", "sherpa_auth=stale-session")
        .await?;
    assert_eq!(stale_response.status().as_u16(), 303);
    assert_eq!(
        stale_response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/login?error=session_expired")
    );
    let clear_cookie = stale_response
        .headers()
        .get(header::SET_COOKIE)
        .expect("stale session response should clear the cookie")
        .to_str()?;
    assert!(clear_cookie.contains("Max-Age=0"));

    let stale_login_page = client
        .get_with_cookie("/login", "sherpa_auth=stale-session")
        .await?;
    assert_eq!(stale_login_page.status().as_u16(), 200);
    let login_page_clear_cookie = stale_login_page
        .headers()
        .get(header::SET_COOKIE)
        .expect("login page should clear an invalid session")
        .to_str()?;
    assert!(login_page_clear_cookie.contains("Max-Age=0"));

    let login_response = client
        .post_form(
            "/login",
            &[("username", "admin"), ("password", TEST_ADMIN_PASSWORD)],
        )
        .await?;
    assert_eq!(login_response.status().as_u16(), 200);
    let replacement_cookie = login_response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            value
                .split(';')
                .next()
                .filter(|pair| pair.starts_with("sherpa_auth="))
        })
        .expect("successful login should return an auth cookie");

    let authenticated_login_page = client.get_with_cookie("/login", replacement_cookie).await?;
    assert_eq!(authenticated_login_page.status().as_u16(), 303);
    assert_eq!(
        authenticated_login_page
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/admin/users")
    );

    let duplicate_cookie = format!("sherpa_auth=stale-session; {replacement_cookie}");
    let protected_response = client
        .get_with_cookie("/admin/users", &duplicate_cookie)
        .await?;
    assert_eq!(protected_response.status().as_u16(), 200);

    Ok(())
}

// ── Database-backed identity ──

#[tokio::test]
async fn demoted_admin_token_loses_admin_rights_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "carol".to_owned(),
        TEST_ADMIN_PASSWORD,
        true,
        vec![],
    )
    .await?;
    let token = ws.login("carol", TEST_ADMIN_PASSWORD).await?;

    let mut carol = db::get_user(&server.db, "carol").await?;
    carol.is_admin = false;
    db::update_user(&server.db, carol).await?;

    let response = ws.rpc_call("user.list", json!({ "token": token })).await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "Demoted admin kept RPC admin rights: {response}"
    );

    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token.clone());
    let response = client.get("/api/v1/admin/users").await?;
    ensure!(
        response.status() == reqwest::StatusCode::FORBIDDEN,
        "Demoted admin kept REST admin rights: {}",
        response.status()
    );

    let response = TestHttpClient::new(server.addr)
        .get_with_cookie("/admin/users", &format!("{AUTH_COOKIE_NAME}={token}"))
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::FORBIDDEN,
        "Demoted admin kept web admin rights: {}",
        response.status()
    );
    Ok(())
}

#[tokio::test]
async fn deleted_user_token_is_rejected_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let dave = db::create_user(
        &server.db,
        "dave".to_owned(),
        TEST_ADMIN_PASSWORD,
        true,
        vec![],
    )
    .await?;
    let token = ws.login("dave", TEST_ADMIN_PASSWORD).await?;
    db::delete_user(&server.db, db::get_user_id(&dave)?).await?;

    let response = ws.rpc_call("user.list", json!({ "token": token })).await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AuthRequired),
        "Deleted user was not asked to authenticate over RPC: {response}"
    );

    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token.clone());
    let response = client.get("/api/v1/admin/users").await?;
    ensure!(
        response.status() == reqwest::StatusCode::UNAUTHORIZED,
        "Deleted user was not rejected over REST: {}",
        response.status()
    );

    let response = TestHttpClient::new(server.addr)
        .get_with_cookie("/labs", &format!("{AUTH_COOKIE_NAME}={token}"))
        .await?;
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    ensure!(
        response.status().is_redirection() && location.contains("session_expired"),
        "Deleted user's session was not cleared: {} {location}",
        response.status()
    );
    Ok(())
}

// ── Sessions (M14) ──

const BOB_PASSWORD: &str = "Bob-Original-Pass-1!";
const BOB_NEW_PASSWORD: &str = "Bob-Changed-Pass-2!";

/// Whether `token` still authenticates an RPC call that needs a session.
async fn rpc_token_works(ws: &mut TestWsClient, token: &str) -> Result<bool> {
    let response = ws.rpc_call("labs.list", json!({ "token": token })).await?;
    if response["error"]["code"] == json!(RpcErrorCode::AuthRequired) {
        return Ok(false);
    }
    ensure!(
        response.get("error").is_none(),
        "labs.list failed: {response}"
    );
    Ok(true)
}

/// Whether `token` is answered `valid` by `auth.validate`.
async fn token_is_valid(ws: &mut TestWsClient, token: &str) -> Result<bool> {
    let response = ws
        .rpc_call("auth.validate", json!({ "token": token }))
        .await?;
    Ok(response["result"]["valid"] == json!(true))
}

/// The `sherpa_auth` token a web login response sets.
fn cookie_token(response: &Response) -> Result<String> {
    let cookie = header_text(response, header::SET_COOKIE);
    let token = cookie
        .strip_prefix(&format!("{AUTH_COOKIE_NAME}="))
        .and_then(|rest| rest.split(';').next())
        .unwrap_or_default()
        .to_owned();
    ensure!(!token.is_empty(), "login set no auth cookie: {cookie}");
    Ok(token)
}

#[tokio::test]
async fn rpc_logout_ends_only_that_session() -> Result<()> {
    let server = TestServer::start().await?;
    db::create_user(&server.db, "bob".to_owned(), BOB_PASSWORD, false, vec![]).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let first = ws.login("bob", BOB_PASSWORD).await?;
    let second = ws.login("bob", BOB_PASSWORD).await?;

    let response = ws
        .rpc_call("auth.logout", json!({ "token": first }))
        .await?;
    ensure!(
        response.get("error").is_none(),
        "auth.logout failed: {response}"
    );
    ensure!(
        response["result"]["ended"] == json!(true),
        "logout: {response}"
    );

    ensure!(
        !rpc_token_works(&mut ws, &first).await?,
        "A logged-out token still works"
    );
    ensure!(
        !token_is_valid(&mut ws, &first).await?,
        "auth.validate accepts a logged-out token"
    );
    ensure!(
        rpc_token_works(&mut ws, &second).await?,
        "Logout ended another session"
    );
    ensure!(
        token_is_valid(&mut ws, &second).await?,
        "auth.validate refuses a live token"
    );
    Ok(())
}

#[tokio::test]
async fn rest_logout_ends_the_bearer_session() -> Result<()> {
    let server = TestServer::start().await?;
    let mut client = TestHttpClient::new(server.addr);
    let response = client
        .post_json(
            "/api/v1/auth/login",
            &json!({ "username": "admin", "password": TEST_ADMIN_PASSWORD }),
        )
        .await?;
    let body: serde_json::Value = response.json().await?;
    client.set_token(body["token"].as_str().unwrap_or_default().to_owned());

    let response = client.post_json("/api/v1/auth/logout", &json!({})).await?;
    ensure!(
        response.status() == StatusCode::OK,
        "logout: {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await?;
    ensure!(body["ended"] == json!(true), "logout: {body}");

    let response = client.get("/api/v1/admin/users").await?;
    ensure!(
        response.status() == StatusCode::UNAUTHORIZED,
        "A logged-out token still works over REST: {}",
        response.status()
    );
    Ok(())
}

#[tokio::test]
async fn web_logout_ends_the_cookie_session_on_the_server() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);
    let response = client
        .post_form(
            "/login",
            &[("username", "admin"), ("password", TEST_ADMIN_PASSWORD)],
        )
        .await?;
    let token = cookie_token(&response)?;
    let cookie = format!("{AUTH_COOKIE_NAME}={token}");

    // The client's cookie jar sends the session with the logout.
    let response = client.post_form("/logout", &[]).await?;
    ensure!(
        response.status() == StatusCode::SEE_OTHER,
        "logout: {}",
        response.status()
    );

    // A copy of the cookie taken before logout no longer works.
    let response = TestHttpClient::new(server.addr)
        .get_with_cookie("/labs", &cookie)
        .await?;
    let location = header_text(&response, header::LOCATION);
    ensure!(
        response.status().is_redirection() && location.contains("session_expired"),
        "A logged-out cookie still works: {} {location}",
        response.status()
    );
    Ok(())
}

#[tokio::test]
async fn changing_your_password_ends_your_other_sessions() -> Result<()> {
    let server = TestServer::start().await?;
    db::create_user(&server.db, "bob".to_owned(), BOB_PASSWORD, false, vec![]).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let current = ws.login("bob", BOB_PASSWORD).await?;
    let other = ws.login("bob", BOB_PASSWORD).await?;

    let response = ws
        .rpc_call(
            "user.passwd",
            json!({
                "token": current,
                "username": "bob",
                "current_password": BOB_PASSWORD,
                "new_password": BOB_NEW_PASSWORD,
            }),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "user.passwd failed: {response}"
    );

    ensure!(
        rpc_token_works(&mut ws, &current).await?,
        "The session that changed it ended"
    );
    ensure!(
        !rpc_token_works(&mut ws, &other).await?,
        "Another session survived"
    );
    Ok(())
}

#[tokio::test]
async fn an_admin_password_reset_ends_every_session_of_the_user() -> Result<()> {
    let server = TestServer::start().await?;
    db::create_user(&server.db, "bob".to_owned(), BOB_PASSWORD, false, vec![]).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let bob = ws.login("bob", BOB_PASSWORD).await?;
    let admin = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "user.passwd",
            json!({ "token": admin, "username": "bob", "new_password": BOB_NEW_PASSWORD }),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "user.passwd failed: {response}"
    );

    ensure!(
        !rpc_token_works(&mut ws, &bob).await?,
        "Bob's session survived the reset"
    );
    ensure!(
        rpc_token_works(&mut ws, &admin).await?,
        "The admin's own session ended"
    );
    Ok(())
}

#[tokio::test]
async fn a_recreated_user_does_not_inherit_the_old_accounts_tokens() -> Result<()> {
    let server = TestServer::start().await?;
    let bob = db::create_user(&server.db, "bob".to_owned(), BOB_PASSWORD, false, vec![]).await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login("bob", BOB_PASSWORD).await?;

    db::delete_user(&server.db, db::get_user_id(&bob)?).await?;
    db::create_user(
        &server.db,
        "bob".to_owned(),
        BOB_NEW_PASSWORD,
        false,
        vec![],
    )
    .await?;

    ensure!(
        !rpc_token_works(&mut ws, &token).await?,
        "The old account's token authenticates as the new account over RPC"
    );
    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token.clone());
    let response = client.get("/api/v1/labs").await?;
    ensure!(
        response.status() == StatusCode::UNAUTHORIZED,
        "The old account's token works over REST: {}",
        response.status()
    );
    let response = TestHttpClient::new(server.addr)
        .get_with_cookie("/labs", &format!("{AUTH_COOKIE_NAME}={token}"))
        .await?;
    let location = header_text(&response, header::LOCATION);
    ensure!(
        response.status().is_redirection() && location.contains("session_expired"),
        "The old account's cookie works: {} {location}",
        response.status()
    );
    Ok(())
}

// ── API spec routes ──

/// Every REST binding in the published spec is a route the server serves. The request is sent
/// without a token: a route that exists answers 401 or 400, one that does not answers 404 or 405.
#[tokio::test]
async fn every_rest_operation_in_the_spec_is_routed() -> Result<()> {
    let server = TestServer::start().await?;
    let client = reqwest::Client::new();
    let mut unrouted = Vec::new();

    for op in build_spec().operations {
        let rest = &op.transports.rest;
        let mut path = rest.path.to_owned();
        for param in &rest.path_params {
            path = path.replace(&format!("{{{param}}}"), "spec-check");
        }
        let method = match rest.method {
            HttpMethod::Get => Method::GET,
            HttpMethod::Post => Method::POST,
            HttpMethod::Delete => Method::DELETE,
        };
        let status = client
            .request(method.clone(), format!("http://{}{path}", server.addr))
            .send()
            .await?
            .status();
        let routed = !matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        );
        if routed == KNOWN_UNROUTED.contains(&op.name) {
            unrouted.push(format!("{} {method} {path}: {status}", op.name));
        }
    }

    assert!(
        unrouted.is_empty(),
        "Routing differs from the spec (update KNOWN_UNROUTED if one was fixed):\n{}",
        unrouted.join("\n")
    );
    Ok(())
}

// ── REST login, logout and public docs ──

/// A response header as text, or empty when it is missing.
fn header_text(response: &Response, name: HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn rest_login_issues_a_token_only_for_the_right_password() -> Result<()> {
    let server = TestServer::start().await?;
    let mut client = TestHttpClient::new(server.addr);

    let response = client
        .post_json(
            "/api/v1/auth/login",
            &json!({ "username": "admin", "password": "wrong-password" }),
        )
        .await?;
    ensure!(
        response.status() == StatusCode::UNAUTHORIZED,
        "wrong password: {}",
        response.status()
    );

    let response = client
        .post_json(
            "/api/v1/auth/login",
            &json!({ "username": "admin", "password": TEST_ADMIN_PASSWORD }),
        )
        .await?;
    ensure!(
        response.status() == StatusCode::OK,
        "right password: {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await?;
    ensure!(
        body["username"] == "admin" && body["is_admin"] == true,
        "login response: {body}"
    );
    let token = body["token"].as_str().unwrap_or_default().to_owned();
    ensure!(!token.is_empty(), "login returned no token: {body}");

    // The token authenticates REST calls.
    client.set_token(token);
    let response = client.get("/api/v1/admin/users").await?;
    ensure!(
        response.status() == StatusCode::OK,
        "issued token was refused: {}",
        response.status()
    );
    Ok(())
}

#[tokio::test]
async fn logout_clears_the_session_cookie() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.post_form("/logout", &[]).await?;
    ensure!(
        response.status() == StatusCode::SEE_OTHER,
        "logout: {}",
        response.status()
    );
    let location = header_text(&response, header::LOCATION);
    let cookie = header_text(&response, header::SET_COOKIE);
    ensure!(
        location == "/login?error=logout_success",
        "logout redirected to {location}"
    );
    ensure!(
        cookie.starts_with(&format!("{AUTH_COOKIE_NAME}=;")) && cookie.contains("Max-Age=0"),
        "logout did not clear the auth cookie: {cookie}"
    );
    Ok(())
}

#[tokio::test]
async fn the_openapi_document_and_docs_page_are_public() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(server.addr);

    let response = client.get("/api/v1/openapi.json").await?;
    ensure!(
        response.status() == StatusCode::OK,
        "openapi.json: {}",
        response.status()
    );
    let doc: serde_json::Value = response.json().await?;
    ensure!(
        doc["openapi"] == "3.1.0",
        "not an OpenAPI 3.1 document: {doc}"
    );
    for path in ["/api/v1/labs", "/api/v1/images", "/api/v1/auth/login"] {
        ensure!(
            doc["paths"].get(path).is_some(),
            "openapi.json does not list {path}"
        );
    }

    let response = client.get("/api/docs").await?;
    let content_type = header_text(&response, header::CONTENT_TYPE);
    ensure!(
        response.status() == StatusCode::OK && content_type.starts_with("text/html"),
        "docs page: {} {content_type}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        body.to_lowercase().contains("swagger"),
        "docs page is not the Swagger UI: {body}"
    );
    Ok(())
}
