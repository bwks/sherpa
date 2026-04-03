mod helpers;

use anyhow::Result;
use serde_json::json;

use helpers::http_client::TestHttpClient;
use helpers::test_server::TestServer;
use helpers::ws_client::TestWsClient;

// ── JWT Authentication Tests ──

#[tokio::test]
#[ignore]
async fn test_auth_login_valid_credentials() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "admin",
                "password": helpers::test_server::TEST_ADMIN_PASSWORD,
            }),
        )
        .await?;

    let result = response.get("result").expect("should have result");
    assert!(result.get("token").is_some(), "should have token");
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );
    assert_eq!(result.get("is_admin").and_then(|v| v.as_bool()), Some(true));
    assert!(result.get("expires_at").is_some(), "should have expires_at");

    Ok(())
}

#[tokio::test]
#[ignore]
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
#[ignore]
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
#[ignore]
async fn test_auth_login_missing_params() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("auth.login", json!({})).await?;

    let error = response.get("error").expect("should have error");
    assert_eq!(error.get("code").and_then(|v| v.as_i64()), Some(-32602)); // InvalidParams

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_auth_validate_valid_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": token }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        result.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_auth_validate_invalid_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws
        .rpc_call("auth.validate", json!({ "token": "invalid.jwt.token" }))
        .await?;

    let result = response.get("result").expect("should have result");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(false));

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_auth_validate_missing_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    let response = ws.rpc_call("auth.validate", json!({})).await?;

    let error = response.get("error").expect("should have error");
    assert_eq!(error.get("code").and_then(|v| v.as_i64()), Some(-32602));

    Ok(())
}

// ── Authenticated Request Tests ──

#[tokio::test]
#[ignore]
async fn test_rpc_requires_auth_token() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;

    // Try inspect without token
    let response = ws
        .rpc_call("inspect", json!({ "lab_id": "nonexistent" }))
        .await?;

    let error = response.get("error").expect("should have error");
    // AuthRequired = -32002
    assert_eq!(error.get("code").and_then(|v| v.as_i64()), Some(-32002));

    Ok(())
}

#[tokio::test]
#[ignore]
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
    assert_eq!(error.get("code").and_then(|v| v.as_i64()), Some(-32003));

    Ok(())
}

// ── Cookie Session Tests ──

#[tokio::test]
#[ignore]
async fn test_cookie_login_valid() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

    let resp = client
        .post_form(
            "/login",
            &[
                ("username", "admin"),
                ("password", helpers::test_server::TEST_ADMIN_PASSWORD),
            ],
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
#[ignore]
async fn test_cookie_login_invalid() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

    let status = client.login_form("admin", "WrongPass123!").await?;

    // Should return 200 with login page showing error, or redirect back to login
    assert!(
        status.as_u16() == 200 || status.is_redirection(),
        "Expected 200 or redirect for invalid login, got {}",
        status
    );

    Ok(())
}

// ── HTTP Route Tests ──

#[tokio::test]
#[ignore]
async fn test_health_check() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

    let response = client.get("/health").await?;
    assert_eq!(response.status().as_u16(), 200);

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_login_page_renders() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

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
#[ignore]
async fn test_protected_route_redirects_unauthenticated() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

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
#[ignore]
async fn test_api_spec_returns_json() -> Result<()> {
    let server = TestServer::start().await?;
    let client = TestHttpClient::new(&server);

    let response = client.get("/api/v1/spec").await?;
    assert_eq!(response.status().as_u16(), 200);

    Ok(())
}
