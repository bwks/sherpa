//! The account lockout: failed logins in a row lock an account until an admin unlocks it.

use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Result, ensure};
use reqwest::{Client, StatusCode, header};
use serde_json::{Value, json};
use shared::error::RpcErrorCode;
use sherpad::auth::cookies::AUTH_COOKIE_NAME;

use crate::helpers::http_client::TestHttpClient;
use crate::helpers::test_server::TestServer;
use crate::helpers::ws_client::TestWsClient;

const BOB_PASSWORD: &str = "Bob-Original-Pass-1!";
const CAROL_PASSWORD: &str = "Carol-Original-Pass-1!";
const WRONG_PASSWORD: &str = "Wrong-Pass-789!";
const REFUSED: &str = "username and/or password is incorrect";
/// Failed logins in a row that lock an account.
const LIMIT: usize = 3;

/// A server with the non-admin users `bob` and `carol`.
async fn server_with_users() -> Result<TestServer> {
    let server = TestServer::start().await?;
    db::create_user(&server.db, "bob".to_owned(), BOB_PASSWORD, false, vec![]).await?;
    db::create_user(
        &server.db,
        "carol".to_owned(),
        CAROL_PASSWORD,
        false,
        vec![],
    )
    .await?;
    Ok(server)
}

/// The response to an RPC login, on a connection of its own.
async fn rpc_login(server: &TestServer, username: &str, password: &str) -> Result<Value> {
    let mut ws = TestWsClient::connect(server).await?;
    ws.rpc_call(
        "auth.login",
        json!({ "username": username, "password": password }),
    )
    .await
}

/// Whether an RPC login with `password` gives a token.
async fn logs_in(server: &TestServer, username: &str, password: &str) -> Result<bool> {
    Ok(rpc_login(server, username, password).await?["result"]["token"].is_string())
}

/// Fail `count` RPC logins of `username` in a row.
async fn fail_logins(server: &TestServer, username: &str, count: usize) -> Result<()> {
    for _ in 0..count {
        let response = rpc_login(server, username, WRONG_PASSWORD).await?;
        ensure!(response["result"].is_null(), "{response}");
    }
    Ok(())
}

async fn is_locked(server: &TestServer, username: &str) -> Result<bool> {
    Ok(db::list_locked_usernames(&server.db)
        .await?
        .iter()
        .any(|locked| locked == username))
}

/// The admin's RPC token.
async fn admin_token(server: &TestServer) -> Result<String> {
    TestWsClient::connect(server).await?.login_admin().await
}

/// The status and body of a REST login.
async fn rest_login(server: &TestServer, username: &str, password: &str) -> Result<(u16, String)> {
    let response = TestHttpClient::new(server.addr)
        .post_json(
            "/api/v1/auth/login",
            &json!({ "username": username, "password": password }),
        )
        .await?;
    Ok((response.status().as_u16(), response.text().await?))
}

/// The status, whether a cookie was set, and the body of a web form login.
async fn web_login(
    server: &TestServer,
    username: &str,
    password: &str,
) -> Result<(u16, bool, String)> {
    let response = TestHttpClient::new(server.addr)
        .post_form("/login", &[("username", username), ("password", password)])
        .await?;
    let cookie = response.headers().contains_key(header::SET_COOKIE);
    Ok((response.status().as_u16(), cookie, response.text().await?))
}

/// A web request with `token` as the session cookie.
async fn web_post(server: &TestServer, token: &str, path: &str) -> Result<reqwest::Response> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .post(format!("http://{}{path}", server.addr))
        .header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"))
        .send()
        .await?)
}

async fn rest_unlock(server: &TestServer, token: &str, username: &str) -> Result<StatusCode> {
    let mut client = TestHttpClient::new(server.addr);
    client.set_token(token.to_owned());
    Ok(client
        .post_json(
            &format!("/api/v1/admin/users/{username}/unlock"),
            &json!({}),
        )
        .await?
        .status())
}

async fn rpc_unlock(server: &TestServer, token: &str, username: &str) -> Result<Value> {
    TestWsClient::connect(server)
        .await?
        .rpc_call(
            "user.unlock",
            json!({ "username": username, "token": token }),
        )
        .await
}

#[tokio::test]
async fn user_login_016_the_third_failed_login_in_a_row_locks_the_account() -> Result<()> {
    let server = server_with_users().await?;

    fail_logins(&server, "bob", LIMIT - 1).await?;
    ensure!(!is_locked(&server, "bob").await?, "locked after two");
    fail_logins(&server, "bob", 1).await?;
    ensure!(is_locked(&server, "bob").await?, "not locked after three");
    ensure!(
        !is_locked(&server, "carol").await?,
        "another account was locked"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_017_a_successful_login_sets_the_count_back_to_zero() -> Result<()> {
    let server = server_with_users().await?;

    fail_logins(&server, "bob", LIMIT - 1).await?;
    ensure!(
        logs_in(&server, "bob", BOB_PASSWORD).await?,
        "right password refused"
    );
    fail_logins(&server, "bob", LIMIT - 1).await?;
    ensure!(!is_locked(&server, "bob").await?, "the count was not reset");
    ensure!(
        logs_in(&server, "bob", BOB_PASSWORD).await?,
        "right password refused"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_018_a_locked_account_is_refused_with_the_right_password() -> Result<()> {
    let server = server_with_users().await?;
    fail_logins(&server, "bob", LIMIT).await?;

    for _ in 0..=LIMIT {
        let response = rpc_login(&server, "bob", BOB_PASSWORD).await?;
        ensure!(
            response["result"].is_null(),
            "a locked account logged in: {response}"
        );
        ensure!(
            response["error"]["message"] == REFUSED,
            "refusal: {response}"
        );
    }
    ensure!(is_locked(&server, "bob").await?, "the account was unlocked");
    Ok(())
}

#[tokio::test]
async fn user_login_019_every_refusal_gets_the_same_answer_on_every_path() -> Result<()> {
    let server = server_with_users().await?;
    fail_logins(&server, "carol", LIMIT).await?;
    // A wrong password for bob, an unknown username, and carol's locked account.
    let refusals = [
        ("bob", WRONG_PASSWORD),
        ("nobody", BOB_PASSWORD),
        ("carol", CAROL_PASSWORD),
    ];

    // Bob's wrong password is the first failure in a row on each path.
    db::unlock_user(&server.db, "bob").await?;
    let mut rpc = Vec::new();
    for (username, password) in refusals {
        rpc.push(rpc_login(&server, username, password).await?["error"].clone());
    }
    ensure!(
        rpc.iter()
            .all(|error| *error == json!({ "code": -32001, "message": REFUSED })),
        "RPC: {rpc:?}"
    );

    // Bob's wrong password is the first failure in a row on each path.
    db::unlock_user(&server.db, "bob").await?;
    let mut rest = Vec::new();
    for (username, password) in refusals {
        rest.push(rest_login(&server, username, password).await?);
    }
    ensure!(
        rest.iter().all(|answer| *answer == rest[0]),
        "REST: {rest:?}"
    );
    ensure!(
        rest[0].0 == 401 && rest[0].1.contains(REFUSED),
        "REST: {rest:?}"
    );

    // Bob's wrong password is the first failure in a row on each path.
    db::unlock_user(&server.db, "bob").await?;
    let mut web = Vec::new();
    for (username, password) in refusals {
        web.push(web_login(&server, username, password).await?);
    }
    ensure!(web.iter().all(|answer| *answer == web[0]), "web: {web:?}");
    ensure!(
        web[0].0 == 200 && !web[0].1 && web[0].2.contains(REFUSED),
        "web: {web:?}"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_020_failed_logins_at_the_same_time_check_no_more_than_three_passwords()
-> Result<()> {
    let server = server_with_users().await?;
    let addr = server.addr;

    let attempts: Vec<_> = (0..4 * LIMIT)
        .map(|_| {
            tokio::spawn(async move {
                TestHttpClient::new(addr)
                    .post_json(
                        "/api/v1/auth/login",
                        &json!({ "username": "bob", "password": WRONG_PASSWORD }),
                    )
                    .await
                    .map(|response| response.status())
            })
        })
        .collect();
    for attempt in attempts {
        let status = attempt.await??;
        ensure!(status == StatusCode::UNAUTHORIZED, "{status}");
    }

    ensure!(is_locked(&server, "bob").await?, "not locked");
    let bob = db::get_user(&server.db, "bob").await?;
    let stored = db::list_login_attempts(&server.db, db::get_user_id(&bob)?).await?;
    ensure!(stored.len() == LIMIT, "stored attempts: {stored:?}");
    ensure!(
        !logs_in(&server, "bob", BOB_PASSWORD).await?,
        "the right password got through"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_021_each_attempt_is_stored_with_its_result_and_ipv4_address_on_every_path()
-> Result<()> {
    let server = server_with_users().await?;
    let started = jiff::Timestamp::now();

    ensure!(logs_in(&server, "bob", BOB_PASSWORD).await?, "RPC login");
    let (status, _) = rest_login(&server, "bob", WRONG_PASSWORD).await?;
    ensure!(status == 401, "REST login: {status}");
    let (_, cookie, _) = web_login(&server, "bob", BOB_PASSWORD).await?;
    ensure!(cookie, "web login");

    let bob = db::get_user(&server.db, "bob").await?;
    let bob_id = db::get_user_id(&bob)?;
    let stored = db::list_login_attempts(&server.db, bob_id.clone()).await?;
    let results: Vec<bool> = stored.iter().map(|attempt| attempt.succeeded).collect();
    ensure!(results == [true, false, true], "{stored:?}");
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    for attempt in &stored {
        ensure!(
            attempt.user == bob_id
                && attempt.address == loopback
                && attempt.attempted_at >= started
                && attempt.attempted_at <= jiff::Timestamp::now(),
            "{attempt:?}"
        );
    }
    let carol = db::get_user(&server.db, "carol").await?;
    ensure!(
        db::list_login_attempts(&server.db, db::get_user_id(&carol)?)
            .await?
            .is_empty(),
        "another user's attempts were stored"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_025_an_admin_unlocks_an_account_with_the_users_page_button() -> Result<()> {
    let server = server_with_users().await?;
    fail_logins(&server, "bob", LIMIT).await?;
    let token = admin_token(&server).await?;

    let page = Client::new()
        .get(format!("http://{}/admin/users", server.addr))
        .header(header::COOKIE, format!("{AUTH_COOKIE_NAME}={token}"))
        .send()
        .await?
        .text()
        .await?;
    ensure!(
        page.contains(r#"hx-post="/admin/users/bob/unlock""#),
        "no Unlock button for bob: {page}"
    );
    ensure!(
        !page.contains(r#"hx-post="/admin/users/carol/unlock""#),
        "an Unlock button for an account that is not locked"
    );

    let response = web_post(&server, &token, "/admin/users/bob/unlock").await?;
    ensure!(response.status() == StatusCode::OK, "{}", response.status());
    ensure!(!is_locked(&server, "bob").await?, "still locked");
    ensure!(
        logs_in(&server, "bob", BOB_PASSWORD).await?,
        "bob cannot log in"
    );
    Ok(())
}

#[tokio::test]
async fn user_login_026_an_admin_unlocks_an_account_over_rpc_and_rest() -> Result<()> {
    let server = server_with_users().await?;
    let token = admin_token(&server).await?;

    fail_logins(&server, "bob", LIMIT).await?;
    let response = rpc_unlock(&server, &token, "bob").await?;
    ensure!(response["result"]["success"] == true, "{response}");
    ensure!(logs_in(&server, "bob", BOB_PASSWORD).await?, "RPC unlock");

    fail_logins(&server, "bob", LIMIT).await?;
    let status = rest_unlock(&server, &token, "bob").await?;
    ensure!(status == StatusCode::OK, "{status}");
    ensure!(logs_in(&server, "bob", BOB_PASSWORD).await?, "REST unlock");

    let response = rpc_unlock(&server, &token, "nobody").await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::NotFound),
        "{response}"
    );
    let status = rest_unlock(&server, &token, "nobody").await?;
    ensure!(status == StatusCode::NOT_FOUND, "{status}");
    Ok(())
}

#[tokio::test]
async fn user_login_027_a_user_who_is_not_an_admin_is_refused_unlock() -> Result<()> {
    let server = server_with_users().await?;
    fail_logins(&server, "carol", LIMIT).await?;
    let bob = TestWsClient::connect(&server)
        .await?
        .login("bob", BOB_PASSWORD)
        .await?;

    let response = rpc_unlock(&server, &bob, "carol").await?;
    ensure!(
        response["error"]["code"] == json!(RpcErrorCode::AccessDenied),
        "{response}"
    );
    let status = rest_unlock(&server, &bob, "carol").await?;
    ensure!(status == StatusCode::FORBIDDEN, "REST: {status}");
    let status = web_post(&server, &bob, "/admin/users/carol/unlock")
        .await?
        .status();
    ensure!(status == StatusCode::FORBIDDEN, "web: {status}");
    ensure!(is_locked(&server, "carol").await?, "carol was unlocked");
    Ok(())
}

#[tokio::test]
async fn user_login_028_an_admin_password_reset_does_not_unlock_the_account() -> Result<()> {
    let server = server_with_users().await?;
    fail_logins(&server, "bob", LIMIT).await?;
    let token = admin_token(&server).await?;
    let new_password = "Bob-Reset-Pass-2!";

    let response = TestWsClient::connect(&server)
        .await?
        .rpc_call(
            "user.passwd",
            json!({ "username": "bob", "new_password": new_password, "token": token }),
        )
        .await?;
    ensure!(response["result"]["success"] == true, "{response}");
    ensure!(is_locked(&server, "bob").await?, "the reset unlocked bob");
    ensure!(
        !logs_in(&server, "bob", new_password).await?,
        "bob logged in with the new password"
    );
    Ok(())
}
