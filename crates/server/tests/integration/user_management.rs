use anyhow::{Result, ensure};
use db::LabDetails;
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response, StatusCode, header};
use serde_json::json;
use shared::data::LabPhase;
use shared::error::RpcErrorCode;
use shared::util::ssh_key_fingerprint;
use sherpad::ServiceError;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;
use sherpad::daemon::state::ensure_admin_account;

use crate::helpers::test_server::{
    TEST_ADMIN_PASSWORD, TEST_SSH_PUBLIC_KEY, TestServer, empty_database,
};
use crate::helpers::ws_client::TestWsClient;

/// Element on the admin users page that receives a refused delete's message.
const USER_DELETE_ERROR_TARGET: &str = "#user-delete-error";
/// A second valid public key, distinct from `TEST_SSH_PUBLIC_KEY`.
const SECOND_SSH_PUBLIC_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID/jzzYPs04UP8SineXGU7LMkDnc71EIPeVx48sC5jXh second";
/// The password the profile password test changes to.
const NEW_PASSWORD: &str = "Changed-Pass-456!";

// ── User Creation ──

#[tokio::test]
async fn user_accounts_001_an_admin_creates_a_user_who_can_log_in() -> Result<()> {
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
    assert_ne!(user_token, "");

    Ok(())
}

#[tokio::test]
async fn user_accounts_004_a_username_already_taken_in_any_case_is_refused() -> Result<()> {
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

    // Try to create the same user again, then a case variant of it
    for username in ["dupuser", "dupUSER"] {
        let response = ws
            .rpc_call(
                "user.create",
                json!({
                    "token": token,
                    "username": username,
                    "password": "DupUser123!",
                    "is_admin": false,
                }),
            )
            .await?;

        assert!(
            response.get("error").is_some(),
            "duplicate user '{username}' should be rejected: {response}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn admin_only_002_every_admin_rpc_method_is_refused_to_a_user_who_is_not_an_admin()
-> Result<()> {
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

    for (method, params) in [
        ("user.list", json!({ "token": user_token })),
        (
            "user.create",
            json!({
                "token": user_token,
                "username": "sneaky",
                "password": "Sneaky123!",
                "is_admin": false,
            }),
        ),
        (
            "user.delete",
            json!({ "token": user_token, "username": "user_b" }),
        ),
    ] {
        let response = ws.rpc_call(method, params).await?;
        assert_eq!(
            response["error"]["code"],
            json!(RpcErrorCode::AccessDenied),
            "{method}: {response}"
        );
    }
    assert!(db::find_user(&server.db, "sneaky").await?.is_none());
    assert!(db::find_user(&server.db, "user_b").await?.is_some());

    Ok(())
}

#[tokio::test]
async fn user_accounts_003_a_username_is_stored_in_lower_case_and_found_in_any_case() -> Result<()>
{
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

    Ok(())
}

#[tokio::test]
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

// ── Password Change ──

// ── User Listing ──

#[tokio::test]
async fn user_accounts_010_an_admin_lists_users() -> Result<()> {
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
    let mut usernames: Vec<&str> = result["users"]
        .as_array()
        .expect("result should have 'users' array")
        .iter()
        .filter_map(|user| user["username"].as_str())
        .collect();
    usernames.sort_unstable();
    assert_eq!(
        usernames,
        ["listuser1", "listuser2", "sherpa-admin"],
        "{response}"
    );

    Ok(())
}

// ── User Deletion ──

#[tokio::test]
async fn user_accounts_013_an_admin_deletes_a_user_who_can_no_longer_log_in() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create users
    for (username, password) in [("todelete", "ToDelete123!"), ("httpdel", "DelPass123!")] {
        let _r = ws
            .rpc_call(
                "user.create",
                json!({
                    "token": token,
                    "username": username,
                    "password": password,
                    "is_admin": false,
                }),
            )
            .await?;
    }

    // Delete a user over RPC
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

    // Delete a user over REST
    let response = Client::new()
        .delete(format!("http://{}/api/v1/admin/users/httpdel", server.addr))
        .bearer_auth(&token)
        .send()
        .await?;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "DELETE /api/v1/admin/users/httpdel should return 200"
    );
    let login = ws
        .rpc_call(
            "auth.login",
            json!({
                "username": "httpdel",
                "password": "DelPass123!",
            }),
        )
        .await?;
    assert!(
        login.get("error").is_some(),
        "a user deleted over REST should not login"
    );

    Ok(())
}

// ── DB-Backed Verification via RPC ──

#[tokio::test]
async fn user_accounts_002_a_user_created_as_an_admin_is_one_and_one_created_without_is_not()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Create users over RPC and REST, with and without the admin flag
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
    ws.rpc_call(
        "user.create",
        json!({
            "token": token,
            "username": "rpcadmin",
            "password": "RpcAdmin123!",
            "is_admin": true,
        }),
    )
    .await?;
    for (username, password, is_admin) in [
        ("httprestuser", "HttpPass123!", false),
        ("httprestadmin", "HttpAdmin123!", true),
    ] {
        let response = Client::new()
            .post(format!("http://{}/api/v1/admin/users", server.addr))
            .bearer_auth(&token)
            .json(&json!({
                "username": username,
                "password": password,
                "is_admin": is_admin,
                "token": token,
            }))
            .send()
            .await?;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "POST /api/v1/admin/users for {username}"
        );
    }

    for (username, is_admin) in [
        ("sherpa-admin", true),
        ("dbverify", false),
        ("rpcadmin", true),
        ("httprestuser", false),
        ("httprestadmin", true),
    ] {
        let response = ws
            .rpc_call("user.info", json!({ "token": token, "username": username }))
            .await?;
        assert_eq!(
            response["result"]["user"]["is_admin"],
            json!(is_admin),
            "{username}: {response}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn user_accounts_011_an_admin_reads_another_accounts_details() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    // Verify admin is accessible via user.info
    let admin_resp = ws
        .rpc_call(
            "user.info",
            json!({ "token": token, "username": "sherpa-admin" }),
        )
        .await?;
    let admin = admin_resp
        .get("result")
        .and_then(|r| r.get("user"))
        .expect("admin user.info should return a result");
    assert_eq!(
        admin.get("username").and_then(|v| v.as_str()),
        Some("sherpa-admin")
    );

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

    Ok(())
}

// ── Account pages ──

/// Send a web request with a session cookie, and a form body when `form` is not empty.
async fn web_request(
    server: &TestServer,
    token: &str,
    method: Method,
    path: &str,
    form: &[(&str, &str)],
) -> Result<Response> {
    let mut request = Client::builder()
        .redirect(Policy::none())
        .build()?
        .request(method, format!("http://{}{path}", server.addr))
        .header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"));
    if !form.is_empty() {
        request = request.form(form);
    }
    Ok(request.send().await?)
}

/// The user's SSH keys as stored.
async fn stored_keys(server: &TestServer, username: &str) -> Result<Vec<String>> {
    Ok(db::get_user(&server.db, username).await?.ssh_keys)
}

#[tokio::test]
async fn account_keys_010_the_profile_page_validates_adds_and_removes_keys_showing_the_list()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "keyowner".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![TEST_SSH_PUBLIC_KEY.to_owned()],
    )
    .await?;
    let token = ws.login("keyowner", TEST_ADMIN_PASSWORD).await?;
    let key_body = TEST_SSH_PUBLIC_KEY
        .split_whitespace()
        .nth(1)
        .unwrap_or_default();

    let response = web_request(&server, &token, Method::GET, "/profile", &[]).await?;
    ensure!(
        response.status() == StatusCode::OK,
        "GET /profile: {}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        body.contains(r#"<div id="ssh-keys-list">"#) && body.contains(key_body),
        "GET /profile did not render the SSH key list: {body}"
    );

    for (key, refusal) in [
        ("not-a-key", "Invalid SSH public key"),
        (TEST_SSH_PUBLIC_KEY, "This SSH key is already registered"),
    ] {
        let body = web_request(
            &server,
            &token,
            Method::POST,
            "/profile/ssh-keys",
            &[("ssh_key", key)],
        )
        .await?
        .text()
        .await?;
        ensure!(
            body.contains(refusal),
            "adding {key:?} was not refused: {body}"
        );
    }
    ensure!(
        stored_keys(&server, "keyowner").await? == vec![TEST_SSH_PUBLIC_KEY.to_owned()],
        "a refused key was stored"
    );

    let body = web_request(
        &server,
        &token,
        Method::POST,
        "/profile/ssh-keys",
        &[("ssh_key", SECOND_SSH_PUBLIC_KEY)],
    )
    .await?
    .text()
    .await?;
    ensure!(
        body.contains(r#"<div id="ssh-keys-list">"#) && body.contains("second"),
        "the added key was not listed: {body}"
    );
    ensure!(
        stored_keys(&server, "keyowner").await?.len() == 2,
        "the added key was not stored"
    );

    let body = web_request(&server, &token, Method::DELETE, "/profile/ssh-keys/5", &[])
        .await?
        .text()
        .await?;
    ensure!(
        body.contains("SSH key not found"),
        "an out-of-range delete was not refused: {body}"
    );
    web_request(&server, &token, Method::DELETE, "/profile/ssh-keys/0", &[]).await?;
    ensure!(
        stored_keys(&server, "keyowner").await? == vec![SECOND_SSH_PUBLIC_KEY.to_owned()],
        "the key at index 0 was not the one deleted"
    );
    Ok(())
}

#[tokio::test]
async fn account_keys_011_the_profile_page_stores_a_key_trimmed_and_refuses_it_again_with_another_comment()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "keyowner".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    let token = ws.login("keyowner", TEST_ADMIN_PASSWORD).await?;
    let key = format!("{TEST_SSH_PUBLIC_KEY} laptop");

    let padded = format!("  {key}\n");
    web_request(
        &server,
        &token,
        Method::POST,
        "/profile/ssh-keys",
        &[("ssh_key", padded.as_str())],
    )
    .await?;
    ensure!(
        stored_keys(&server, "keyowner").await? == vec![key.clone()],
        "the key was not stored trimmed"
    );

    let other_comment = format!("{TEST_SSH_PUBLIC_KEY} desktop");
    let body = web_request(
        &server,
        &token,
        Method::POST,
        "/profile/ssh-keys",
        &[("ssh_key", other_comment.as_str())],
    )
    .await?
    .text()
    .await?;
    ensure!(
        body.contains("This SSH key is already registered"),
        "the key with another comment was not refused: {body}"
    );
    ensure!(
        stored_keys(&server, "keyowner").await? == vec![key],
        "a refused key changed the account's keys"
    );
    Ok(())
}

#[tokio::test]
async fn account_keys_007_an_admin_removes_a_key_from_another_users_account() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let admin = ws.login_admin().await?;
    db::create_user(
        &server.db,
        "keyowner".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![
            TEST_SSH_PUBLIC_KEY.to_owned(),
            SECOND_SSH_PUBLIC_KEY.to_owned(),
        ],
    )
    .await?;
    let key_body = TEST_SSH_PUBLIC_KEY
        .split_whitespace()
        .nth(1)
        .unwrap_or_default();

    // The admin's page for the user lists the keys.
    let response = web_request(&server, &admin, Method::GET, "/admin/users/keyowner", &[]).await?;
    ensure!(
        response.status() == StatusCode::OK,
        "GET /admin/users/keyowner: {}",
        response.status()
    );
    let body = response.text().await?;
    ensure!(
        body.contains(r#"<div id="admin-ssh-keys-list""#) && body.contains(key_body),
        "the admin page did not render the SSH key list: {body}"
    );

    // Remove over RPC.
    let response = ws
        .rpc_call(
            "user.ssh_key.remove",
            json!({ "token": admin, "username": "keyowner", "ssh_key": SECOND_SSH_PUBLIC_KEY }),
        )
        .await?;
    ensure!(
        response["result"]["ssh_keys"] == json!([TEST_SSH_PUBLIC_KEY]),
        "an admin removing the user's key over RPC: {response}"
    );

    // Remove by fingerprint over REST.
    let fingerprint = ssh_key_fingerprint(TEST_SSH_PUBLIC_KEY)?;
    let response = Client::new()
        .delete(format!(
            "http://{}/api/v1/admin/users/keyowner/ssh-keys",
            server.addr
        ))
        .bearer_auth(&admin)
        .json(&json!({ "username": "keyowner", "ssh_key": fingerprint, "token": admin }))
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::OK,
        "REST remove returned {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await?;
    ensure!(body["ssh_keys"] == json!([]), "key not removed: {body}");
    ensure!(
        stored_keys(&server, "keyowner").await?.is_empty(),
        "the removed keys are still stored"
    );
    Ok(())
}

#[tokio::test]
async fn user_passwords_004_the_profile_password_change_needs_the_current_password() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "pwowner".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    let token = ws.login("pwowner", TEST_ADMIN_PASSWORD).await?;

    for (current, confirm, refusal) in [
        (
            "wrong-password",
            NEW_PASSWORD,
            "Current password is incorrect",
        ),
        (
            TEST_ADMIN_PASSWORD,
            "Something-Else-789!",
            "New passwords do not match",
        ),
    ] {
        let body = web_request(
            &server,
            &token,
            Method::POST,
            "/profile/password",
            &[
                ("current_password", current),
                ("new_password", NEW_PASSWORD),
                ("confirm_new_password", confirm),
            ],
        )
        .await?
        .text()
        .await?;
        ensure!(
            body.contains(refusal),
            "password change was not refused: {body}"
        );
    }
    ensure!(
        ws.login("pwowner", TEST_ADMIN_PASSWORD).await.is_ok(),
        "a refused change altered the password"
    );

    let body = web_request(
        &server,
        &token,
        Method::POST,
        "/profile/password",
        &[
            ("current_password", TEST_ADMIN_PASSWORD),
            ("new_password", NEW_PASSWORD),
            ("confirm_new_password", NEW_PASSWORD),
        ],
    )
    .await?
    .text()
    .await?;
    ensure!(
        body.contains("Password updated successfully"),
        "password change failed: {body}"
    );
    ensure!(
        ws.login("pwowner", NEW_PASSWORD).await.is_ok(),
        "the new password does not log in"
    );
    ensure!(
        ws.login("pwowner", TEST_ADMIN_PASSWORD).await.is_err(),
        "the old password still logs in"
    );
    Ok(())
}

// ── User deletion rules on every transport ──

/// IPv4-only lab networks for tests that seed lab records directly.
fn ipv4_lab_networks() -> db::LabNetworks {
    db::LabNetworks {
        management_network: "10.202.1.0/24".to_string(),
        gateway_ipv4: "10.202.1.1".to_string(),
        router_ipv4: "10.202.1.2".to_string(),
        management_network_v6: None,
        loopback_network_v6: None,
        gateway_ipv6: None,
        router_ipv6: None,
    }
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
async fn user_accounts_015_deleting_a_user_who_owns_labs_is_refused_on_all_transports() -> Result<()>
{
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
        &LabDetails::default(),
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

    ensure_web_refusal(
        web_request(&server, &token, Method::DELETE, "/admin/users/carol", &[]).await?,
        "c0ffee01",
    )
    .await?;

    ensure!(
        db::find_user(&server.db, "carol").await?.is_some(),
        "a refused delete removed the user"
    );
    ensure!(
        db::find_lab(&server.db, "c0ffee01").await?.is_some(),
        "a refused delete removed the user's lab"
    );

    // Once the lab is gone, the delete succeeds.
    db::delete_lab_from(&server.db, "c0ffee01", LabPhase::Creating).await?;
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
async fn user_accounts_014_self_delete_is_refused_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
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
        web_request(&server, &token, Method::DELETE, "/admin/users/dana", &[]).await?,
        "your own account",
    )
    .await?;

    ensure!(
        db::find_user(&server.db, "dana").await?.is_some(),
        "a refused self-delete removed the user"
    );
    Ok(())
}

#[tokio::test]
async fn builtin_admin_004_built_in_admin_delete_is_refused_on_all_transports() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    // A second admin, so the self-delete rule is not what refuses.
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
        .rpc_call(
            "user.delete",
            json!({ "token": token, "username": "sherpa-admin" }),
        )
        .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "RPC delete of admin was not refused: {response}"
    );

    let response = Client::new()
        .delete(format!(
            "http://{}/api/v1/admin/users/sherpa-admin",
            server.addr
        ))
        .bearer_auth(&token)
        .send()
        .await?;
    ensure!(
        response.status() == StatusCode::FORBIDDEN,
        "REST delete of admin returned {}",
        response.status()
    );

    ensure_web_refusal(
        web_request(
            &server,
            &token,
            Method::DELETE,
            "/admin/users/sherpa-admin",
            &[],
        )
        .await?,
        "built-in sherpa-admin account",
    )
    .await?;

    ensure!(
        db::find_user(&server.db, "sherpa-admin").await?.is_some(),
        "a refused delete removed the admin account"
    );
    Ok(())
}

#[tokio::test]
async fn builtin_admin_001_startup_is_refused_with_no_admin_account_and_no_password() -> Result<()>
{
    let db = empty_database().await?;

    let error = ensure_admin_account(&db, None).await.unwrap_err();
    ensure!(
        matches!(
            error.downcast_ref::<ServiceError>(),
            Some(ServiceError::AdminAccountMissing)
        ),
        "missing admin should be refused: {error}"
    );
    Ok(())
}

#[tokio::test]
async fn builtin_admin_002_startup_creates_admin_from_a_password_and_leaves_it_alone() -> Result<()>
{
    let db = empty_database().await?;

    ensure_admin_account(&db, Some(TEST_ADMIN_PASSWORD)).await?;
    let admin = db::find_user(&db, "sherpa-admin").await?;
    ensure!(
        admin.is_some_and(|admin| admin.is_admin),
        "SHERPA_ADMIN_PASSWORD should seed the admin account"
    );

    // Once the account exists, startup succeeds without the password.
    ensure_admin_account(&db, None).await?;
    Ok(())
}

#[tokio::test]
async fn account_keys_005_a_key_already_on_the_account_is_refused_even_with_another_comment()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "alice".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    let alice = ws.login("alice", TEST_ADMIN_PASSWORD).await?;
    let key = format!("{TEST_SSH_PUBLIC_KEY} alice@laptop");

    // Add over RPC; usernames are case-insensitive.
    let response = ws
        .rpc_call(
            "user.ssh_key.add",
            json!({ "token": alice, "username": "Alice", "ssh_key": key }),
        )
        .await?;
    ensure!(
        response["result"]["ssh_keys"] == json!([key]),
        "add failed: {response}"
    );

    // The same key data again, whatever its comment, is a duplicate.
    let response = ws
        .rpc_call(
            "user.ssh_key.add",
            json!({ "token": alice, "username": "alice", "ssh_key": TEST_SSH_PUBLIC_KEY }),
        )
        .await?;
    ensure!(
        response.get("error").is_some(),
        "duplicate accepted: {response}"
    );
    ensure!(
        stored_keys(&server, "alice").await? == vec![key],
        "a refused add changed alice's keys"
    );
    Ok(())
}

#[tokio::test]
async fn account_keys_006_a_key_is_removed_by_itself_or_its_fingerprint_and_a_missing_key_is_not_found()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let key = format!("{TEST_SSH_PUBLIC_KEY} alice@laptop");
    db::create_user(
        &server.db,
        "alice".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![key.clone(), SECOND_SSH_PUBLIC_KEY.to_owned()],
    )
    .await?;
    let alice = ws.login("alice", TEST_ADMIN_PASSWORD).await?;

    // The key itself, whatever its comment.
    let response = ws
        .rpc_call(
            "user.ssh_key.remove",
            json!({ "token": alice, "username": "alice", "ssh_key": TEST_SSH_PUBLIC_KEY }),
        )
        .await?;
    ensure!(
        response["result"]["ssh_keys"] == json!([SECOND_SSH_PUBLIC_KEY]),
        "removing a key by itself: {response}"
    );

    let fingerprint = ssh_key_fingerprint(SECOND_SSH_PUBLIC_KEY)?;
    let response = ws
        .rpc_call(
            "user.ssh_key.remove",
            json!({ "token": alice, "username": "alice", "ssh_key": fingerprint }),
        )
        .await?;
    ensure!(
        response["result"]["ssh_keys"] == json!([]),
        "removing a key by its fingerprint: {response}"
    );

    let response = ws
        .rpc_call(
            "user.ssh_key.remove",
            json!({ "token": alice, "username": "alice", "ssh_key": key }),
        )
        .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::NotFound),
        "removing a missing key: {response}"
    );
    Ok(())
}

#[tokio::test]
async fn account_keys_008_another_user_cannot_remove_a_key_from_an_account() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    for username in ["alice", "bob"] {
        db::create_user(
            &server.db,
            username.to_owned(),
            TEST_ADMIN_PASSWORD,
            false,
            if username == "alice" {
                vec![TEST_SSH_PUBLIC_KEY.to_owned()]
            } else {
                vec![]
            },
        )
        .await?;
    }
    let bob = ws.login("bob", TEST_ADMIN_PASSWORD).await?;

    let response = ws
        .rpc_call(
            "user.ssh_key.remove",
            json!({ "token": bob, "username": "alice", "ssh_key": TEST_SSH_PUBLIC_KEY }),
        )
        .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "bob changed alice's keys: {response}"
    );
    ensure!(
        stored_keys(&server, "alice").await? == vec![TEST_SSH_PUBLIC_KEY.to_owned()],
        "a refused removal changed alice's keys"
    );
    Ok(())
}

// ── Device password ──

/// A device password that meets the rule.
const DEVICE_PASSWORD: &str = "Granite-Owl42";
/// A login password that also meets the device password rule, to try as both.
const RULE_LOGIN_PASSWORD: &str = "Basalt-Heron7";
/// A login password the device password is never set with.
const WRONG_LOGIN_PASSWORD: &str = "Wrong-Pass-789!";

/// Whether `username` has a device password, as stored.
async fn device_password_is_set(server: &TestServer, username: &str) -> Result<bool> {
    let user = db::get_user(&server.db, username).await?;
    db::device_password_set(&server.db, &db::get_user_id(&user)?).await
}

/// Create `alice`, a user with `RULE_LOGIN_PASSWORD`, and `dana`, an admin, and log both in.
/// Returns their tokens.
async fn alice_and_dana(server: &TestServer, ws: &mut TestWsClient) -> Result<(String, String)> {
    db::create_user(
        &server.db,
        "alice".to_owned(),
        RULE_LOGIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    db::create_user(
        &server.db,
        "dana".to_owned(),
        TEST_ADMIN_PASSWORD,
        true,
        vec![],
    )
    .await?;
    let alice = ws.login("alice", RULE_LOGIN_PASSWORD).await?;
    let dana = ws.login("dana", TEST_ADMIN_PASSWORD).await?;
    Ok((alice, dana))
}

/// Set a device password over RPC, as the owner of `token`.
async fn rpc_set_device_password(
    ws: &mut TestWsClient,
    token: &str,
    username: &str,
    login_password: &str,
    device_password: &str,
) -> Result<serde_json::Value> {
    ws.rpc_call(
        "user.device_password.set",
        json!({ "token": token, "username": username, "login_password": login_password,
                "device_password": device_password }),
    )
    .await
}

/// Set a device password over REST, as the owner of `token`.
async fn rest_set_device_password(
    server: &TestServer,
    token: &str,
    username: &str,
    login_password: &str,
    device_password: &str,
) -> Result<Response> {
    Ok(Client::new()
        .post(format!(
            "http://{}/api/v1/admin/users/{username}/device-password",
            server.addr
        ))
        .bearer_auth(token)
        .json(
            &json!({ "username": username, "login_password": login_password,
                       "device_password": device_password, "token": token }),
        )
        .send()
        .await?)
}

/// Assert a REST device password request was answered with `status`.
async fn ensure_rest_status(response: Response, status: StatusCode) -> Result<()> {
    ensure!(
        response.status() == status,
        "expected {status}, got {}: {}",
        response.status(),
        response.text().await?
    );
    Ok(())
}

/// Submit the profile page's device password form as `token`, returning the page fragment.
async fn submit_profile_device_password(
    server: &TestServer,
    token: &str,
    login_password: &str,
    device_password: &str,
    confirm: &str,
) -> Result<String> {
    Ok(web_request(
        server,
        token,
        Method::POST,
        "/profile/device-password",
        &[
            ("login_password", login_password),
            ("device_password", device_password),
            ("confirm_device_password", confirm),
        ],
    )
    .await?
    .text()
    .await?)
}

#[tokio::test]
async fn device_password_002_nobody_sets_another_users_device_password_an_admin_included()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (_, dana) = alice_and_dana(&server, &mut ws).await?;

    // An admin, with their own login password or the user's, cannot set another user's.
    for login_password in [
        TEST_ADMIN_PASSWORD,
        RULE_LOGIN_PASSWORD,
        WRONG_LOGIN_PASSWORD,
    ] {
        let response =
            rpc_set_device_password(&mut ws, &dana, "alice", login_password, DEVICE_PASSWORD)
                .await?;
        ensure!(
            response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
            "an admin set alice's device password over RPC: {response}"
        );
        let response =
            rest_set_device_password(&server, &dana, "alice", login_password, DEVICE_PASSWORD)
                .await?;
        ensure_rest_status(response, StatusCode::FORBIDDEN).await?;
    }
    ensure!(
        !device_password_is_set(&server, "alice").await?,
        "a refused device password was stored"
    );
    Ok(())
}

#[tokio::test]
async fn device_password_003_a_wrong_login_password_is_refused_and_nothing_is_stored() -> Result<()>
{
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (alice, _) = alice_and_dana(&server, &mut ws).await?;

    let response = rpc_set_device_password(
        &mut ws,
        &alice,
        "alice",
        WRONG_LOGIN_PASSWORD,
        DEVICE_PASSWORD,
    )
    .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "a wrong login password was accepted over RPC: {response}"
    );
    let response = rest_set_device_password(
        &server,
        &alice,
        "alice",
        WRONG_LOGIN_PASSWORD,
        DEVICE_PASSWORD,
    )
    .await?;
    ensure_rest_status(response, StatusCode::FORBIDDEN).await?;
    ensure!(
        !device_password_is_set(&server, "alice").await?,
        "a refused device password was stored"
    );
    Ok(())
}

#[tokio::test]
async fn device_password_004_a_device_password_equal_to_the_login_password_is_refused() -> Result<()>
{
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (alice, _) = alice_and_dana(&server, &mut ws).await?;

    let response = rpc_set_device_password(
        &mut ws,
        &alice,
        "alice",
        RULE_LOGIN_PASSWORD,
        RULE_LOGIN_PASSWORD,
    )
    .await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::InvalidParams)
            && response.to_string().contains("must differ"),
        "the login password was accepted as the device password over RPC: {response}"
    );
    let response = rest_set_device_password(
        &server,
        &alice,
        "alice",
        RULE_LOGIN_PASSWORD,
        RULE_LOGIN_PASSWORD,
    )
    .await?;
    ensure_rest_status(response, StatusCode::BAD_REQUEST).await?;
    ensure!(
        !device_password_is_set(&server, "alice").await?,
        "a refused device password was stored"
    );
    Ok(())
}

#[tokio::test]
async fn device_password_010_account_details_and_the_user_list_carry_only_whether_it_is_set()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    db::create_user(
        &server.db,
        "alice".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    db::create_user(
        &server.db,
        "bob".to_owned(),
        TEST_ADMIN_PASSWORD,
        false,
        vec![],
    )
    .await?;
    db::create_user(
        &server.db,
        "dana".to_owned(),
        TEST_ADMIN_PASSWORD,
        true,
        vec![],
    )
    .await?;
    let alice = ws.login("alice", TEST_ADMIN_PASSWORD).await?;
    let bob = ws.login("bob", TEST_ADMIN_PASSWORD).await?;
    let dana = ws.login("dana", TEST_ADMIN_PASSWORD).await?;

    let response = rpc_set_device_password(
        &mut ws,
        &alice,
        "alice",
        TEST_ADMIN_PASSWORD,
        DEVICE_PASSWORD,
    )
    .await?;
    ensure!(
        response["result"]["device_password_set"] == json!(true),
        "alice could not set her device password: {response}"
    );

    // user.info and user.list say whether it is set, and never return it.
    let response = ws
        .rpc_call("user.info", json!({ "token": alice, "username": "alice" }))
        .await?;
    ensure!(
        response["result"]["user"]["device_password_set"] == json!(true),
        "user.info: {response}"
    );
    let response = ws.rpc_call("user.list", json!({ "token": dana })).await?;
    let set: Vec<(String, bool)> = response["result"]["users"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|user| {
            (
                user["username"].as_str().unwrap_or_default().to_owned(),
                user["device_password_set"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    ensure!(
        set.contains(&("alice".to_owned(), true)) && set.contains(&("bob".to_owned(), false)),
        "user.list: {response}"
    );
    ensure!(
        !response.to_string().contains(DEVICE_PASSWORD),
        "user.list returned the password"
    );

    // A device password set over REST shows in the REST account details the same way.
    let response =
        rest_set_device_password(&server, &bob, "bob", TEST_ADMIN_PASSWORD, DEVICE_PASSWORD)
            .await?;
    ensure_rest_status(response, StatusCode::OK).await?;
    let response = Client::new()
        .get(format!("http://{}/api/v1/admin/users/bob", server.addr))
        .bearer_auth(&bob)
        .send()
        .await?;
    let body: serde_json::Value = response.json().await?;
    ensure!(
        body["user"]["device_password_set"] == json!(true),
        "REST user info: {body}"
    );
    ensure!(
        !body.to_string().contains(DEVICE_PASSWORD),
        "REST user info returned the password"
    );
    Ok(())
}

#[tokio::test]
async fn device_password_012_the_profile_form_sets_the_users_own_and_refuses_the_rest() -> Result<()>
{
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let (alice, dana) = alice_and_dana(&server, &mut ws).await?;

    let body = web_request(&server, &alice, Method::GET, "/profile", &[])
        .await?
        .text()
        .await?;
    ensure!(
        body.contains(r#"hx-post="/profile/device-password""#) && body.contains(">not set</span>"),
        "the profile page has no device password form or state: {body}"
    );

    // The form names no user: an admin naming another user is refused, not applied.
    let response = web_request(
        &server,
        &dana,
        Method::POST,
        "/profile/device-password",
        &[
            ("username", "alice"),
            ("login_password", TEST_ADMIN_PASSWORD),
            ("device_password", DEVICE_PASSWORD),
            ("confirm_device_password", DEVICE_PASSWORD),
        ],
    )
    .await?;
    ensure!(
        response.status().is_client_error(),
        "a form naming another user was accepted: {}",
        response.status()
    );

    for (login_password, device_password, confirm, refusal) in [
        (
            WRONG_LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            DEVICE_PASSWORD,
            "the login password is incorrect",
        ),
        (
            RULE_LOGIN_PASSWORD,
            RULE_LOGIN_PASSWORD,
            RULE_LOGIN_PASSWORD,
            "must differ from your Sherpa login password",
        ),
        (
            RULE_LOGIN_PASSWORD,
            DEVICE_PASSWORD,
            "Granite-Owl43",
            "Device passwords do not match",
        ),
        (
            RULE_LOGIN_PASSWORD,
            "granite-owl42",
            "granite-owl42",
            "at least one uppercase letter",
        ),
    ] {
        let body = submit_profile_device_password(
            &server,
            &alice,
            login_password,
            device_password,
            confirm,
        )
        .await?;
        ensure!(
            body.contains(refusal),
            "the device password was not refused with {refusal:?}: {body}"
        );
    }
    ensure!(
        !device_password_is_set(&server, "alice").await?
            && !device_password_is_set(&server, "dana").await?,
        "a refused device password was stored"
    );

    let body = submit_profile_device_password(
        &server,
        &alice,
        RULE_LOGIN_PASSWORD,
        DEVICE_PASSWORD,
        DEVICE_PASSWORD,
    )
    .await?;
    ensure!(
        body.contains("Device password set") && body.contains(">set</span>"),
        "the device password was not set: {body}"
    );
    ensure!(
        device_password_is_set(&server, "alice").await?,
        "the device password was not stored"
    );
    let body = web_request(&server, &alice, Method::GET, "/profile", &[])
        .await?
        .text()
        .await?;
    ensure!(
        body.contains(">set</span>") && !body.contains(DEVICE_PASSWORD),
        "the profile page does not show the device password as set: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn only_the_user_adds_a_key_to_their_account_admins_included() -> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    for (username, is_admin) in [("alice", false), ("bob", false), ("dana", true)] {
        db::create_user(
            &server.db,
            username.to_owned(),
            TEST_ADMIN_PASSWORD,
            is_admin,
            vec![],
        )
        .await?;
    }
    let alice = ws.login("alice", TEST_ADMIN_PASSWORD).await?;
    let bob = ws.login("bob", TEST_ADMIN_PASSWORD).await?;
    let dana = ws.login("dana", TEST_ADMIN_PASSWORD).await?;

    // An admin and another user are refused, over RPC and REST, and alice's keys stay as they
    // were.
    for (caller, token) in [("dana", &dana), ("bob", &bob)] {
        let response = ws
            .rpc_call(
                "user.ssh_key.add",
                json!({ "token": token, "username": "alice", "ssh_key": SECOND_SSH_PUBLIC_KEY }),
            )
            .await?;
        ensure!(
            response["error"]["code"] == json!(RpcErrorCode::AccessDenied)
                && response["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message
                        .contains("only user 'alice' can do this for their own account")),
            "RPC add by {caller} to alice's account: {response}"
        );
        let response = Client::new()
            .post(format!(
                "http://{}/api/v1/admin/users/alice/ssh-keys",
                server.addr
            ))
            .bearer_auth(token)
            .json(&json!({ "username": "alice", "ssh_key": SECOND_SSH_PUBLIC_KEY, "token": token }))
            .send()
            .await?;
        ensure!(
            response.status() == StatusCode::FORBIDDEN,
            "REST add by {caller} to alice's account returned {}",
            response.status()
        );
    }
    // The admin page for a user has no form to add a key, and no route takes one.
    let response = web_request(
        &server,
        &dana,
        Method::POST,
        "/admin/users/alice/ssh-keys",
        &[("ssh_key", SECOND_SSH_PUBLIC_KEY)],
    )
    .await?;
    ensure!(
        !response.status().is_success(),
        "web add by an admin to alice's account returned {}",
        response.status()
    );
    let page = web_request(&server, &dana, Method::GET, "/admin/users/alice", &[])
        .await?
        .text()
        .await?;
    ensure!(
        !page.contains(r#"hx-post="/admin/users/alice/ssh-keys""#),
        "the admin page still offers to add a key: {page}"
    );
    ensure!(
        stored_keys(&server, "alice").await?.is_empty(),
        "a refused add changed alice's keys"
    );

    // Alice adds her own, and an admin adds to their own account.
    for (username, token) in [("alice", &alice), ("dana", &dana)] {
        let response = ws
            .rpc_call(
                "user.ssh_key.add",
                json!({ "token": token, "username": username, "ssh_key": SECOND_SSH_PUBLIC_KEY }),
            )
            .await?;
        ensure!(
            response["result"]["ssh_keys"] == json!([SECOND_SSH_PUBLIC_KEY]),
            "{username} adding their own key: {response}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn user_accounts_008_a_user_is_created_without_keys_and_a_request_with_keys_is_refused()
-> Result<()> {
    let server = TestServer::start().await?;
    let mut ws = TestWsClient::connect(&server).await?;
    let token = ws.login_admin().await?;

    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "keyed",
                "password": "TestUser123!",
                "is_admin": false,
                "ssh_keys": [TEST_SSH_PUBLIC_KEY],
            }),
        )
        .await?;
    ensure!(
        response.get("error").is_some(),
        "RPC user.create with keys was accepted: {response}"
    );
    let response = Client::new()
        .post(format!("http://{}/api/v1/admin/users", server.addr))
        .bearer_auth(&token)
        .json(&json!({
            "username": "keyed",
            "password": "TestUser123!",
            "is_admin": false,
            "ssh_keys": [TEST_SSH_PUBLIC_KEY],
            "token": token,
        }))
        .send()
        .await?;
    ensure!(
        response.status().is_client_error(),
        "REST user.create with keys returned {}",
        response.status()
    );
    ensure!(
        db::find_user(&server.db, "keyed").await?.is_none(),
        "a refused create made the user"
    );

    let response = ws
        .rpc_call(
            "user.create",
            json!({
                "token": token,
                "username": "keyless",
                "password": "TestUser123!",
                "is_admin": false,
            }),
        )
        .await?;
    ensure!(response.get("result").is_some(), "user.create: {response}");
    ensure!(
        stored_keys(&server, "keyless").await?.is_empty(),
        "a new user has keys"
    );
    Ok(())
}
