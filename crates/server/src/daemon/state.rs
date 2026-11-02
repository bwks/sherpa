use anyhow::{Context, Result, bail};
use bollard::Docker;
use dashmap::DashMap;
use libvirt::Qemu;
use shared::data::{Config, UpRequest};
use shared::konst::{ADMIN_USERNAME, SHERPA_ADMIN_PASSWORD_ENV, SHERPA_DB_FILE_PATH};
use std::env;
use std::path::Path;
use std::sync::Arc;
use tracing::instrument;

use crate::api::websocket::connection::ConnectionRegistry;
use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::daemon::metrics::Metrics;
use crate::daemon::storage::secure_lab_storage;
use crate::validate::validate_tailscale_gateway_settings;

/// The type of operation a job represents.
pub enum JobType {
    Create { request: UpRequest },
    Destroy { lab_id: String },
}

/// A pending job waiting for an SSE stream handler to pick it up.
pub struct Job {
    pub lab_name: String,
    pub job_type: JobType,
    /// The user who created the job. The job runs as this user, and only they or an admin
    /// may view or stream it.
    pub requester: AuthContext,
}

impl Job {
    /// Whether `ctx` may view or stream this job.
    pub fn can_view(&self, ctx: &AuthContext) -> bool {
        ctx.can_access(&self.requester.username)
    }
}

/// Application state shared across the server.
///
/// This contains all runtime state needed by handlers, including:
/// - WebSocket connection registry for real-time communication
/// - Database connection (SurrealDB)
/// - libvirt client (for VMs and unikernels)
/// - Docker client (for containers)
/// - Sherpa configuration
/// - JWT secret for authentication
#[derive(Clone)]
pub struct AppState {
    /// Registry of active WebSocket connections
    pub connections: ConnectionRegistry,
    /// SurrealDB connection
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
    /// Keyed by job_id, consumed once by the stream handler.
    pub pending_jobs: Arc<DashMap<String, Job>>,
}

impl AppState {
    /// Create a new AppState with all infrastructure connections
    #[instrument(skip(config, metrics))]
    pub async fn new(config: Config, metrics: Metrics) -> Result<Self> {
        validate_tailscale_gateway_settings(&config.tailscale)
            .context("Invalid server Tailscale settings")?;
        // Load or generate JWT secret
        let jwt_secret =
            jwt::load_or_generate_secret().context("Failed to load or generate JWT secret")?;
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

        secure_lab_storage();

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
        })
    }
}

/// Make sure the built-in `admin` account exists before the server starts.
///
/// With `admin_password` (from `SHERPA_ADMIN_PASSWORD`) a missing account is created; without
/// it, a missing account is an error, so the server never runs without its recovery account.
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

    #[test]
    fn jobs_are_visible_to_their_creator_and_admins_only() {
        let job = Job {
            lab_name: "lab".to_owned(),
            job_type: JobType::Destroy {
                lab_id: "lab1".to_owned(),
            },
            requester: AuthContext::new("alice".to_owned(), false),
        };
        assert!(job.can_view(&AuthContext::new("alice".to_owned(), false)));
        assert!(job.can_view(&AuthContext::new("admin".to_owned(), true)));
        assert!(!job.can_view(&AuthContext::new("bob".to_owned(), false)));
    }
}
