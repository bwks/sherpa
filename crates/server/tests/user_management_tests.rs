mod helpers;

use anyhow::{Result, ensure};
use reqwest::{Client, Response, StatusCode, header};
use serde_json::json;
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;

use helpers::test_server::{TEST_ADMIN_PASSWORD, TestServer};
use helpers::ws_client::TestWsClient;

/// Element on the admin users page that receives a refused delete's message.
const USER_DELETE_ERROR_TARGET: &str = "#user-delete-error";

// ── User Creation ──

#[tokio::test]
#[ignore]
async fn test_admin_creates_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "testuser",
                "password": "TestUser123!",
                "is_admin": false,
            }),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "user.create should succeed"
    );
    assert!(
        response.get("error").is_none(),
        "user.create should not error"
    );

    // Verify user can login
    let user_token = ws.login("testuser", "TestUser123!").await?;
    assert!(!user_token.is_empty());

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_duplicate_username_rejected() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create user first time
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "dupuser",
                "password": "DupUser123!",
                "is_admin": false,
            }),
        )
        .await?;

    // Try to create same user again
    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "dupuser",
                "password": "DupUser123!",
                "is_admin": false,
            }),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "duplicate user should be rejected"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_non_admin_cannot_create_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Create a regular user
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "regular",
                "password": "Regular123!",
                "is_admin": false,
            }),
        )
        .await?;

    let user_token = ws.login("regular", "Regular123!").await?;

    // Try to create user as non-admin
    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": user_token,
                "username": "sneaky",
                "password": "Sneaky123!",
                "is_admin": false,
            }),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "non-admin should not create users"
    );
    assert_eq!(
        response
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32003) // AccessDenied
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_usernames_are_lowercased_and_case_insensitive() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "MixedCase",
                "password": "MixedCase123!",
                "is_admin": false,
            }),
        )
        .await?;
    let created = response
        .get("result")
        .and_then(|r| r.get("username"))
        .and_then(|u| u.as_str());
    assert_eq!(created, Some("mixedcase"), "{response}");

    // Login and self-service lookups ignore case.
    let user_token = ws.login("MIXEDCASE", "MixedCase123!").await?;
    let response = ws
        .rpc_call(
            "user.info",
            json!({ "token": user_token, "username": "MixedCase" }),
        )
        .await?;
    let username = response
        .get("result")
        .and_then(|r| r.get("user"))
        .and_then(|u| u.get("username"))
        .and_then(|u| u.as_str());
    assert_eq!(username, Some("mixedcase"), "{response}");

    // A case variant of an existing user is a duplicate.
    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "mixedCASE",
                "password": "MixedCase123!",
                "is_admin": false,
            }),
        )
        .await?;
    assert!(response.get("error").is_some(), "{response}");

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_invalid_and_reserved_usernames_rejected() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    for username in [
        "brad.searle",
        "brad@example.com",
        "1brad",
        "Admin",
        "sherpa",
        "ubuntu",
    ] {
        let response = ws
            .rpc_call(
                "user.create",
                json!({
                    "token": admin_token,
                    "username": username,
                    "password": "Rejected123!",
                    "is_admin": false,
                }),
            )
            .await?;
        assert!(
            response.get("error").is_some(),
            "'{username}' should be rejected: {response}"
        );
    }

    // The REST endpoint reports the rule as a bad request.
    let response = Client::new()
        .post(format!("http://{}/api/v1/admin/users", server.addr))
        .bearer_auth(&admin_token)
        .json(&json!({
            "username": "brad.searle",
            "password": "Rejected123!",
            "is_admin": false,
            "token": admin_token,
        }))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.text().await?;
    assert!(body.contains("lowercase letters"), "{body}");

    Ok(())
}

// ── User Info ──

#[tokio::test]
#[ignore]
async fn test_user_gets_own_info() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "user.info",
            json!({
                "token": token,
                "username": "admin",
            }),
        )
        .await?;

    let result = response.get("result").expect("should have result");
    // user.info returns GetUserInfoResponse { user: UserInfo { ... } }
    let user = result.get("user").expect("result should have 'user' field");
    assert_eq!(user.get("username").and_then(|v| v.as_str()), Some("admin"));
    assert_eq!(user.get("is_admin").and_then(|v| v.as_bool()), Some(true));

    Ok(())
}

// ── Password Change ──

#[tokio::test]
#[ignore]
async fn test_user_changes_own_password() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Create a user
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": admin_token,
                "username": "pwduser",
                "password": "OldPass123!",
                "is_admin": false,
            }),
        )
        .await?;

    let user_token = ws.login("pwduser", "OldPass123!").await?;

    // Change password
    let response = ws
        .rpc_call(
            "user.passwd",
            json!({
                "token": user_token,
                "username": "pwduser",
                "current_password": "OldPass123!",
                "new_password": "NewPass456!",
            }),
        )
        .await?;

    assert!(
        response.get("result").is_some(),
        "password change should succeed: {:?}",
        response
    );

    // Old password should fail
    let old_login = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "pwduser",
                "password": "OldPass123!",
            }),
        )
        .await?;
    assert!(old_login.get("error").is_some(), "old password should fail");

    // New password should work
    let new_token = ws.login("pwduser", "NewPass456!").await?;
    assert!(!new_token.is_empty());

    Ok(())
}

// ── User Listing ──

#[tokio::test]
#[ignore]
async fn test_admin_lists_users() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create a couple users
    for name in &["listuser1", "listuser2"] {
        let _r = ws
            .rpc_call(
                "user.create",
                json!({
                    "token": token,
                    "username": name,
                    "password": "ListUser123!",
                    "is_admin": false,
                }),
            )
            .await?;
    }

    let response = ws.rpc_call("user.list", json!({ "token": token })).await?;

    let result = response.get("result").expect("should have result");
    // user.list returns ListUsersResponse { users: Vec<UserInfo> }
    let users = result
        .get("users")
        .and_then(|v| v.as_array())
        .expect("result should have 'users' array");
    assert!(users.len() >= 3, "should have admin + 2 created users");

    Ok(())
}

// ── User Deletion ──

#[tokio::test]
#[ignore]
async fn test_admin_deletes_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create user
    let _r = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "todelete",
                "password": "ToDelete123!",
                "is_admin": false,
            }),
        )
        .await?;

    // Delete user
    let response = ws
        .rpc_call(
            "user.delete",
            json!({
                "token": token,
                "username": "todelete",
            }),
        )
        .await?;

    assert!(response.get("result").is_some(), "delete should succeed");

    // Verify login fails
    let login = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "todelete",
                "password": "ToDelete123!",
            }),
        )
        .await?;
    assert!(
        login.get("error").is_some(),
        "deleted user should not login"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_last_admin_cannot_be_deleted() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Try to delete the only admin
    let response = ws
        .rpc_call(
            "user.delete",
            json!({
                "token": token,
                "username": "admin",
            }),
        )
        .await?;

    assert!(
        response.get("error").is_some(),
        "should not be able to delete last admin"
    );

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_non_admin_cannot_delete_user() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin_token = ws.login_admin().await?;

    // Create two regular users
    for name in &["user_a", "user_b"] {
        let _r = ws
            .rpc_call(
                "user.create",
                json!({
                    "token": admin_token,
                    "username": name,
                    "password": "UserAB123!",
                    "is_admin": false,
                }),
            )
            .await?;
    }

    let user_token = ws.login("user_a", "UserAB123!").await?;

    // Try to delete another user
    let response = ws
        .rpc_call(
            "user.delete",
            json!({
                "token": user_token,
                "username": "user_b",
            }),
        )
        .await?;

    assert!(response.get("error").is_some());
    assert_eq!(
        response
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32003) // AccessDenied: non-admin cannot delete other users
    );

    Ok(())
}

// ── DB-Backed Verification via RPC ──

#[tokio::test]
#[ignore]
async fn test_db_reflects_user_operations() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Verify admin is accessible via user.info
    let admin_resp = ws
        .rpc_call("user.info", json!({ "token": token, "username": "admin" }))
        .await?;
    let admin = admin_resp
        .get("result")
        .and_then(|r| r.get("user"))
        .expect("admin user.info should return a result");
    assert_eq!(
        admin.get("username").and_then(|v| v.as_str()),
        Some("admin")
    );
    assert_eq!(admin.get("is_admin").and_then(|v| v.as_bool()), Some(true));

    // Create a user via RPC
    ws.rpc_call(
        "user.create",
        json!({
            "token": token,
            "username": "dbverify",
            "password": "DbVerify123!",
            "is_admin": false,
        }),
    )
    .await?;

    // Verify the new user is accessible via user.info
    let new_resp = ws
        .rpc_call(
            "user.info",
            json!({ "token": token, "username": "dbverify" }),
        )
        .await?;
    let new_user = new_resp
        .get("result")
        .and_then(|r| r.get("user"))
        .expect("dbverify user.info should return a result");
    assert_eq!(
        new_user.get("username").and_then(|v| v.as_str()),
        Some("dbverify")
    );
    assert_eq!(
        new_user.get("is_admin").and_then(|v| v.as_bool()),
        Some(false)
    );

    Ok(())
}

// ── User deletion rules on every transport ──

/// IPv4-only lab networks for tests that seed lab records directly.
fn ipv4_lab_networks() -> db::LabNetworks {
    db::LabNetworks {
        loopback_network: "127.125.1.0/24".to_string(),
        management_network: "10.202.1.0/24".to_string(),
        gateway_ipv4: "10.202.1.1".to_string(),
        router_ipv4: "10.202.1.2".to_string(),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
}

/// Delete a user through the web admin page with the admin's session cookie.
async fn web_delete_user(server: &TestServer, token: &str, username: &str) -> Result<Response> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .delete(format!("http://{}/admin/users/{username}", server.addr))
        .header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"))
        .send()
        .await?)
}

/// Assert the web admin page showed a refusal instead of removing the user's row.
async fn ensure_web_refusal(response: Response, expected: &str) -> Result<()> {
    let retarget = response
        .headers()
        .get("hx-retarget")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    ensure!(
        response.status() == StatusCode::OK && retarget == USER_DELETE_ERROR_TARGET,
        "web delete was not shown as a refusal: {} {retarget}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        body.contains(expected),
        "web refusal did not explain itself: {body}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn deleting_a_user_who_owns_labs_is_refused_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;
    let carol = db::create_user(
        &server.db,
        "carol".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    db::create_lab(
        &server.db,
        "carol-lab",
        "c0ffee01",
        &carol,
        &ipv4_lab_networks(),
    )
    .await?;

    let response = ws
        .rpc_call(
            "user.delete",
            json!({ "token": token, "username": "carol" }),
        )
        .await?;
    let message = response["error"]["message"].as_str().unwrap_or_default();
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::ServerError)
            && message.contains("c0ffee01"),
        "RPC delete of a lab owner was not refused with the labs listed: {response}"
    );

    let response = Client::new()
        .delete(format!("http://{}/api/v1/admin/users/carol", server.addr))
        .bearer_auth(&token)
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::CONFLICT,
        "REST delete of a lab owner returned {}",
        response.status()
    );

    ensure_web_refusal(web_delete_user(&server, &token, "carol").await?, "c0ffee01").await?;

    ensure!(
        db::find_user(&server.db, "carol").await?.is_some(),
        "a refused delete removed the user"
    );
    ensure!(
        db::find_lab(&server.db, "c0ffee01").await?.is_some(),
        "a refused delete removed the user's lab"
    );

    // Once the lab is gone, the delete succeeds.
    db::delete_lab(&server.db, "c0ffee01").await?;
    let response = ws
        .rpc_call(
            "user.delete",
            json!({ "token": token, "username": "carol" }),
        )
        .await?;
    ensure!(
        response.get("error").is_none(),
        "delete of a user without labs failed: {response}"
    );
    ensure!(db::find_user(&server.db, "carol").await?.is_none());
    Ok(())
}

#[tokio::test]
#[ignore = "requires the disposable SurrealDB test database"]
async fn self_delete_is_refused_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    // A second admin, so the self-delete rule is what refuses, not the last-admin rule.
    db::create_user(
        &server.db,
        "dana".to_owned(),
        TEST_ADMIN_PASSWORD,
        true,
        vec![],
    )
    .await?;
    let token = ws.login("dana", TEST_ADMIN_PASSWORD).await?;

    let response = ws
        .rpc_call("user.delete", json!({ "token": token, "username": "dana" }))
        .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "RPC self-delete was not refused: {response}"
    );

    let response = Client::new()
        .delete(format!("http://{}/api/v1/admin/users/dana", server.addr))
        .bearer_auth(&token)
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::FORBIDDEN,
        "REST self-delete returned {}",
        response.status()
    );

    ensure_web_refusal(
        web_delete_user(&server, &token, "dana").await?,
        "your own account",
    )
    .await?;

    ensure!(
        db::find_user(&server.db, "dana").await?.is_some(),
        "a refused self-delete removed the user"
    );
    Ok(())
}
