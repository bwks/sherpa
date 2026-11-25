use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use anyhow::{Context, Result};
use ipnet::{Ipv4Net, Ipv6Net};
use tracing::instrument;

use db::{Database, apply_schema, create_admin_user, create_user, find_user, list_users, open};
use libvirt::{BridgeNetwork, Qemu, SherpaStoragePool};
use shared::data::{Config, NodeModel};
use shared::konst::{
    ADMIN_USERNAME, SHERPA_BINS_PATH, SHERPA_CONTAINERS_PATH, SHERPA_DB_FILE_PATH,
    SHERPA_IMAGES_PATH, SHERPA_STORAGE_POOL, SHERPA_STORAGE_POOL_PATH, SHERPA_USER_NAME_ENV,
    SHERPA_USER_PASSWORD_ENV,
};
use shared::util::{
    create_dir, default_config, file_exists, normalize_username, term_msg_highlight,
    term_msg_surround, term_msg_underline, write_file_with_permissions,
};
use shared::validators::{validate_ssh_key, validate_username};

use crate::host_account::running_account_name;
use crate::konst::{
    SHERPA_ADMIN_PASSWORD_ENV, SHERPA_BLANK_DISK_DIR, SHERPA_CONFIG_FILE_PATH,
    SHERPA_DB_BACKUP_PATH, SHERPA_DB_PATH,
};
use crate::validate::{
    validate_management_prefix_ipv4, validate_management_prefix_ipv6, validate_service_account,
};

const SHERPA_BASE_DIR: &str = "/opt/sherpa";
const SHERPA_CONFIG_PATH: &str = "/opt/sherpa/config";
/// `sherpa.toml` can hold secrets: only `sherpa` may read it, whatever the umask of `init`.
const SHERPA_CONFIG_FILE_MODE: u32 = 0o600;
const SHERPA_BRIDGE_NETWORK_NAME: &str = "sherpa-bridge";
const SHERPA_BRIDGE_NETWORK_BRIDGE: &str = "br-sherpa0";
const SHERPA_USER_IS_ADMIN_ENV: &str = "SHERPA_USER_IS_ADMIN";
const SHERPA_USER_SSH_KEY_ENV: &str = "SHERPA_USER_SSH_KEY";

/// Set up a new server: its directories, `sherpa.toml`, libvirt network and storage pool,
/// database, admin account and first user. It runs as the `sherpa` service account only.
///
/// # Errors
///
/// Fails, before anything is written, when it does not run as the `sherpa` service account.
/// Fails when a management prefix has no room for a lab, a directory or file cannot be
/// written, libvirt cannot create the network or
/// pool, the database cannot be opened or migrated, or the first user's details are invalid or
/// cannot be read.
pub async fn init(
    force: bool,
    server_ipv4: Ipv4Addr,
    server_ipv6: Option<Ipv6Addr>,
    ws_port: u16,
    http_port: u16,
    management_prefix_ipv4: Ipv4Net,
    management_prefix_ipv6: Option<Ipv6Net>,
) -> Result<()> {
    validate_service_account(&running_account_name()?, "init")?;

    term_msg_surround("Sherpa Server Initializing");
    validate_management_prefix_ipv4(management_prefix_ipv4)?;
    if let Some(prefix) = management_prefix_ipv6 {
        validate_management_prefix_ipv6(prefix)?;
    }

    // Create server directories
    term_msg_highlight("Creating Directories");
    create_dir(SHERPA_BASE_DIR)?;
    create_dir(SHERPA_CONFIG_PATH)?;
    create_dir(SHERPA_CONTAINERS_PATH)?;
    create_dir(SHERPA_BINS_PATH)?;
    create_dir(SHERPA_DB_PATH)?;
    create_dir(SHERPA_DB_BACKUP_PATH)?;
    create_dir(SHERPA_IMAGES_PATH)?;
    create_dir(&format!("{SHERPA_IMAGES_PATH}/{SHERPA_BLANK_DISK_DIR}"))?;

    // Create image subdirectories for each node model
    term_msg_underline("Creating Node Image Directories");
    for model in NodeModel::to_vec() {
        create_dir(&format!("{SHERPA_IMAGES_PATH}/{model}"))?;
    }

    // Create container image directories from default config
    let config = default_config();
    for container_image in &config.container_images {
        create_dir(&format!(
            "{SHERPA_CONTAINERS_PATH}/{}",
            container_image.name
        ))?;
    }

    // Write server config
    if file_exists(SHERPA_CONFIG_FILE_PATH) && !force {
        println!("Config file already exists: {SHERPA_CONFIG_FILE_PATH}");
    } else {
        term_msg_underline("Writing Server Config");
        let config = server_config(
            server_ipv4,
            server_ipv6,
            ws_port,
            http_port,
            management_prefix_ipv4,
            management_prefix_ipv6,
        );
        write_server_config(&config, Path::new(SHERPA_CONFIG_FILE_PATH))?;
        println!("Config written to: {SHERPA_CONFIG_FILE_PATH}");
    }

    // Libvirt network and storage pool
    let qemu = Qemu::default();
    let qemu_conn = qemu.connect()?;

    term_msg_highlight("Creating Networks");
    println!("Creating network: {SHERPA_BRIDGE_NETWORK_NAME}");
    let bridge_network = BridgeNetwork {
        network_name: SHERPA_BRIDGE_NETWORK_NAME.to_owned(),
        bridge_name: SHERPA_BRIDGE_NETWORK_BRIDGE.to_owned(),
    };
    bridge_network.create(&qemu_conn)?;

    term_msg_highlight("Creating Storage Pools");
    println!("Creating storage pool: {SHERPA_STORAGE_POOL}");
    let storage_pool = SherpaStoragePool {
        name: SHERPA_STORAGE_POOL.to_owned(),
        path: SHERPA_STORAGE_POOL_PATH.to_owned(),
    };
    storage_pool.create(&qemu_conn)?;

    // Database initialization
    term_msg_highlight("Initializing Database");
    let db = open(Path::new(SHERPA_DB_FILE_PATH)).await?;
    println!("Database: {SHERPA_DB_FILE_PATH}");

    term_msg_underline("Applying Database Schema");
    apply_schema(&db).await?;
    println!("Database schema applied");

    create_admin_account(&db).await?;
    create_first_user(&db).await?;

    term_msg_surround("Sherpa Server Initialized");
    Ok(())
}

/// The default server config with the settings `init` takes.
fn server_config(
    server_ipv4: Ipv4Addr,
    server_ipv6: Option<Ipv6Addr>,
    ws_port: u16,
    http_port: u16,
    management_prefix_ipv4: Ipv4Net,
    management_prefix_ipv6: Option<Ipv6Net>,
) -> Config {
    Config {
        server_ipv4,
        server_ipv6,
        ws_port,
        http_port,
        management_prefix_ipv4,
        management_prefix_ipv6,
        ..default_config()
    }
}

/// Write the server config to `path` as TOML, at `SHERPA_CONFIG_FILE_MODE` whatever the umask.
/// It replaces any file there, and a file left wider by an older release gets the mode too.
#[instrument(skip(config), fields(path = %path.display()))]
fn write_server_config(config: &Config, path: &Path) -> Result<()> {
    let toml_string =
        toml::to_string_pretty(config).context("Unable to serialize server config")?;
    write_file_with_permissions(path, toml_string.as_bytes(), SHERPA_CONFIG_FILE_MODE)
        .context("Failed to write the server config")
}

/// Create the built-in `sherpa-admin` account unless it already exists.
///
/// The password comes from `SHERPA_ADMIN_PASSWORD` in the environment, or is prompted for. An
/// existing `sherpa-admin` is left untouched, so `init` is safe to re-run.
#[instrument(skip(db))]
async fn create_admin_account(db: &Database) -> Result<()> {
    term_msg_underline("Creating Admin Account");
    if find_user(db, ADMIN_USERNAME)
        .await
        .context("Failed to look up the admin account")?
        .is_some()
    {
        println!("Admin account '{ADMIN_USERNAME}' already exists");
        return Ok(());
    }

    let password = match env::var(SHERPA_ADMIN_PASSWORD_ENV).ok() {
        Some(password) => {
            println!("Using the admin password from {SHERPA_ADMIN_PASSWORD_ENV}");
            password
        }
        None => prompt_password(ADMIN_USERNAME)?,
    };
    create_admin_user(db, &password)
        .await
        .context("Failed to create the admin account")?;
    println!("Admin account '{ADMIN_USERNAME}' created");
    Ok(())
}

/// Create the first personal user account, unless one already exists.
///
/// Lab work is done from personal accounts; `sherpa-admin` cannot start labs. Each answer comes
/// from its `SHERPA_USER_*` environment variable when that is set, otherwise it is prompted for,
/// so `init` can run unattended. These are not read from the env file, which would keep the password.
#[instrument(skip(db))]
async fn create_first_user(db: &Database) -> Result<()> {
    term_msg_underline("Creating First User Account");
    let users = list_users(db)
        .await
        .context("Failed to list user accounts")?;
    if users.iter().any(|user| user.username != ADMIN_USERNAME) {
        println!("User accounts already exist, skipping");
        return Ok(());
    }

    println!("Labs are started from personal user accounts, not from '{ADMIN_USERNAME}'.");
    let username = match env::var(SHERPA_USER_NAME_ENV) {
        Ok(username) => {
            println!("Using the username from {SHERPA_USER_NAME_ENV}");
            username
        }
        Err(_) => prompt("Username")?,
    };
    let username = normalize_username(&username);
    validate_username(&username)?;
    let password = match env::var(SHERPA_USER_PASSWORD_ENV) {
        Ok(password) if password.is_empty() => {
            anyhow::bail!("{SHERPA_USER_PASSWORD_ENV} is set but empty")
        }
        Ok(password) => {
            println!("Using the password from {SHERPA_USER_PASSWORD_ENV}");
            password
        }
        Err(_) => prompt_password(&username)?,
    };
    let is_admin = match env::var(SHERPA_USER_IS_ADMIN_ENV) {
        Ok(answer) => {
            parse_yes_no(&answer, true).context(format!("Invalid {SHERPA_USER_IS_ADMIN_ENV}"))?
        }
        Err(_) => prompt_yes_no("Make this user an administrator?", true)?,
    };
    let ssh_keys = match env::var(SHERPA_USER_SSH_KEY_ENV) {
        Ok(answer) => {
            read_ssh_key(&answer).context(format!("Invalid {SHERPA_USER_SSH_KEY_ENV}"))?
        }
        Err(_) => prompt_ssh_key(&username)?,
    };

    if ssh_keys.is_empty() {
        println!(
            "No SSH key added. '{username}' needs one to start labs. Add it on the web UI Profile \
             page or with 'sherpa server user ssh-key add'."
        );
    }
    create_user(db, username.clone(), &password, is_admin, ssh_keys)
        .await
        .context(format!("Failed to create user '{username}'"))?;
    println!("User '{username}' created");
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read input")?;

    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{label} cannot be empty");
    }
    Ok(trimmed)
}

fn prompt_optional(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("Failed to read input")?;
    Ok(input.trim().to_string())
}

fn prompt_yes_no(question: &str, default: bool) -> Result<bool> {
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    parse_yes_no(&prompt_optional(&format!("{question} {hint}"))?, default)
}

/// A yes or no answer; an empty answer is the default.
fn parse_yes_no(answer: &str, default: bool) -> Result<bool> {
    match answer.trim().to_lowercase().as_str() {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        other => anyhow::bail!("Expected yes or no, got '{other}'"),
    }
}

/// Ask for an SSH public key as a file path or the key itself. Enter skips it.
fn prompt_ssh_key(username: &str) -> Result<Vec<String>> {
    read_ssh_key(&prompt_optional(&format!(
        "SSH public key for {username} (file path or key, Enter to skip)"
    ))?)
}

/// An SSH public key given as a file path or the key itself; an empty answer gives no key.
fn read_ssh_key(answer: &str) -> Result<Vec<String>> {
    let answer = answer.trim();
    if answer.is_empty() {
        return Ok(vec![]);
    }
    let key = if Path::new(answer).is_file() {
        fs::read_to_string(answer)
            .context(format!("Failed to read SSH public key file '{answer}'"))?
            .trim()
            .to_string()
    } else {
        answer.to_string()
    };
    validate_ssh_key(&key).context("Invalid SSH public key")?;
    Ok(vec![key])
}

fn prompt_password(username: &str) -> Result<String> {
    let password = rpassword::prompt_password(format!("Password for {username}: "))
        .context("Failed to read password")?;
    let confirm =
        rpassword::prompt_password("Confirm password: ").context("Failed to read password")?;

    if password != confirm {
        anyhow::bail!("Passwords do not match");
    }
    if password.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use nix::sys::stat::{Mode, umask};

    use super::*;

    const TEST_SSH_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIE6/8GjrZ1QD9nWLGeWkclvKEWliuCe2qDrak7cEmjIG dev";
    /// The umask the test runs its checks under when it is run again in a child process, in octal.
    const UMASK_ENV: &str = "SHERPA_TEST_INIT_UMASK";
    const UMASK_TEST: &str =
        "init::tests::server_file_modes_003_init_writes_the_config_0600_under_any_umask";

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// The umask is per process, so each one is set in a child process of this test: changing it
    /// here would change the modes of files other tests create meanwhile.
    #[test]
    fn server_file_modes_003_init_writes_the_config_0600_under_any_umask() {
        let Ok(mask) = env::var(UMASK_ENV) else {
            for mask in ["0000", "0002", "0022", "0077"] {
                let output = Command::new(env::current_exe().unwrap())
                    .args(["--exact", UMASK_TEST, "--test-threads=1"])
                    .env(UMASK_ENV, mask)
                    .output()
                    .unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    output.status.success() && stdout.contains("1 passed"),
                    "umask {mask}: {stdout}{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        umask(Mode::from_bits_truncate(
            u32::from_str_radix(&mask, 8).unwrap(),
        ));
        let config = default_config();
        let dir = tempfile::tempdir().unwrap();

        let new_file = dir.path().join("sherpa.toml");
        write_server_config(&config, &new_file).unwrap();
        assert_eq!(mode_of(&new_file), SHERPA_CONFIG_FILE_MODE, "new file");

        // `init --force` over a file an older release left at the umask's mode.
        let old_file = dir.path().join("old.toml");
        fs::write(&old_file, "old").unwrap();
        fs::set_permissions(&old_file, fs::Permissions::from_mode(0o664)).unwrap();
        write_server_config(&config, &old_file).unwrap();
        assert_eq!(mode_of(&old_file), SHERPA_CONFIG_FILE_MODE, "replaced file");
        assert_ne!(fs::read_to_string(&old_file).unwrap(), "old");
    }

    #[test]
    fn server_init_006_a_yes_or_no_answer_reads_in_any_case_with_a_default() {
        assert!(parse_yes_no("", true).unwrap());
        assert!(!parse_yes_no("", false).unwrap());
        assert!(parse_yes_no("Yes", false).unwrap());
        assert!(parse_yes_no("y", false).unwrap());
        assert!(!parse_yes_no("NO", true).unwrap());
        assert!(!parse_yes_no("n", true).unwrap());
        assert!(parse_yes_no("maybe", true).is_err());
    }

    #[test]
    fn server_init_007_the_ssh_key_answer_is_a_key_a_file_or_nothing() {
        assert_eq!(read_ssh_key(TEST_SSH_KEY).unwrap(), vec![TEST_SSH_KEY]);
        assert_eq!(read_ssh_key("").unwrap(), [] as [String; 0]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ed25519.pub");
        fs::write(&path, format!("{TEST_SSH_KEY}\n")).unwrap();
        assert_eq!(
            read_ssh_key(path.to_str().unwrap()).unwrap(),
            vec![TEST_SSH_KEY]
        );
    }
}
