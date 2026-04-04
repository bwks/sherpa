mod helpers;

use anyhow::Result;
use serde_json::json;

use helpers::test_server::TestServer;
use helpers::ws_client::TestWsClient;

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
