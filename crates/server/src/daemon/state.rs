use anyhow::{Context, Result, bail};
use bollard::Docker;
use dashmap::DashMap;
use libvirt::Qemu;
use shared::data::{Config, UpRequest};
use shared::konst::{ADMIN_USERNAME, SHERPA_DB_FILE_PATH, SHERPA_USERNAME};
use std::env;
use std::path::Path;
use std::sync::Arc;
use tracing::instrument;

use crate::api::websocket::connection::{ConnectionRegistry, create_registry};
use crate::auth::context::AuthContext;
use crate::auth::jwt;
use crate::auth::session::{self, SessionLookup};
use crate::daemon::console_sessions::ConsoleSessions;
use crate::daemon::db_key::load_or_create_db_key;
use crate::daemon::image_folders::move_renamed_model_folders;
use crate::daemon::link_states::LinkStates;
use crate::daemon::metrics::Metrics;
use crate::daemon::storage::secure_storage;
use crate::daemon::unreadable_ends::UnreadableEnds;
use crate::daemon::vault_seals::VaultSeals;
use crate::konst::SHERPA_ADMIN_PASSWORD_ENV;
use crate::services::image_lifecycle::BackedVolumes;
use crate::validate::{check_host_memory_settings, validate_tailscale_gateway_settings};

/// The key that seals the database's secret values: 32 bytes, mode 0600, beside the JWT secret
/// in the `sherpa`-only (0700) secrets directory. It is never in the database, its backups or
/// the database directory, and is backed up separately from them.
const SHERPA_DB_KEY_PATH: &str = "/opt/sherpa/.secret/db.key";
const JWT_SECRET_PATH: &str = "/opt/sherpa/.secret/jwt.secret";

/// The type of operation a job represents.
pub enum JobType {
    Create { request: UpRequest },
    Destroy { lab_id: String },
}

/// A pending job waiting for an SSE stream handler to pick it up.
pub struct Job {
    pub lab_name: String,
    pub job_type: JobType,
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
    pub connections: ConnectionRegistry,
    pub db: db::Database,
    pub qemu: Arc<Qemu>,
    pub docker: Arc<Docker>,
    pub config: Arc<Config>,
    pub jwt_secret: Arc<Vec<u8>>,
    pub metrics: Metrics,
    pub pending_jobs: Arc<DashMap<String, Job>>,
    pub link_states: LinkStates,
    pub unreadable_ends: UnreadableEnds,
    pub vault_seals: VaultSeals,
    pub console_sessions: ConsoleSessions,
}

impl AppState {
    /// Create a new `AppState` with all infrastructure connections
    ///
    /// # Errors
    ///
    /// Fails when the Tailscale gateway or host memory settings are invalid, the JWT secret cannot
    /// be loaded or generated, the database cannot be opened or migrated, the database key is
    /// missing, damaged or not the database's (see [`load_or_create_db_key`] and
    /// [`db::verify_secret_key`]), the admin account is missing (see
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

        // The key that seals the database's secret values, checked against the database.
        let registered = db::secret_key_registered(&db).await?;
        let db_key = load_or_create_db_key(Path::new(SHERPA_DB_KEY_PATH), registered)?;
        let db = db.with_secret_key(db_key);
        db::verify_secret_key(&db).await?;

        let admin_password = env::var(SHERPA_ADMIN_PASSWORD_ENV).ok();
        ensure_admin_account(&db, admin_password.as_deref()).await?;
        warn_on_sherpa_logins(&db).await?;

        secure_storage();

        // Initialize libvirt client
        let qemu = Qemu::default();
        let backing_files = qemu
            .backed_volumes()
            .map(|volumes| {
                volumes
                    .into_iter()
                    .map(|volume| volume.backing_file)
                    .collect::<Vec<_>>()
            })
            .inspect_err(|error| {
                tracing::warn!(
                    error = %format!("{error:#}"),
                    "Cannot read the storage pool's disks"
                );
            })
            .ok();
        move_renamed_model_folders(Path::new(&config.images_dir), backing_files.as_deref())
            .context("Failed to move the image folders of renamed node models")?;
        tracing::info!("Initialized libvirt/QEMU client");

        // Initialize Docker client
        let docker =
            Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

        tracing::info!("Connected to Docker daemon");

        Ok(Self {
            connections: create_registry(),
            db,
            qemu: Arc::new(qemu),
            docker: Arc::new(docker),
            config: Arc::new(config),
            jwt_secret: Arc::new(jwt_secret),
            metrics,
            pending_jobs: Arc::new(DashMap::new()),
            link_states: LinkStates::default(),
            unreadable_ends: UnreadableEnds::default(),
            vault_seals: VaultSeals::default(),
            console_sessions: ConsoleSessions::default(),
        })
    }
}

/// Make sure the built-in `sherpa-admin` account exists before the server starts.
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
                "The built-in sherpa-admin account does not exist"
            );
            bail!(
                "The built-in '{ADMIN_USERNAME}' account does not exist. Run 'sudo -u sherpa -H \
                 sherpad init', or set {SHERPA_ADMIN_PASSWORD_ENV} to create it on startup"
            );
        }
        tracing::debug!("Admin account exists");
    }
    Ok(())
}

/// Warn, once at startup, when nodes log in as `sherpa`: they have the
/// published password and their lab's key until they are redeployed. Returns how many there are.
///
/// # Errors
///
/// Fails when the database cannot be read.
#[instrument(skip(db))]
pub async fn warn_on_sherpa_logins(db: &db::Database) -> Result<usize> {
    let count = db::count_nodes_by_ssh_username(db, SHERPA_USERNAME)
        .await
        .context("Failed to count nodes that log in as sherpa")?;
    if count > 0 {
        tracing::warn!(
            count,
            "{count} lab nodes log in as '{SHERPA_USERNAME}' with its \
             published password. Redeploy or recreate them to give them their owner's login"
        );
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    use shared::data::{ModelKind, NodeModel, SessionChannel};

    use crate::auth::session::SessionRejection;
    use crate::log_capture::capture_log;

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
    async fn user_sessions_022_a_job_runs_as_its_requester_only_while_their_session_lasts() {
        {
            let (db, _, job) = job_queued_by_carol(false).await;
            match job.resolve_requester(&db).await.expect("resolve") {
                SessionLookup::Active(ctx) => assert_eq!(ctx.username, "carol"),
                other @ SessionLookup::Rejected(_) => {
                    panic!("expected an active requester, got {other:?}")
                }
            }
        }
        {
            let (db, _, job) = job_queued_by_carol(false).await;
            session::logout(&db, &job.requester.session_id)
                .await
                .expect("logout");
            assert!(matches!(
                job.resolve_requester(&db).await.expect("resolve"),
                SessionLookup::Rejected(SessionRejection::Ended)
            ));
        }
        {
            let (db, carol, job) = job_queued_by_carol(false).await;
            db::delete_user(&db, db::get_user_id(&carol).expect("id"))
                .await
                .expect("delete");
            assert!(matches!(
                job.resolve_requester(&db).await.expect("resolve"),
                SessionLookup::Rejected(SessionRejection::Ended)
            ));
        }
        {
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

    /// A database with one lab holding `logins` nodes, each logging in as the given user.
    async fn database_with_node_logins(logins: &[&str]) -> db::Database {
        let db = db::open_in_memory().await.expect("open database");
        db::apply_schema(&db).await.expect("apply schema");
        let user = db::create_user(&db, "alice".to_owned(), "TestPass123!", false, vec![])
            .await
            .expect("user");
        let networks = db::LabNetworks {
            management_network: "172.31.1.0/24".to_owned(),
            gateway_ipv4: "172.31.1.1".to_owned(),
            router_ipv4: "172.31.1.2".to_owned(),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        };
        let lab = db::create_lab(
            &db,
            "Old",
            "lab-0001",
            &user,
            &networks,
            &db::LabDetails::default(),
        )
        .await
        .expect("lab");
        let image = db::upsert_node_image(
            &db,
            ModelKind::default_for(NodeModel::UbuntuLinux).node_config(),
            db::DefaultRequest::Keep,
        )
        .await
        .expect("image");
        for (index, login) in (1u16..).zip(logins) {
            let node = db::create_node(
                &db,
                &format!("node{index}"),
                index,
                image.id.clone().expect("image id"),
                lab.id.clone().expect("lab id"),
            )
            .await
            .expect("node");
            db::update_node_ssh_username(&db, node.id.expect("node id"), Some(login))
                .await
                .expect("login");
        }
        db
    }

    #[test]
    fn upgraded_labs_003_sherpad_warns_with_the_count_of_nodes_that_log_in_as_sherpa() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let (count, output) = capture_log(|| {
            runtime.block_on(async {
                let db = database_with_node_logins(&["sherpa", "alice", "sherpa"]).await;
                warn_on_sherpa_logins(&db).await.expect("count")
            })
        });
        assert_eq!(count, 2);
        assert!(output.contains("WARN"), "{output}");
        assert!(output.contains("count=2"), "{output}");
        assert!(output.contains("Redeploy or recreate them"), "{output}");

        let (count, output) = capture_log(|| {
            runtime.block_on(async {
                let db = database_with_node_logins(&["alice"]).await;
                warn_on_sherpa_logins(&db).await.expect("count")
            })
        });
        assert_eq!(count, 0);
        assert!(!output.contains("WARN"), "{output}");
        assert!(!output.contains("Redeploy"), "{output}");
    }
}
