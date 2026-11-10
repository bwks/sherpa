//! Login sessions: their states, the events that move them, and the operations behind each.
//!
//! Every token names a session (its `sid` claim), and every session is a row in the `session`
//! table. A token is accepted only while its session is [`SessionState::Active`]: the row exists
//! and has not expired. The states and the events that move a session between them are defined
//! here, in [`transition`], and nowhere else:
//!
//! | From | Event | To | Where |
//! |---|---|---|---|
//! | none | login | `Active` | [`start`] |
//! | `Active` | use | `Active` | [`lookup`], on every request and when a web job starts |
//! | `Expired` | use | refused: expired | [`lookup`] |
//! | `Ended` | use | refused: ended | [`lookup`] |
//! | `Active`, `Expired` | logout | `Ended` | [`logout`] |
//! | `Active`, `Expired` | password changed | `Ended`, except the caller's own session | `db::change_user_password`, from `services::user_password` |
//! | `Active`, `Expired` | user deleted | `Ended` | `db::delete_user`; the foreign key cascades |
//! | `Expired` | sweep | `Ended` | [`sweep`], from [`run_sweeper`] |
//! | `Active` | time passes `expires_at` | `Expired` | none; [`state_of`] reads it from the clock |
//!
//! `Ended` is the absence of the row, so ending is idempotent, and a user re-created with the same
//! name never sees an old account's sessions: they belong to the deleted row's id.

use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result};
use db::Database;
use jiff::{SignedDuration, Timestamp};
use shared::auth::jwt::Claims;
use shared::data::{DbSession, DbUser, SessionChannel, ValidateResponse};
use tokio::time::{self, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::auth::context::AuthContext;
use crate::auth::jwt;

/// Where a session is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// The row exists and `expires_at` is in the future: its token is accepted.
    Active,
    /// The row exists but `expires_at` has passed; the sweep will delete it.
    Expired,
    /// There is no row: logged out, ended by a password change or user deletion, or swept.
    Ended,
}

/// Why a session was ended on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Logout,
    PasswordChanged,
    UserDeleted,
}

/// Something that happens to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// A request, or a web job starting, presents the session.
    Use,
    /// The session is ended.
    End(EndReason),
    /// The periodic sweep runs.
    Sweep,
}

/// Why a session cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRejection {
    Expired,
    Ended,
}

impl fmt::Display for SessionRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expired => f.write_str("the session has expired"),
            Self::Ended => f.write_str("the session has ended"),
        }
    }
}

/// The result of presenting a session.
#[derive(Debug, Clone)]
pub enum SessionLookup {
    /// The session is active; the context carries the user's current name and admin flag.
    Active(AuthContext),
    Rejected(SessionRejection),
}

/// A session just started, and the token that names it.
#[derive(Debug, Clone)]
pub struct IssuedSession {
    pub token: String,
    pub ctx: AuthContext,
    /// Unix seconds.
    pub issued_at: i64,
    /// Unix seconds; the token's `exp` and the row's `expires_at`.
    pub expires_at: i64,
}

/// The state of a session whose row is `row` (`None` when there is none), at `now`.
#[must_use]
pub fn state_of(row: Option<&DbSession>, now: Timestamp) -> SessionState {
    match row {
        None => SessionState::Ended,
        Some(session) if session.expires_at <= now => SessionState::Expired,
        Some(_) => SessionState::Active,
    }
}

/// The state a session in state `from` moves to on `event`.
///
/// # Errors
///
/// A [`SessionEvent::Use`] of a session that is not active is refused, with the reason.
pub fn transition(
    from: SessionState,
    event: SessionEvent,
) -> Result<SessionState, SessionRejection> {
    match (from, event) {
        (SessionState::Active, SessionEvent::Use | SessionEvent::Sweep) => Ok(SessionState::Active),
        (SessionState::Expired, SessionEvent::Use) => Err(SessionRejection::Expired),
        (SessionState::Ended, SessionEvent::Use) => Err(SessionRejection::Ended),
        (_, SessionEvent::End(_))
        | (SessionState::Expired | SessionState::Ended, SessionEvent::Sweep) => {
            Ok(SessionState::Ended)
        }
    }
}

/// Start a session for `user` through `channel`, lasting `lifetime_seconds`, and sign its token.
///
/// # Errors
///
/// Fails when the user has no id, the row cannot be written or the token cannot be signed.
#[instrument(skip(db, secret, user), fields(username = %user.username, ?channel))]
pub async fn start(
    db: &Database,
    secret: &[u8],
    user: &DbUser,
    channel: SessionChannel,
    lifetime_seconds: i64,
) -> Result<IssuedSession> {
    let issued_at = Timestamp::now().as_second();
    let expires_at = Timestamp::from_second(issued_at)
        .and_then(|now| now.checked_add(SignedDuration::from_secs(lifetime_seconds)))
        .context("Session lifetime is out of range")?;
    let session = db::create_session(db, db::get_user_id(user)?, channel, expires_at)
        .await
        .context("Failed to start a session")?;
    let claims = Claims::new(
        user.username.clone(),
        session.id.clone(),
        user.is_admin,
        issued_at,
        session.expires_at.as_second(),
    );
    let token = jwt::create_token(secret, &claims)?;
    tracing::info!(
        event = "auth.session_started",
        username = %user.username,
        ?channel,
        expires_at = claims.exp,
        "Session started"
    );
    Ok(IssuedSession {
        token,
        ctx: AuthContext::new(user.username.clone(), user.is_admin, session.id),
        issued_at,
        expires_at: claims.exp,
    })
}

/// Present session `session_id`: the [`SessionEvent::Use`] transition.
///
/// # Errors
///
/// Fails only when the database cannot be read; a session that cannot be used is
/// [`SessionLookup::Rejected`].
#[instrument(level = "debug", skip(db, session_id))]
pub async fn lookup(db: &Database, session_id: &str) -> Result<SessionLookup> {
    let found = db::find_session(db, session_id)
        .await
        .context("Failed to look up the session")?;
    let state = state_of(found.as_ref().map(|found| &found.session), Timestamp::now());
    Ok(match (transition(state, SessionEvent::Use), found) {
        (Ok(SessionState::Active), Some(found)) => SessionLookup::Active(AuthContext::new(
            found.username,
            found.is_admin,
            found.session.id,
        )),
        (Err(rejection), _) => SessionLookup::Rejected(rejection),
        // `transition` accepts a use only of an active session, which has a row.
        (Ok(_), _) => SessionLookup::Rejected(SessionRejection::Ended),
    })
}

/// Whether `token` is valid, and whose it is: the answer to `auth.validate`, over RPC or REST.
///
/// A token is valid when its signature and expiry check out and its session is active.
///
/// # Errors
///
/// Fails only when the database cannot be read.
#[instrument(level = "debug", skip_all)]
pub async fn token_status(db: &Database, secret: &[u8], token: &str) -> Result<ValidateResponse> {
    let invalid = ValidateResponse {
        valid: false,
        username: None,
        is_admin: None,
        expires_at: None,
    };
    let claims = match jwt::validate_token(secret, token) {
        Ok(claims) => claims,
        Err(error) => {
            tracing::debug!(error = format!("{error:#}"), "Token validation failed");
            return Ok(invalid);
        }
    };
    Ok(match lookup(db, &claims.sid).await? {
        SessionLookup::Active(ctx) => ValidateResponse {
            valid: true,
            username: Some(ctx.username),
            is_admin: Some(ctx.is_admin),
            expires_at: Some(claims.exp),
        },
        SessionLookup::Rejected(rejection) => {
            tracing::debug!(%rejection, "Token's session cannot be used");
            invalid
        }
    })
}

/// End session `session_id`: the [`EndReason::Logout`] transition. Returns whether the session
/// was still there; logging out of an ended session is not an error.
///
/// # Errors
///
/// Fails when the database cannot be written; the session is then still active.
#[instrument(skip(db, session_id))]
pub async fn logout(db: &Database, session_id: &str) -> Result<bool> {
    let ended = db::delete_session(db, session_id)
        .await
        .context("Failed to end the session")?;
    tracing::info!(event = "auth.session_ended", reason = ?EndReason::Logout, ended, "Logged out");
    Ok(ended)
}

/// Delete the sessions that have expired by `now`: the [`SessionEvent::Sweep`] transition.
/// Returns how many were deleted.
///
/// # Errors
///
/// Fails when the database cannot be written.
#[instrument(level = "debug", skip(db))]
pub async fn sweep(db: &Database, now: Timestamp) -> Result<usize> {
    db::delete_expired_sessions(db, now)
        .await
        .context("Failed to sweep expired sessions")
}

/// Sweep expired sessions at once, then every `interval`, until `cancel` fires.
///
/// Expired sessions are already refused by [`lookup`]; the sweep only keeps the table from
/// growing. A failed sweep is logged and retried at the next interval.
#[instrument(skip(db, cancel))]
pub async fn run_sweeper(db: Database, cancel: CancellationToken, interval: Duration) {
    let mut ticks = time::interval(interval);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                tracing::info!("Session sweeper stopped");
                return;
            }
            _ = ticks.tick() => match sweep(&db, Timestamp::now()).await {
                Ok(0) => {}
                Ok(swept) => tracing::info!(event = "auth.sessions_swept", swept, "Deleted expired sessions"),
                Err(error) => tracing::warn!(error = format!("{error:#}"), "Session sweep failed"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::RecordId;
    use tokio::time::{Instant, sleep, timeout};

    const SECRET: &[u8] = b"test_secret_32_bytes_long_enough";
    const PASSWORD: &str = "Original-Pass-123!";
    const DAY: i64 = 24 * 60 * 60;

    const STATES: [SessionState; 3] = [
        SessionState::Active,
        SessionState::Expired,
        SessionState::Ended,
    ];
    const EVENTS: [SessionEvent; 5] = [
        SessionEvent::Use,
        SessionEvent::End(EndReason::Logout),
        SessionEvent::End(EndReason::PasswordChanged),
        SessionEvent::End(EndReason::UserDeleted),
        SessionEvent::Sweep,
    ];

    async fn database() -> Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        db
    }

    async fn user(db: &Database, username: &str) -> DbUser {
        db::create_user(db, username.to_owned(), PASSWORD, false, vec![])
            .await
            .expect("create user")
    }

    async fn state_now(db: &Database, session_id: &str) -> SessionState {
        let found = db::find_session(db, session_id).await.expect("find");
        state_of(found.as_ref().map(|found| &found.session), Timestamp::now())
    }

    fn row(expires_at: Timestamp) -> DbSession {
        DbSession {
            id: "s".to_owned(),
            user: RecordId::new("user", "u"),
            channel: SessionChannel::Cli,
            created_at: Timestamp::UNIX_EPOCH,
            expires_at,
        }
    }

    // ── The machine ──

    #[test]
    fn the_transition_table() {
        use SessionRejection as R;
        use SessionState::{Active, Ended, Expired};
        let expected = |from, event| match (from, event) {
            (Active, SessionEvent::Use | SessionEvent::Sweep) => Ok(Active),
            (Expired, SessionEvent::Use) => Err(R::Expired),
            (Ended, SessionEvent::Use) => Err(R::Ended),
            _ => Ok(Ended),
        };
        for from in STATES {
            for event in EVENTS {
                assert_eq!(
                    transition(from, event),
                    expected(from, event),
                    "{from:?} on {event:?}"
                );
            }
        }
    }

    #[test]
    fn only_an_active_session_can_be_used() {
        for from in STATES {
            assert_eq!(
                transition(from, SessionEvent::Use).is_ok(),
                from == SessionState::Active,
                "{from:?}"
            );
        }
    }

    #[test]
    fn every_end_reason_ends_every_state() {
        for from in STATES {
            for reason in [
                EndReason::Logout,
                EndReason::PasswordChanged,
                EndReason::UserDeleted,
            ] {
                assert_eq!(
                    transition(from, SessionEvent::End(reason)),
                    Ok(SessionState::Ended)
                );
            }
        }
    }

    #[test]
    fn state_follows_the_row_and_the_clock() {
        let now = Timestamp::from_second(1_000).expect("time");
        let later = Timestamp::from_second(1_001).expect("time");
        assert_eq!(state_of(None, now), SessionState::Ended);
        assert_eq!(state_of(Some(&row(later)), now), SessionState::Active);
        assert_eq!(state_of(Some(&row(now)), now), SessionState::Expired);
        assert_eq!(state_of(Some(&row(now)), later), SessionState::Expired);
    }

    // ── Login and use ──

    #[tokio::test]
    async fn a_new_session_is_active_and_its_token_names_it() {
        let db = database().await;
        let alice = user(&db, "alice").await;

        let issued = start(&db, SECRET, &alice, SessionChannel::Cli, DAY)
            .await
            .expect("start");

        let claims = jwt::validate_token(SECRET, &issued.token).expect("token");
        assert_eq!(claims.sid, issued.ctx.session_id);
        assert_eq!(claims.exp, issued.expires_at);
        assert_eq!(claims.exp - claims.iat, DAY);
        match lookup(&db, &claims.sid).await.expect("lookup") {
            SessionLookup::Active(ctx) => {
                assert_eq!(ctx.username, "alice");
                assert_eq!(ctx.session_id, claims.sid);
            }
            other @ SessionLookup::Rejected(_) => {
                panic!("expected an active session, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn an_unknown_session_is_ended() {
        let db = database().await;
        assert!(matches!(
            lookup(&db, "no-such-session").await.expect("lookup"),
            SessionLookup::Rejected(SessionRejection::Ended)
        ));
    }

    #[tokio::test]
    async fn an_expired_session_is_refused_before_it_is_swept() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let issued = start(&db, SECRET, &alice, SessionChannel::Web, -1)
            .await
            .expect("start");
        assert!(matches!(
            lookup(&db, &issued.ctx.session_id).await.expect("lookup"),
            SessionLookup::Rejected(SessionRejection::Expired)
        ));
    }

    #[tokio::test]
    async fn a_lookup_reads_the_current_admin_flag() {
        let db = database().await;
        let carol = db::create_user(&db, "carol".to_owned(), PASSWORD, true, vec![])
            .await
            .expect("create");
        let issued = start(&db, SECRET, &carol, SessionChannel::Web, DAY)
            .await
            .expect("start");
        let mut demoted = carol.clone();
        demoted.is_admin = false;
        db::update_user(&db, demoted).await.expect("demote");

        match lookup(&db, &issued.ctx.session_id).await.expect("lookup") {
            SessionLookup::Active(ctx) => assert!(!ctx.is_admin),
            other @ SessionLookup::Rejected(_) => {
                panic!("expected an active session, got {other:?}")
            }
        }
    }

    // ── M14.1: logout and password change end sessions ──

    #[tokio::test]
    async fn a_token_used_after_logout_is_refused() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let web = start(&db, SECRET, &alice, SessionChannel::Web, DAY)
            .await
            .expect("start");
        let cli = start(&db, SECRET, &alice, SessionChannel::Cli, DAY)
            .await
            .expect("start");

        assert!(logout(&db, &web.ctx.session_id).await.expect("logout"));

        assert_eq!(
            state_now(&db, &web.ctx.session_id).await,
            transition(SessionState::Active, SessionEvent::End(EndReason::Logout))
                .expect("transition")
        );
        let status = token_status(&db, SECRET, &web.token).await.expect("status");
        assert!(!status.valid, "a logged-out token is still valid");
        assert!(
            token_status(&db, SECRET, &cli.token)
                .await
                .expect("status")
                .valid,
            "logout ended another session"
        );
        // Logging out again is not an error.
        assert!(!logout(&db, &web.ctx.session_id).await.expect("logout"));
    }

    #[tokio::test]
    async fn a_token_used_after_a_password_change_is_refused() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let current = start(&db, SECRET, &alice, SessionChannel::Web, DAY)
            .await
            .expect("start");
        let other = start(&db, SECRET, &alice, SessionChannel::Cli, DAY)
            .await
            .expect("start");

        db::change_user_password(
            &db,
            db::get_user_id(&alice).expect("id"),
            "new-hash",
            Some(&current.ctx.session_id),
        )
        .await
        .expect("change");

        assert_eq!(
            state_now(&db, &other.ctx.session_id).await,
            transition(
                SessionState::Active,
                SessionEvent::End(EndReason::PasswordChanged)
            )
            .expect("transition")
        );
        assert!(
            !token_status(&db, SECRET, &other.token)
                .await
                .expect("status")
                .valid
        );
        assert!(
            token_status(&db, SECRET, &current.token)
                .await
                .expect("status")
                .valid
        );
    }

    // ── M14.2: a deleted user's token ──

    #[tokio::test]
    async fn a_token_of_a_deleted_user_does_not_work_for_a_new_user_of_the_same_name() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let issued = start(&db, SECRET, &alice, SessionChannel::Cli, DAY)
            .await
            .expect("start");

        db::delete_user(&db, db::get_user_id(&alice).expect("id"))
            .await
            .expect("delete");
        assert_eq!(
            state_now(&db, &issued.ctx.session_id).await,
            transition(
                SessionState::Active,
                SessionEvent::End(EndReason::UserDeleted)
            )
            .expect("transition")
        );
        user(&db, "alice").await;

        assert!(matches!(
            lookup(&db, &issued.ctx.session_id).await.expect("lookup"),
            SessionLookup::Rejected(SessionRejection::Ended)
        ));
        assert!(
            !token_status(&db, SECRET, &issued.token)
                .await
                .expect("status")
                .valid
        );
    }

    // ── The sweep ──

    #[tokio::test]
    async fn the_sweep_ends_exactly_the_sessions_the_machine_calls_expired() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let expired = start(&db, SECRET, &alice, SessionChannel::Web, -1)
            .await
            .expect("start");
        let live = start(&db, SECRET, &alice, SessionChannel::Web, DAY)
            .await
            .expect("start");
        let now = Timestamp::now();
        let mut before = Vec::new();
        for issued in [&expired, &live] {
            let found = db::find_session(&db, &issued.ctx.session_id)
                .await
                .expect("find");
            before.push(state_of(found.as_ref().map(|found| &found.session), now));
        }
        assert_eq!(before, [SessionState::Expired, SessionState::Active]);

        assert_eq!(sweep(&db, now).await.expect("sweep"), 1);

        for (issued, state) in [(&expired, before[0]), (&live, before[1])] {
            assert_eq!(
                state_now(&db, &issued.ctx.session_id).await,
                transition(state, SessionEvent::Sweep).expect("transition")
            );
        }
    }

    #[tokio::test]
    async fn the_sweeper_sweeps_at_start_and_stops_when_cancelled() {
        let db = database().await;
        let alice = user(&db, "alice").await;
        let expired = start(&db, SECRET, &alice, SessionChannel::Web, -1)
            .await
            .expect("start");
        let cancel = CancellationToken::new();
        let sweeper = tokio::spawn(run_sweeper(
            db.clone(),
            cancel.clone(),
            Duration::from_secs(3600),
        ));

        let deadline = Instant::now() + Duration::from_secs(10);
        while db::find_session(&db, &expired.ctx.session_id)
            .await
            .expect("find")
            .is_some()
        {
            assert!(
                Instant::now() < deadline,
                "the sweeper did not sweep at start"
            );
            sleep(Duration::from_millis(20)).await;
        }
        cancel.cancel();
        timeout(Duration::from_secs(5), sweeper)
            .await
            .expect("the sweeper did not stop")
            .expect("the sweeper panicked");
    }
}
