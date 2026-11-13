use anyhow::{Context, Result, bail};
use bollard::Docker;
use dashmap::DashMap;
use libvirt::Qemu;
use shared::data::{Config, UpRequest};
use shared::konst::{
    ADMIN_USERNAME, JWT_SECRET_PATH, SHERPA_ADMIN_PASSWORD_ENV, SHERPA_DB_FILE_PATH,
};
use std::env;
use std::path::Path;
use std::sync::Arc;
use tracing::instrument;

use crate::api::websocket::connection::ConnectionRegistry;
use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::auth::session::{self, SessionLookup};
use crate::daemon::link_states::LinkStates;
use crate::daemon::metrics::Metrics;
use crate::daemon::storage::secure_storage;
use crate::validate::{check_host_memory_settings, validate_tailscale_gateway_settings};

/// The type of operation a job represents.
pub enum JobType {
    Create { request: UpRequest },
    Destroy { lab_id: String },
}

/// A pending job waiting for an SSE stream handler to pick it up.
pub struct Job {
    pub lab_name: String,
    pub job_type: JobType,
    /// The user who created the job, and the session they created it through. Only they or an
    /// admin may view or stream it. It runs as the requester's session resolved again when it
    /// starts ([`Job::resolve_requester`]), not as this snapshot.
    pub requester: AuthContext,
}

impl Job {
    /// Whether `ctx` may view or stream this job.
    #[must_use]
    pub fn can_view(&self, ctx: &AuthContext) -> bool {
        ctx.can_access(&self.requester.username)
    }

    /// The requester as they are now, when the job starts: their session presented again
    /// (`session::lookup`). A requester who logged out, changed their password elsewhere or was
    /// deleted since queuing the job is refused, and one whose admin flag changed runs with the
    /// new value.
    ///
    /// # Errors
    ///
    /// Fails when the database cannot be read.
    #[instrument(skip(self, db), fields(requester = %self.requester.username))]
    pub async fn resolve_requester(&self, db: &db::Database) -> Result<SessionLookup> {
        session::lookup(db, &self.requester.session_id).await
    }
}

/// Application state shared across the server.
///
/// This contains all runtime state needed by handlers, including:
/// - WebSocket connection registry for real-time communication
/// - Embedded database handle (Turso)
/// - libvirt client (for VMs and unikernels)
/// - Docker client (for containers)
/// - Sherpa configuration
/// - JWT secret for authentication
#[derive(Clone)]
pub struct AppState {
    /// Registry of active WebSocket connections
    pub connections: ConnectionRegistry,
    /// Embedded database handle
    pub db: db::Database,
    /// libvirt/QEMU client
    pub qemu: Arc<Qemu>,
    /// Docker client
    pub docker: Arc<Docker>,
    /// Sherpa configuration
    pub config: Arc<Config>,
    /// JWT secret for token creation and validation
    pub jwt_secret: Arc<Vec<u8>>,
    /// OpenTelemetry metrics instruments
    pub metrics: Metrics,
    /// Pending jobs awaiting SSE stream pickup.
    /// Keyed by `job_id`, consumed once by the stream handler.
    pub pending_jobs: Arc<DashMap<String, Job>>,
    /// The last known state of each lab's P2p links, for `inspect`.
    pub link_states: LinkStates,
}

impl AppState {
    /// Create a new `AppState` with all infrastructure connections
    ///
    /// # Errors
    ///
    /// Fails when the Tailscale gateway or host memory settings are invalid, the JWT secret cannot
    /// be loaded or generated, the database cannot be opened or migrated, the admin account is missing (see
    /// [`ensure_admin_account`]), or Docker cannot be reached.
    #[instrument(skip(config, metrics))]
    pub async fn new(config: Config, metrics: Metrics) -> Result<Self> {
        validate_tailscale_gateway_settings(&config.tailscale)
            .context("Invalid server Tailscale settings")?;
        check_host_memory_settings(config.host_memory)
            .context("Invalid server host memory settings")?;
        // Load or generate JWT secret
        let jwt_secret = jwt::load_or_generate_secret(Path::new(JWT_SECRET_PATH))
            .context("Failed to load or generate JWT secret")?;
        tracing::info!("JWT secret loaded successfully");

        // Open the embedded database
        let db = db::open(Path::new(SHERPA_DB_FILE_PATH))
            .await
            .context("Failed to open the database")?;

        tracing::info!("Opened database at {}", SHERPA_DB_FILE_PATH);

        // Apply database schema
        db::apply_schema(&db)
            .await
            .context("Failed to apply database schema")?;
        tracing::debug!("Database schema applied");

        let admin_password = env::var(SHERPA_ADMIN_PASSWORD_ENV).ok();
        ensure_admin_account(&db, admin_password.as_deref()).await?;

        secure_storage();

        // Initialize libvirt client
        let qemu = Qemu::default();
        tracing::info!("Initialized libvirt/QEMU client");

        // Initialize Docker client
        let docker =
            Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

        tracing::info!("Connected to Docker daemon");

        Ok(Self {
            connections: crate::api::websocket::connection::create_registry(),
            db,
            qemu: Arc::new(qemu),
            docker: Arc::new(docker),
            config: Arc::new(config),
            jwt_secret: Arc::new(jwt_secret),
            metrics,
            pending_jobs: Arc::new(DashMap::new()),
            link_states: LinkStates::default(),
        })
    }
}

/// Make sure the built-in `admin` account exists before the server starts.
///
/// With `admin_password` (from `SHERPA_ADMIN_PASSWORD`) a missing account is created; without
/// it, a missing account is an error, so the server never runs without its recovery account.
///
/// # Errors
///
/// Fails when the account is missing and no password is given, or the database cannot be read
/// or written.
#[instrument(skip(db, admin_password))]
pub async fn ensure_admin_account(db: &db::Database, admin_password: Option<&str>) -> Result<()> {
    if let Some(admin_password) = admin_password {
        match db::seed_admin_user(db, admin_password).await {
            Ok(true) => {
                tracing::info!(username = ADMIN_USERNAME, "Admin account created");
            }
            Ok(false) => {
                tracing::debug!("Admin account seeding skipped (admin account already exists)");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to create admin account");
                return Err(e).context("Admin account creation failed");
            }
        }
    } else {
        let admin = db::find_user(db, ADMIN_USERNAME)
            .await
            .context("Failed to look up the admin account")?;
        if admin.is_none() {
            tracing::error!(
                username = ADMIN_USERNAME,
                "The built-in admin account does not exist"
            );
            bail!(
                "The built-in '{ADMIN_USERNAME}' account does not exist. Run 'sherpad init', \
                 or set {SHERPA_ADMIN_PASSWORD_ENV} to create it on startup"
            );
        }
        tracing::debug!("Admin account exists");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::SessionChannel;

    use crate::auth::session::SessionRejection;

    #[test]
    fn jobs_are_visible_to_their_creator_and_admins_only() {
        let job = Job {
            lab_name: "lab".to_owned(),
            job_type: JobType::Destroy {
                lab_id: "lab1".to_owned(),
            },
            requester: AuthContext::for_test("alice", false),
        };
        assert!(job.can_view(&AuthContext::for_test("alice", false)));
        assert!(job.can_view(&AuthContext::for_test("admin", true)));
        assert!(!job.can_view(&AuthContext::for_test("bob", false)));
    }

    const SECRET: &[u8] = b"test_secret_32_bytes_long_enough";
    const PASSWORD: &str = "Original-Pass-123!";

    /// A database holding `carol`, and a destroy job she queued through a new web session.
    async fn job_queued_by_carol(is_admin: bool) -> (db::Database, db::DbUser, Job) {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        let carol = db::create_user(&db, "carol".to_owned(), PASSWORD, is_admin, vec![])
            .await
            .expect("create user");
        let issued = session::start(&db, SECRET, &carol, SessionChannel::Web, 3600)
            .await
            .expect("start session");
        let job = Job {
            lab_name: "lab".to_owned(),
            job_type: JobType::Destroy {
                lab_id: "lab1".to_owned(),
            },
            requester: issued.ctx,
        };
        (db, carol, job)
    }

    #[tokio::test]
    async fn a_job_runs_as_its_requester_when_the_session_is_active() {
        let (db, _, job) = job_queued_by_carol(false).await;
        match job.resolve_requester(&db).await.expect("resolve") {
            SessionLookup::Active(ctx) => assert_eq!(ctx.username, "carol"),
            other @ SessionLookup::Rejected(_) => {
                panic!("expected an active requester, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn a_job_does_not_start_after_its_requester_logged_out() {
        let (db, _, job) = job_queued_by_carol(false).await;
        session::logout(&db, &job.requester.session_id)
            .await
            .expect("logout");
        assert!(matches!(
            job.resolve_requester(&db).await.expect("resolve"),
            SessionLookup::Rejected(SessionRejection::Ended)
        ));
    }

    #[tokio::test]
    async fn a_job_does_not_start_after_its_requester_was_deleted() {
        let (db, carol, job) = job_queued_by_carol(false).await;
        db::delete_user(&db, db::get_user_id(&carol).expect("id"))
            .await
            .expect("delete");
        assert!(matches!(
            job.resolve_requester(&db).await.expect("resolve"),
            SessionLookup::Rejected(SessionRejection::Ended)
        ));
    }

    #[tokio::test]
    async fn a_job_runs_with_the_requesters_current_rights() {
        let (db, carol, job) = job_queued_by_carol(true).await;
        assert!(job.requester.is_admin);
        let mut demoted = carol.clone();
        demoted.is_admin = false;
        db::update_user(&db, demoted).await.expect("demote");

        match job.resolve_requester(&db).await.expect("resolve") {
            SessionLookup::Active(ctx) => assert!(!ctx.is_admin, "the job kept admin rights"),
            other @ SessionLookup::Rejected(_) => {
                panic!("expected an active requester, got {other:?}")
            }
        }
    }
}
