//! The bus itself: a NATS server with `JetStream`, in a container on the host.
//!
//! It uses the host's network and listens on one address, such as a lab's management bridge, so
//! only that lab's guests and the host reach it. Every agent authenticates with an `NKey`, and the
//! server config that says what each may do is rendered from the agents file and the registry
//! of enrolled guests (see `config` and `registry`).
//!
//! The bus takes TLS only, handshake first, with a server certificate from its own CA (see
//! `tls`); `start` makes the CA once and issues the certificate when it needs to.
//!
//! `enroll` and `unenroll` change the registry, render the config again and send the server
//! SIGHUP. nats-server reloads its users on SIGHUP: a client whose user is gone is disconnected
//! and refused, and every other client stays connected.

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::path::{self, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerStateStatusEnum, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, InspectContainerOptions,
    KillContainerOptionsBuilder,
};
use container::{
    container_status, docker_connection, remove_container, start_container, stop_container,
};
use futures_util::TryStreamExt;

use crate::agents::Agents;
use crate::channel::{Channel, Endpoint};
use crate::config;
use crate::files::standard_state_dir;
use crate::keys::{self, public_key_of};
use crate::message::check_agent_name;
use crate::registry::{self, Registry};
use crate::state::{self, BusRecord, LabRecord};
use crate::tls::{self, TLS_DIR, TlsFiles};

/// The bus container.
const CONTAINER_NAME: &str = "sherpa-agent-bus";
/// `nats:2.15-alpine`, nats-server 2.15.2.
const IMAGE: &str = "nats@sha256:afae2fdbe2e326c65e1246cb83b9d05d319188734f3bd577ac8072daae78a10f";
/// The volume holding `JetStream`'s stored messages; it outlives the container.
const VOLUME: &str = "sherpa-agent-bus-data";
const DATA_DIR: &str = "/data";
/// The directory, in the state directory, that holds the server config. It is bind-mounted
/// read-only at `CONFIG_DIR_TARGET`, so a config replaced by a rename reaches the container.
/// The server certificate and key are written into it too.
const CONFIG_DIR: &str = "nats";
const CONFIG_DIR_TARGET: &str = config::MOUNT_DIR;
const CONFIG_FILE: &str = "nats.conf";
/// Where the server config used to be: a single bind-mounted file, which a rename never reached.
const OLD_CONFIG_FILE: &str = "nats.conf";
/// Where `start` records the bus.
const RECORD_FILE: &str = "bus.toml";
/// The registry of enrolled guests, and the lock taken while it or the config changes.
const REGISTRY_FILE: &str = "enrolled.toml";
const LOCK_FILE: &str = "enroll.lock";
const PORT: u16 = 4222;
/// How often, and how long, `start` waits for the server to accept connections, and `enroll`
/// for the reloaded server to accept the new key.
const CONNECT_ATTEMPTS: u32 = 20;
const CONNECT_DELAY: Duration = Duration::from_millis(500);

/// Where and how the bus runs.
pub struct Bus {
    pub container: String,
    pub volume: String,
    pub listen: Ipv4Addr,
    pub port: u16,
    /// The state directory: the server config, the record and the registry; outside the
    /// repository.
    pub state_dir: PathBuf,
    /// The lab the bus serves, when started for one.
    pub lab: Option<LabRecord>,
}

impl Bus {
    /// The bus as users run it: the standard container, volume and port, listening on
    /// `listen`, with its files under `$XDG_STATE_HOME/agent-bus/`.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub fn standard(listen: Ipv4Addr, lab: Option<LabRecord>) -> Result<Self> {
        Ok(Self {
            container: CONTAINER_NAME.to_owned(),
            volume: VOLUME.to_owned(),
            listen,
            port: PORT,
            state_dir: standard_state_dir()?,
            lab,
        })
    }

    /// The standard bus as `bus start` recorded it, and the agents file it was started with.
    ///
    /// # Errors
    ///
    /// Fails when there is no state directory or no usable record.
    pub fn recorded() -> Result<(Self, PathBuf)> {
        Self::from_record(CONTAINER_NAME, VOLUME, standard_state_dir()?)
    }

    /// The bus recorded in `state_dir`, run as `container` with `volume`, and its agents file.
    fn from_record(container: &str, volume: &str, state_dir: PathBuf) -> Result<(Self, PathBuf)> {
        let path = state_dir.join(RECORD_FILE);
        match Self::find_record(container, volume, state_dir)? {
            Some(found) => Ok(found),
            None => bail!(
                "No bus recorded in {}: run `agent-bus bus start` first",
                path.display()
            ),
        }
    }

    /// As `from_record`, but `None` when no bus is recorded: it was never started, or stopped.
    fn find_record(
        container: &str,
        volume: &str,
        state_dir: PathBuf,
    ) -> Result<Option<(Self, PathBuf)>> {
        let path = state_dir.join(RECORD_FILE);
        let Some(record) = state::read(&path)? else {
            return Ok(None);
        };
        let Some(agents) = record.agents else {
            bail!(
                "{} does not name the agents file: the bus was started by an older agent-bus; run `agent-bus bus start` again",
                path.display()
            );
        };
        let address: SocketAddrV4 = record
            .url
            .strip_prefix("nats://")
            .and_then(|address| address.parse().ok())
            .with_context(|| {
                format!(
                    "The URL {} in {} is not nats://<ipv4>:<port>",
                    record.url,
                    path.display()
                )
            })?;
        let bus = Self {
            container: container.to_owned(),
            volume: volume.to_owned(),
            listen: *address.ip(),
            port: address.port(),
            state_dir,
            lab: record.lab,
        };
        Ok(Some((bus, agents)))
    }

    /// Where `start` records the standard bus and the other commands look for it.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub fn standard_record_path() -> Result<PathBuf> {
        Ok(standard_state_dir()?.join(RECORD_FILE))
    }

    fn config_dir(&self) -> PathBuf {
        self.state_dir.join(CONFIG_DIR)
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir().join(CONFIG_FILE)
    }

    fn record_path(&self) -> PathBuf {
        self.state_dir.join(RECORD_FILE)
    }

    fn registry_path(&self) -> PathBuf {
        self.state_dir.join(REGISTRY_FILE)
    }

    fn lock_path(&self) -> PathBuf {
        self.state_dir.join(LOCK_FILE)
    }

    fn url(&self) -> String {
        format!("nats://{}:{}", self.listen, self.port)
    }

    /// The CA and the record in `tls/`; the server certificate and key in the config
    /// directory, which the container mounts.
    fn tls_files(&self) -> TlsFiles {
        TlsFiles::new(&self.state_dir.join(TLS_DIR), &self.config_dir())
    }

    /// The bus's URL, trusting the CA in this state directory.
    fn endpoint(&self) -> Endpoint {
        Endpoint {
            url: self.url(),
            ca: self.tls_files().ca_cert,
        }
    }
}

/// Start the bus for the agents in `agents_path` and the enrolled guests, and prepare it. This
/// renders the server config, replaces any existing bus container (the config may have changed;
/// the messages live in the volume), then connects as the host with `host_seed` and creates the
/// inbox stream and every agent's inbox.
///
/// # Errors
///
/// Fails when the agents file or the registry is not valid, `host_seed` is not the host's key,
/// or Docker or the bus fails.
pub async fn start(bus: &Bus, agents_path: &Path, host_seed: &str) -> Result<()> {
    ensure!(
        !bus.listen.is_unspecified(),
        "The bus cannot listen on {}: its TLS certificate names the one address clients connect \
         to. Give that address",
        bus.listen
    );
    let fixed = load_agents(agents_path, Some(host_seed))?;
    let agents_path = path::absolute(agents_path)
        .with_context(|| format!("Unable to resolve {}", agents_path.display()))?;
    let _lock = registry::lock(&bus.lock_path())?;
    let merged = registry::merge(fixed, &Registry::load(&bus.registry_path())?)?;
    warn_shadowed(&merged.shadowed, &agents_path);
    let agents = merged.agents;
    config::write(&bus.config_path(), &agents)?;
    println!("Wrote {}", bus.config_path().display());
    let files = bus.tls_files();
    let outcome = tls::ensure(&files, &[IpAddr::V4(bus.listen)])?;
    if outcome.ca_created {
        println!(
            "Made the bus CA {}; start every guest agent again so it gets it",
            files.ca_cert.display()
        );
    }
    if let Some(reason) = outcome.reissued {
        println!(
            "Issued the server certificate for {} ({reason})",
            bus.listen
        );
    }

    let docker = docker_connection()?;
    pull_image(&docker).await?;
    remove(&docker, &bus.container).await?;
    docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(bus.container.clone()),
                ..Default::default()
            }),
            container_config(bus),
        )
        .await
        .context("Unable to create the bus container")?;
    println!("Created {}", bus.container);
    start_container(&docker, &bus.container).await?;
    state::remove(&bus.state_dir.join(OLD_CONFIG_FILE))?;

    let channel = connect_when_ready(&bus.endpoint(), host_seed).await?;
    let outcomes = channel
        .create_inboxes(agents.all().map(|agent| agent.name.as_str()))
        .await?;
    state::write(
        &bus.record_path(),
        &BusRecord {
            url: bus.url(),
            agents: Some(agents_path),
            lab: bus.lab.clone(),
        },
    )?;
    println!("Bus listening on {}", bus.url());
    if let Some(lab) = &bus.lab {
        println!("Serving lab {} ({})", lab.name, lab.id);
    }
    for (agent, change) in outcomes {
        println!("Inbox ready: {agent} ({change})");
    }
    Ok(())
}

/// Enroll the guest `name` on the running bus and return its new seed. A fresh key pair replaces
/// the guest's previous key in the registry, the config is rendered again and the bus reloads
/// it, so the previous key is refused from then on. The guest's inbox is created first, so a
/// failure there changes no key. When the new key cannot be saved, loaded or connected with,
/// the previous registry and config are put back and loaded, so the previous key works again
/// and no key whose seed is lost stays enrolled. The seed is only returned: it is never written
/// or logged. Progress goes to stderr, so stdout can carry the seed alone.
///
/// # Errors
///
/// Fails when the name is not valid or the agents file lists it, `host_seed` is not the host's
/// key, the bus is not running or not reloadable, or the reloaded bus does not accept the new
/// key.
pub async fn enroll(bus: &Bus, agents_path: &Path, name: &str, host_seed: &str) -> Result<String> {
    check_agent_name(name)?;
    let fixed = load_agents(agents_path, Some(host_seed))?;
    refuse_fixed(&fixed, agents_path, name, "enrolled")?;
    let docker = docker_connection()?;
    ensure!(
        reloadable(&docker, bus).await?,
        "The bus container {} is not running: start it with `agent-bus bus start`",
        bus.container
    );
    let _lock = registry::lock(&bus.lock_path())?;
    let host = Channel::connect(&bus.endpoint(), host_seed).await?;
    for (agent, change) in host.create_inboxes([name]).await? {
        eprintln!("Inbox ready: {agent} ({change})");
    }
    let key = keys::generate()?;
    let mut registry = Registry::load(&bus.registry_path())?;
    let before = registry.clone();
    let previous = registry.enroll(name, &key.public_key);
    let enrolled = match apply(bus, &docker, fixed.clone(), agents_path, &registry, true).await {
        Ok(()) => connect_when_ready(&bus.endpoint(), &key.seed)
            .await
            .map(drop)
            .context("The reloaded bus refused the new key"),
        Err(error) => Err(error),
    };
    if let Err(error) = enrolled {
        let restored = apply(bus, &docker, fixed, agents_path, &before, true).await;
        return Err(match restored {
            Ok(()) => error.context(format!(
                "Unable to enroll '{name}'. The previous registry and config are back and the bus \
                 has reloaded them, so its previous key, if any, still works. Fix the cause and \
                 enroll it again"
            )),
            Err(restore) => error.context(format!(
                "Unable to enroll '{name}', and putting the previous registry and config back \
                 failed too ({restore:#}). Fix the cause, then enroll it again, which replaces \
                 the key, or run `agent-bus bus start` to render and load the config from the \
                 registry"
            )),
        });
    }
    if previous.is_some() {
        eprintln!("Enrolled '{name}' with a new key; its previous key is refused");
    } else {
        eprintln!("Enrolled '{name}'");
    }
    Ok(key.seed)
}

/// Remove the enrolled guest `name`: its key leaves the registry and the config, and a running
/// bus reloads, which disconnects and refuses it. Its inbox and messages stay in the stream. A
/// name that is not enrolled changes nothing.
///
/// # Errors
///
/// Fails when the name is not valid or the agents file lists it, the registry cannot be changed,
/// or the running bus cannot be reloaded.
pub async fn unenroll(bus: &Bus, agents_path: &Path, name: &str) -> Result<()> {
    check_agent_name(name)?;
    let fixed = load_agents(agents_path, None)?;
    refuse_fixed(&fixed, agents_path, name, "unenrolled")?;
    let docker = docker_connection()?;
    let running = reloadable(&docker, bus).await?;
    let _lock = registry::lock(&bus.lock_path())?;
    let mut registry = Registry::load(&bus.registry_path())?;
    if registry.unenroll(name).is_none() {
        eprintln!("'{name}' is not enrolled; nothing changed");
        return Ok(());
    }
    apply(bus, &docker, fixed, agents_path, &registry, running).await?;
    if running {
        eprintln!("Unenrolled '{name}': the bus no longer accepts its key");
    } else {
        eprintln!(
            "Unenrolled '{name}'; the bus is not running, and will not accept its key when started"
        );
    }
    Ok(())
}

/// Remove the enrolled guest `name` from the standard bus, running or stopped.
///
/// # Errors
///
/// As `unenroll`, and when the bus container runs but no bus is recorded.
pub async fn unenroll_standard(name: &str) -> Result<()> {
    unenroll_in(CONTAINER_NAME, VOLUME, standard_state_dir()?, name).await
}

/// Remove the enrolled guest `name` from the bus run as `container` with `volume`, whose files
/// are in `state_dir`. A recorded bus is unenrolled from with `unenroll`. With no record (the
/// bus was stopped, or never started) only the registry changes: `bus start` renders the config
/// from it, so the key is refused from the next start. The agents file is not known then, so
/// its names are not refused; they are never in the registry anyway.
async fn unenroll_in(container: &str, volume: &str, state_dir: PathBuf, name: &str) -> Result<()> {
    if let Some((bus, agents_path)) = Bus::find_record(container, volume, state_dir.clone())? {
        return unenroll(&bus, &agents_path, name).await;
    }
    check_agent_name(name)?;
    let docker = docker_connection()?;
    ensure!(
        !matches!(
            container_status(&docker, container).await,
            Ok(Some(ContainerStateStatusEnum::RUNNING))
        ),
        "The bus container {container} runs, but no bus is recorded in {}: run `agent-bus bus \
         start` again, then unenroll '{name}'",
        state_dir.join(RECORD_FILE).display()
    );
    let _lock = registry::lock(&state_dir.join(LOCK_FILE))?;
    let path = state_dir.join(REGISTRY_FILE);
    let mut registry = Registry::load(&path)?;
    if registry.unenroll(name).is_none() {
        eprintln!("'{name}' is not enrolled; nothing changed");
        return Ok(());
    }
    registry.save(&path)?;
    eprintln!("Unenrolled '{name}'; the bus is stopped, and will not accept its key when started");
    Ok(())
}

/// Every agent of the recorded bus: the agents file's, then the enrolled guests, as `start`
/// renders them. `host_seed` must be the host's.
///
/// # Errors
///
/// Fails when the agents file or the registry is not valid, or `host_seed` is not the host's.
pub fn agents(bus: &Bus, agents_path: &Path, host_seed: &str) -> Result<Agents> {
    let fixed = load_agents(agents_path, Some(host_seed))?;
    let merged = registry::merge(fixed, &Registry::load(&bus.registry_path())?)?;
    warn_shadowed(&merged.shadowed, agents_path);
    Ok(merged.agents)
}

/// Read the agents file at `path` and, given a seed, check that it is the host's.
fn load_agents(path: &Path, host_seed: Option<&str>) -> Result<Agents> {
    let agents = Agents::load(path)?;
    if let Some(seed) = host_seed {
        ensure!(
            public_key_of(seed)? == agents.host.key,
            "AGENT_BUS_SEED is not the seed of the host '{}' in the agents file",
            agents.host.name
        );
    }
    Ok(agents)
}

/// The agents file wins over the registry: a name it lists is never enrolled or unenrolled.
fn refuse_fixed(fixed: &Agents, agents_path: &Path, name: &str, action: &str) -> Result<()> {
    ensure!(
        !fixed.has(name),
        "'{name}' is in the agents file {}, which wins over enrollment, so it cannot be {action}: remove it from there first",
        agents_path.display()
    );
    Ok(())
}

fn warn_shadowed(shadowed: &[String], agents_path: &Path) {
    for name in shadowed {
        eprintln!(
            "Enrolled guest '{name}' is left out: the agents file {} lists that name, and it wins",
            agents_path.display()
        );
    }
}

/// Save `registry`, render the config from it and the agents file, and reload the bus if
/// `reload`. The merge is checked before anything is written.
async fn apply(
    bus: &Bus,
    docker: &Docker,
    fixed: Agents,
    agents_path: &Path,
    registry: &Registry,
    reload: bool,
) -> Result<()> {
    let merged = registry::merge(fixed, registry)?;
    warn_shadowed(&merged.shadowed, agents_path);
    registry.save(&bus.registry_path())?;
    config::write(&bus.config_path(), &merged.agents)?;
    if reload {
        docker
            .kill_container(
                &bus.container,
                Some(KillContainerOptionsBuilder::default().signal("HUP").build()),
            )
            .await
            .with_context(|| format!("Unable to send SIGHUP to {}", bus.container))?;
    }
    Ok(())
}

/// Whether the bus container runs and reads the config this bus writes. `false` when it is not
/// running; an error when it runs with another config, such as one started by an older
/// agent-bus, which mounted the config file itself and would never see the new one.
async fn reloadable(docker: &Docker, bus: &Bus) -> Result<bool> {
    if !matches!(
        container_status(docker, &bus.container).await,
        Ok(Some(ContainerStateStatusEnum::RUNNING))
    ) {
        return Ok(false);
    }
    let details = docker
        .inspect_container(&bus.container, None::<InspectContainerOptions>)
        .await
        .with_context(|| format!("Unable to inspect {}", bus.container))?;
    let source = bus.config_dir().display().to_string();
    let mounted = details
        .host_config
        .and_then(|host| host.mounts)
        .unwrap_or_default()
        .iter()
        .any(|mount| {
            mount.target.as_deref() == Some(CONFIG_DIR_TARGET)
                && mount.source.as_deref() == Some(source.as_str())
        });
    ensure!(
        mounted,
        "The running bus {} does not read {}: it was started by an older agent-bus or from another state directory; run `agent-bus bus start` again",
        bus.container,
        source
    );
    Ok(true)
}

/// Stop and remove the bus container. The stored messages stay in their volume.
///
/// # Errors
///
/// Fails when Docker cannot stop or remove the container.
pub async fn stop() -> Result<()> {
    stop_bus(
        &docker_connection()?,
        CONTAINER_NAME,
        &Bus::standard_record_path()?,
    )
    .await?;
    println!("Removed {CONTAINER_NAME}; stored messages are kept in {VOLUME}");
    Ok(())
}

/// Stop and remove the bus container `container`, and remove its record at `record`. A
/// container that is stopped already, or gone, is no error, so stopping again finishes the job.
async fn stop_bus(docker: &Docker, container: &str, record: &Path) -> Result<()> {
    remove(docker, container).await?;
    state::remove(record)
}

/// Print whether the bus container is running.
///
/// # Errors
///
/// Fails when Docker cannot be reached.
pub async fn status() -> Result<()> {
    let docker = docker_connection()?;
    match container_status(&docker, CONTAINER_NAME).await {
        Ok(Some(ContainerStateStatusEnum::RUNNING)) => println!("{CONTAINER_NAME}: running"),
        Ok(Some(state)) => println!("{CONTAINER_NAME}: {state}"),
        Ok(None) => println!("{CONTAINER_NAME}: state unknown"),
        Err(error) => println!("{CONTAINER_NAME}: not created ({error:#})"),
    }
    match state::read(&Bus::standard_record_path()?)? {
        Some(record) => {
            println!("URL: {}", record.url);
            match record.lab {
                Some(lab) => println!("Lab: {} ({})", lab.name, lab.id),
                None => println!("Lab: none (started with --listen)"),
            }
        }
        None => println!("URL: not recorded (run `agent-bus bus start`)"),
    }
    let ca = standard_state_dir()?.join(TLS_DIR).join(tls::CA_CERT_FILE);
    if ca.is_file() {
        println!("CA: {}", ca.display());
    } else {
        println!("CA: none yet (run `agent-bus bus start`)");
    }
    Ok(())
}

/// Pull the pinned image unless Docker already has it.
async fn pull_image(docker: &Docker) -> Result<()> {
    if docker.inspect_image(IMAGE).await.is_ok() {
        return Ok(());
    }
    println!("Pulling {IMAGE}");
    let options = CreateImageOptionsBuilder::default()
        .from_image(IMAGE)
        .build();
    let mut stream = docker.create_image(Some(options), None, None);
    while stream
        .try_next()
        .await
        .context("Unable to pull the bus image")?
        .is_some()
    {}
    Ok(())
}

/// Stop the container `name` if it runs, then remove it. A container that does not exist is
/// already removed.
async fn remove(docker: &Docker, name: &str) -> Result<()> {
    if let Ok(Some(ContainerStateStatusEnum::RUNNING)) = container_status(docker, name).await {
        stop_container(docker, name).await?;
    }
    remove_container(docker, name).await
}

/// Connect with `seed`, waiting for a freshly started or reloaded server to accept it.
async fn connect_when_ready(server: &Endpoint, seed: &str) -> Result<Channel> {
    let mut last = None;
    for _ in 0..CONNECT_ATTEMPTS {
        match Channel::connect(server, seed).await {
            Ok(channel) => return Ok(channel),
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(CONNECT_DELAY).await;
    }
    match last {
        Some(error) => Err(error).context("The bus did not accept the connection"),
        None => bail!("The bus did not accept the connection"),
    }
}

fn container_config(bus: &Bus) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec![
            "-c".to_owned(),
            format!("{CONFIG_DIR_TARGET}/{CONFIG_FILE}"),
            "--addr".to_owned(),
            bus.listen.to_string(),
            "--port".to_owned(),
            bus.port.to_string(),
            "--jetstream".to_owned(),
            "--store_dir".to_owned(),
            DATA_DIR.to_owned(),
        ]),
        host_config: Some(HostConfig {
            network_mode: Some("host".to_owned()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            mounts: Some(vec![
                Mount {
                    target: Some(DATA_DIR.to_owned()),
                    source: Some(bus.volume.clone()),
                    typ: Some(MountTypeEnum::VOLUME),
                    ..Default::default()
                },
                Mount {
                    target: Some(CONFIG_DIR_TARGET.to_owned()),
                    source: Some(bus.config_dir().display().to_string()),
                    typ: Some(MountTypeEnum::BIND),
                    read_only: Some(true),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_nats::connection::State;
    use async_nats::jetstream::consumer::DeliverPolicy;
    use async_nats::rustls::crypto::aws_lc_rs;
    use async_nats::{Client, ConnectOptions, Event};
    use bollard::query_parameters::RemoveVolumeOptions;
    use futures_util::StreamExt;
    use shared::util::LabFolder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::watch;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use super::*;

    use crate::agents::Role;
    use crate::channel::{Inbox, inbox_prefix, tls_options};
    use crate::files::{DIR_MODE, PRIVATE_FILE_MODE};
    use crate::files::{millis, now_ms};
    use crate::guest::run::heartbeat;
    use crate::keys::{NewKey, generate};
    use crate::listen::{Journal, serve};
    use crate::message::{Kind, Message, TaskUsage, Usage};
    use crate::report;
    use crate::status::{Heartbeat, Observed, Online, Row, Running, STALE, Status, row};

    /// The heartbeat's beat in the offline test, not the production `HEARTBEAT`.
    const BEAT: Duration = Duration::from_millis(200);

    /// Install aws-lc-rs as rustls's process default, as `main` does, for the tests that connect
    /// to a real bus.
    fn install_crypto_provider() {
        let _ = aws_lc_rs::default_provider().install_default();
    }

    fn test_bus(state_dir: PathBuf) -> Bus {
        Bus {
            container: "sherpa-agent-bus-test".to_owned(),
            volume: "sherpa-agent-bus-test-data".to_owned(),
            listen: Ipv4Addr::new(172, 31, 3, 1),
            port: 4222,
            state_dir,
            lab: None,
        }
    }

    #[test]
    fn the_bus_listens_only_on_the_given_address_with_jetstream_and_its_config() {
        let bus = test_bus(PathBuf::from("/home/u/.local/state/agent-bus"));
        let config = container_config(&bus);
        let command = config.cmd.unwrap();
        assert_eq!(command[..2], ["-c", "/etc/agent-bus/nats.conf"]);
        assert_eq!(command[2..6], ["--addr", "172.31.3.1", "--port", "4222"]);
        assert!(command.contains(&"--jetstream".to_owned()));
        let host = config.host_config.unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("host"));
        let mounts = host.mounts.unwrap();
        assert_eq!(
            mounts[0].source.as_deref(),
            Some("sherpa-agent-bus-test-data")
        );
        assert_eq!(mounts[1].typ, Some(MountTypeEnum::BIND));
        // The directory is mounted, not the file, so a config replaced by a rename reaches the
        // container when it reloads.
        assert_eq!(
            mounts[1].source.as_deref(),
            Some("/home/u/.local/state/agent-bus/nats")
        );
        assert_eq!(mounts[1].target.as_deref(), Some(CONFIG_DIR_TARGET));
        assert_eq!(mounts[1].read_only, Some(true));
        assert_eq!(
            bus.config_path(),
            PathBuf::from("/home/u/.local/state/agent-bus/nats/nats.conf")
        );
    }

    #[test]
    fn the_recorded_bus_gives_the_address_and_the_agents_file() {
        let dir = std::env::temp_dir().join(format!("agent-bus-recorded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state_dir = dir.join("agent-bus");
        let error = Bus::from_record("c", "v", state_dir.clone()).err().unwrap();
        assert!(format!("{error:#}").contains("bus start"), "{error:#}");
        let mut record = BusRecord {
            url: "nats://192.0.2.1:4300".to_owned(),
            agents: None,
            lab: None,
        };
        state::write(&state_dir.join(RECORD_FILE), &record).unwrap();
        // A record from before enrollment does not name the agents file.
        let error = Bus::from_record("c", "v", state_dir.clone()).err().unwrap();
        assert!(
            format!("{error:#}").contains("older agent-bus"),
            "{error:#}"
        );
        record.agents = Some(PathBuf::from("/home/u/agents.toml"));
        state::write(&state_dir.join(RECORD_FILE), &record).unwrap();
        let (bus, agents) = Bus::from_record("c", "v", state_dir.clone()).unwrap();
        assert_eq!(bus.url(), "nats://192.0.2.1:4300");
        assert_eq!(bus.container, "c");
        assert_eq!(agents, PathBuf::from("/home/u/agents.toml"));
        assert_eq!(bus.registry_path(), state_dir.join("enrolled.toml"));
        record.url = "tls://example:4222".to_owned();
        state::write(&state_dir.join(RECORD_FILE), &record).unwrap();
        assert!(Bus::from_record("c", "v", state_dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn message(from: &str, to: &str, text: &str) -> Message {
        Message {
            task: "1a2b3c4d".to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind: Kind::Task,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    /// What the integration test shows, run against a started bus.
    async fn exercise(bus: &Bus, agents: &Agents, host: &NewKey, dev: &NewKey) -> Result<()> {
        let url = bus.endpoint();
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&url, &host.seed).await?;
        let dev_channel = Channel::connect(&url, &dev.seed).await?;

        // The host's task reaches the guest, which reads and acknowledges it.
        host_channel.send(&message("host", "dev", "first")).await?;
        let mut dev_inbox = dev_channel.inbox("dev").await?;
        let delivery = timeout(wait, dev_inbox.next())
            .await
            .context("no message for dev")??
            .context("the inbox closed")?;
        ensure!(delivery.message.text == "first", "wrong message");
        delivery.ack().await?;
        ensure!(
            host_channel.waiting("dev").await? == 0,
            "the guest's acknowledgement was not taken"
        );

        // The guest's reply reaches the host.
        dev_channel.send(&message("dev", "host", "reply")).await?;
        let mut host_inbox = host_channel.inbox("host").await?;
        let reply = timeout(wait, host_inbox.next())
            .await
            .context("no message for host")??
            .context("the inbox closed")?;
        ensure!(reply.message.text == "reply", "wrong reply");
        reply.ack().await?;

        // The host's listener drops a message whose sender is not its subject's, and takes the
        // next. Only the host could publish it: a guest may publish under its own name only.
        let (raw_host, _) = watched(&url, &host.seed).await?;
        raw_host
            .publish(
                "agents.host.dev",
                serde_json::to_vec(&message("other", "host", "forged"))?.into(),
            )
            .await?;
        raw_host.flush().await?;
        dev_channel.send(&message("dev", "host", "genuine")).await?;
        let next = timeout(wait, host_inbox.next())
            .await
            .context("no message for host")??
            .context("the inbox closed")?;
        ensure!(
            next.message.text == "genuine",
            "the host took a forged message: {}",
            next.message.text
        );
        next.ack().await?;

        // The guest cannot pose as another agent.
        let posed = timeout(wait, dev_channel.send(&message("other", "host", "posing"))).await;
        ensure!(
            !matches!(posed, Ok(Ok(()))),
            "a guest sent as another agent"
        );

        // It cannot read the host's replies: a request of the host's gets no copy to the guest.
        let (spy, _) = watched(&url, &dev.seed).await?;
        let mut spied = spy
            .subscribe(format!("{}.>", inbox_prefix(&host.public_key)))
            .await?;
        let mut everything = spy.subscribe("_INBOX.>").await?;
        host_channel.waiting("dev").await?;
        ensure!(
            timeout(Duration::from_secs(2), spied.next()).await.is_err()
                && timeout(Duration::from_secs(1), everything.next())
                    .await
                    .is_err(),
            "a guest read the host's replies"
        );

        // The guest cannot send to another agent. The message must not arrive.
        let denied = timeout(wait, dev_channel.send(&message("dev", "other", "no"))).await;
        ensure!(
            !matches!(denied, Ok(Ok(()))),
            "a guest sent to another agent"
        );
        ensure!(
            host_channel.waiting("other").await? == 0,
            "a message reached another agent's inbox"
        );
        // It cannot read another agent's inbox either.
        ensure!(
            timeout(wait, dev_channel.inbox("other"))
                .await
                .is_ok_and(|inbox| inbox.is_err()),
            "a guest opened another agent's inbox"
        );
        // It cannot create the stream or the consumers.
        ensure!(
            timeout(
                wait,
                dev_channel.create_inboxes(agents.all().map(|agent| agent.name.as_str()))
            )
            .await
            .is_ok_and(|created| created.is_err()),
            "a guest created the stream"
        );

        // A seed that is not in the agents file is refused.
        let stranger = generate()?;
        ensure!(
            timeout(wait, Channel::connect(&url, &stranger.seed))
                .await
                .is_ok_and(|connected| connected.is_err()),
            "an unknown key connected"
        );
        Ok(())
    }

    /// A guest writes its status and the host reads it with the inbox's state; a guest may write
    /// no other agent's status and read none.
    async fn status_on_bus(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let dev_channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
        let names = ["host", "dev", "other"];

        // No status yet for dev (the earlier steps wrote none).
        let none = host_channel
            .statuses(names)
            .await?
            .context("bus start created no status bucket")?;
        ensure!(none.is_empty(), "statuses before any was written: {none:?}");

        let now = now_ms();
        let status = Status {
            online: true,
            at_ms: now,
            running: Some(Running {
                task: "t1".to_owned(),
                since_ms: now - 30_000,
            }),
            queued: 2,
        };
        dev_channel.put_status("dev", &status).await?;
        host_channel.send(&message("host", "dev", "queued")).await?;
        let found = host_channel.statuses(names).await?.unwrap_or_default();
        ensure!(
            found.len() == 1 && found[0].0 == "dev" && found[0].1.status == status,
            "the host read {found:?}"
        );
        let stored = found[0].1.stored_ms;
        ensure!(
            stored.abs_diff(now_ms()) < 10_000,
            "the bus's time {stored} is not now"
        );
        let state = host_channel
            .inbox_state("dev")
            .await?
            .context("dev has no inbox")?;
        let shown = row(
            &Observed {
                name: "dev".to_owned(),
                role: Role::Guest,
                inbox: Some(state),
                heartbeat: Some(found[0].1.clone()),
            },
            now_ms(),
            STALE,
        );
        ensure!(shown.online == Online::Yes, "{shown:?}");
        ensure!(
            shown.running.as_ref().is_some_and(|running| running.task == "t1"
                && (30..40).contains(&running.age_secs)),
            "{shown:?}"
        );
        ensure!(
            shown.local_queue == Some(2)
                && shown.inbox_unread == Some(1)
                && shown.queued == Some(3),
            "{shown:?}"
        );
        ensure!(
            host_channel.inbox_state("nobody").await?.is_none(),
            "an inbox for an agent that has none"
        );

        status_refusals(&host_channel, &dev_channel, &status).await?;

        // The last status, written as a guest stops, takes it offline at once.
        dev_channel
            .put_status(
                "dev",
                &Status {
                    online: false,
                    ..status
                },
            )
            .await?;
        let found = host_channel.statuses(names).await?.unwrap_or_default();
        let last = row(
            &Observed {
                name: "dev".to_owned(),
                role: Role::Guest,
                inbox: None,
                heartbeat: found.first().map(|(_, heartbeat)| heartbeat.clone()),
            },
            now_ms(),
            STALE,
        );
        ensure!(
            last.online == Online::No && last.running.is_none(),
            "{last:?}"
        );

        // Drain dev's inbox for the later steps.
        let mut inbox = dev_channel.inbox("dev").await?;
        let delivery = timeout(wait, inbox.next())
            .await
            .context("no message for dev")??
            .context("the inbox closed")?;
        delivery.ack().await
    }

    /// The heartbeat as the guest agent runs it, against the bus, with a short beat and a short
    /// stale threshold so the test does not wait 35 s. A guest that stops cleanly writes a last,
    /// offline status and is offline at once, even against the production threshold. A guest
    /// that is killed writes nothing more: it shows online until its last status is older than
    /// the threshold, then offline.
    async fn offline_detection(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let stale = Duration::from_secs(2);
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;

        // A clean stop: the engine is gone, the heartbeat writes its last status and returns.
        let (status, beating) = start_heartbeat(bus, dev, &host_channel).await?;
        let beat = dev_status(&host_channel).await?;
        let shown = dev_row(&beat, stale);
        ensure!(
            shown.online == Online::Yes,
            "a beating guest is not online: {shown:?}"
        );
        drop(status);
        timeout(wait, beating)
            .await
            .context("the heartbeat did not end with the engine")??;
        let last = dev_status(&host_channel).await?;
        ensure!(
            !last.status.online,
            "no offline status was written: {last:?}"
        );
        let shown = dev_row(&last, STALE);
        ensure!(
            shown.online == Online::No && shown.last_active_age_secs.is_some_and(|age| age < 5),
            "a guest that stopped is not offline at once: {shown:?}"
        );

        // A kill: the heartbeat ends without its last status, and its connection goes.
        let (status, beating) = start_heartbeat(bus, dev, &host_channel).await?;
        beating.abort();
        ensure!(
            beating.await.is_err_and(|error| error.is_cancelled()),
            "the heartbeat was not killed"
        );
        // A write in flight as it was killed has landed by now.
        tokio::time::sleep(BEAT).await;
        let last = dev_status(&host_channel).await?;
        ensure!(
            last.status.online,
            "a killed guest wrote an offline status: {last:?}"
        );
        let shown = dev_row(&last, stale);
        ensure!(
            shown.online == Online::Yes,
            "a status younger than the threshold is not online: {shown:?}"
        );
        let stale_at = Duration::from_millis(last.stored_ms.saturating_add(millis(stale)));
        let until = stale_at.saturating_sub(Duration::from_millis(now_ms()));
        tokio::time::sleep(until + Duration::from_millis(500)).await;
        let after = dev_status(&host_channel).await?;
        ensure!(
            after == last,
            "a killed heartbeat still wrote: {after:?} after {last:?}"
        );
        let shown = dev_row(&after, stale);
        ensure!(
            shown.online == Online::No && shown.running.is_none(),
            "a stale status is not offline: {shown:?}"
        );
        drop(status);
        Ok(())
    }

    /// Start dev's heartbeat, beating every `BEAT`, and wait until the bus has an online status
    /// from it. The sender is the engine's end: dropping it stops the heartbeat cleanly.
    async fn start_heartbeat(
        bus: &Bus,
        dev: &NewKey,
        host_channel: &Channel,
    ) -> Result<(watch::Sender<Status>, JoinHandle<()>)> {
        let channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
        let (status, receiver) = watch::channel(Status {
            online: true,
            ..Status::default()
        });
        let started = now_ms();
        let beating = tokio::spawn(async move {
            heartbeat(&channel, "dev", receiver, BEAT).await;
        });
        timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(beat) = dev_status(host_channel).await
                    && beat.status.online
                    && beat.status.at_ms >= started
                {
                    return;
                }
                tokio::time::sleep(BEAT).await;
            }
        })
        .await
        .context("the heartbeat wrote no online status")?;
        Ok((status, beating))
    }

    /// dev's last status, as the host reads it.
    async fn dev_status(host_channel: &Channel) -> Result<Heartbeat> {
        let found = host_channel.statuses(["dev"]).await?.unwrap_or_default();
        found
            .into_iter()
            .next()
            .map(|(_, beat)| beat)
            .context("dev has no status")
    }

    /// dev's row from its status alone, with `stale` as the threshold.
    fn dev_row(beat: &Heartbeat, stale: Duration) -> Row {
        row(
            &Observed {
                name: "dev".to_owned(),
                role: Role::Guest,
                inbox: None,
                heartbeat: Some(beat.clone()),
            },
            now_ms(),
            stale,
        )
    }

    /// A guest may write no other agent's status, and read none.
    async fn status_refusals(host: &Channel, dev: &Channel, status: &Status) -> Result<()> {
        let wait = Duration::from_secs(15);
        let names = ["host", "dev", "other"];
        // dev cannot write another agent's status, or read any.
        let forged = timeout(wait, dev.put_status("other", status)).await;
        ensure!(
            !matches!(forged, Ok(Ok(()))),
            "a guest wrote another agent's status"
        );
        let found = host.statuses(names).await?.unwrap_or_default();
        ensure!(found.len() == 1, "a forged status was stored: {found:?}");
        ensure!(
            timeout(wait, dev.statuses(names))
                .await
                .is_ok_and(|read| read.is_err()),
            "a guest read the status bucket"
        );

        Ok(())
    }

    /// A guest sends progress messages and then a completed one with usage; a host listener
    /// must get every one, also after it restarts.
    async fn reply_burst(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let wait = Duration::from_secs(15);
        let mut host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let dev_channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
        let mut inbox = host_channel.inbox("host").await?;
        for round in 0..2 {
            let task = format!("burst{round}");
            for n in 0..4 {
                let mut progress = message("dev", "host", &format!("progress {n}"));
                progress.task.clone_from(&task);
                progress.kind = Kind::Progress;
                dev_channel.send(&progress).await?;
            }
            let mut done = message("dev", "host", "done");
            done.task.clone_from(&task);
            done.kind = Kind::Completed;
            done.usage = Some(Usage {
                input: 3,
                output: 120,
                cache_creation: 900,
                cache_read: 4000,
                num_turns: 2,
                duration_ms: 5000,
                duration_api_ms: 4000,
                cost_usd: 0.0123,
                session_cost_usd: 0.0123,
                models: Vec::new(),
            });
            dev_channel.send(&done).await?;
            for n in 0..5 {
                let delivery = timeout(wait, inbox.next())
                    .await
                    .with_context(|| format!("message {n} of {task} never reached the host"))??
                    .context("the inbox closed")?;
                ensure!(delivery.message.task == task, "wrong task");
                delivery.ack().await?;
            }
            // A second reader would take some of the messages, so the bus refuses it.
            let second = Channel::connect(&bus.endpoint(), &host.seed).await?;
            ensure!(
                second.inbox("host").await.is_err(),
                "a second reader was attached to the host's inbox"
            );
            // The listener dies with a pull request outstanding, and the guest sends meanwhile.
            drop(inbox);
            drop(host_channel);
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut late = message("dev", "host", "late");
            late.task.clone_from(&task);
            late.kind = Kind::Progress;
            dev_channel.send(&late).await?;
            host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
            inbox = host_channel.inbox("host").await?;
            let delivery = timeout(wait, inbox.next())
                .await
                .context("a message sent while the listener was gone never arrived")??
                .context("the inbox closed")?;
            ensure!(delivery.message.text == "late", "wrong message");
            delivery.ack().await?;
            // The listener restarts: nothing is left, nothing is delivered again.
            drop(inbox);
            inbox = host_channel.inbox("host").await?;
            ensure!(
                host_channel.waiting("host").await? == 0,
                "messages are left in the host's inbox"
            );
        }
        Ok(())
    }

    /// `agent-bus listen` keeps each message in its journal and acknowledges it; a restarted
    /// listener with `--since` prints them from the journal, and the bus delivers none again.
    async fn listener_journal(bus: &Bus, host: &NewKey, dev: &NewKey, dir: &Path) -> Result<()> {
        let wait = Duration::from_secs(15);
        let path = dir.join("state").join("listen-host.jsonl");
        let dev_channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let listener = {
            let channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
            let journal = Journal::new(path.clone());
            tokio::spawn(async move {
                serve(&channel, "host", journal, None, &mut std::io::sink()).await
            })
        };
        for n in 0..3 {
            let mut reply = message("dev", "host", &format!("journal {n}"));
            reply.task = "journal1".to_owned();
            dev_channel.send(&reply).await?;
        }
        let journaled = |path: &Path| {
            std::fs::read_to_string(path).map_or(0, |text| {
                text.lines()
                    .filter(|line| line.contains("journal1"))
                    .count()
            })
        };
        timeout(wait, async {
            while journaled(&path) < 3 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("the listener did not journal the messages")?;
        // Journaled before acknowledged: wait for the last acknowledgement to be taken.
        timeout(wait, async {
            while host_channel.waiting("host").await.unwrap_or(1) > 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("the listener did not acknowledge the messages")?;
        listener.abort();
        let _ = listener.await;
        ensure!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777 == PRIVATE_FILE_MODE,
            "the journal is not private"
        );
        ensure!(
            std::fs::metadata(path.parent().context("no directory")?)?
                .permissions()
                .mode()
                & 0o777
                == DIR_MODE,
            "the state directory is not private"
        );
        // A restarted listener catches up from the journal, and nothing is delivered again.
        let texts: Vec<String> = Journal::new(path.clone())
            .since(0)?
            .into_iter()
            .filter(|message| message.task == "journal1")
            .map(|message| message.text)
            .collect();
        ensure!(
            texts == ["journal 0", "journal 1", "journal 2"],
            "wrong journal: {texts:?}"
        );
        let mut printed = Vec::new();
        let channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let restarted = timeout(
            Duration::from_secs(2),
            serve(
                &channel,
                "host",
                Journal::new(path.clone()),
                Some(Duration::from_hours(1)),
                &mut printed,
            ),
        )
        .await;
        ensure!(
            restarted.is_err(),
            "the restarted listener stopped: {restarted:?}"
        );
        let printed = String::from_utf8(printed)?;
        ensure!(
            printed.matches("journal1").count() == 3,
            "the restarted listener printed {printed}"
        );
        ensure!(
            journaled(&path) == 3,
            "a message was delivered again after the restart"
        );
        Ok(())
    }

    /// The host reads the outcomes still kept in the stream, with its own permissions, and the
    /// read leaves its inbox as it was.
    async fn usage_history(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let dev_channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
        for (kind, runs, cost_usd) in [(Kind::Question, 1, 0.04), (Kind::Completed, 2, 0.06)] {
            let mut outcome = message("dev", "host", "outcome");
            outcome.task = "usage1".to_owned();
            outcome.kind = kind;
            outcome.task_usage = Some(TaskUsage {
                runs,
                cost_usd,
                input: 10,
                output: 5,
                cache_read: 90,
                ..TaskUsage::default()
            });
            dev_channel.send(&outcome).await?;
        }
        let messages = timeout(wait, host_channel.history("host"))
            .await
            .context("the history was not read in time")??;
        let rows = report::rows(&messages, Some("dev"), false);
        let row = rows
            .iter()
            .find(|row| row.task == "usage1")
            .context("the task is missing from the report")?;
        ensure!(row.state == Kind::Completed, "wrong state {:?}", row.state);
        ensure!(
            row.usage.as_ref().is_some_and(|usage| usage.runs == 2),
            "wrong usage {:?}",
            row.usage
        );
        ensure!(
            report::table(&rows).contains("$0.0600"),
            "the cost is missing from the table"
        );
        // `show` prints the task's messages, all kinds, oldest first.
        let shown = report::task_messages(&messages, "usage1");
        ensure!(
            shown.len() == 2
                && shown.first().is_some_and(|m| m.kind == Kind::Question)
                && shown.get(1).is_some_and(|m| m.kind == Kind::Completed),
            "wrong messages for the task: {shown:?}"
        );
        // Nothing was acknowledged or consumed: the two outcomes are still in the inbox, and
        // reading twice gives the same.
        ensure!(
            host_channel.waiting("host").await? >= 2,
            "the history read consumed the host's inbox"
        );
        ensure!(
            host_channel.history("host").await?.len() == messages.len(),
            "a second read of the history differs"
        );
        // Drain the inbox as the other checks expect.
        let mut inbox = host_channel.inbox("host").await?;
        while host_channel.waiting("host").await? > 0 {
            let delivery = timeout(wait, inbox.next())
                .await
                .context("the inbox ran dry")??
                .context("the inbox closed")?;
            delivery.ack().await?;
        }
        // The guest may not read the history: it cannot create a consumer.
        ensure!(
            timeout(wait, dev_channel.history("host"))
                .await
                .is_ok_and(|read| read.is_err()),
            "a guest read the host's history"
        );
        Ok(())
    }

    /// An inbox filtered on `agents.<name>` and started at a stream sequence (the shape a live
    /// bus had after the `max_waiting` upgrade, before per-sender subjects) is updated in place
    /// by `bus start`: the bus refuses to change where a consumer starts, so the update keeps
    /// it. A message that was waiting, unread and unacknowledged, is delivered once; one from
    /// before the start is not.
    async fn upgrade_unsplit_inbox(
        bus: &Bus,
        agents_path: &Path,
        host: &NewKey,
        other: &NewKey,
    ) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        host_channel
            .send(&message("host", "other", "before the start"))
            .await?;
        let start_sequence = host_channel.make_inbox_unsplit("other").await?;
        host_channel
            .send(&message("host", "other", "waiting"))
            .await?;
        drop(host_channel);

        start(bus, agents_path, &host.seed).await?;

        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let config = host_channel.inbox_config("other").await?;
        ensure!(
            config.filter_subject == "agents.other.>",
            "the filter was not updated: {}",
            config.filter_subject
        );
        ensure!(
            config.deliver_policy == DeliverPolicy::ByStartSequence { start_sequence },
            "the start changed: {:?}",
            config.deliver_policy
        );
        ensure!(config.max_waiting == 1, "max_waiting changed");
        let other_channel = Channel::connect(&bus.endpoint(), &other.seed).await?;
        let mut inbox = other_channel.inbox("other").await?;
        let delivery = timeout(wait, inbox.next())
            .await
            .context("the waiting message did not arrive after the upgrade")??
            .context("the inbox closed")?;
        ensure!(
            delivery.message.text == "waiting",
            "expected the waiting message, got {}",
            delivery.message.text
        );
        delivery.ack().await?;
        ensure!(
            timeout(Duration::from_secs(3), inbox.next()).await.is_err(),
            "a message was delivered again or from before the start"
        );
        drop(inbox);
        ensure!(
            host_channel.waiting("other").await? == 0,
            "messages are left in the inbox"
        );
        Ok(())
    }

    /// An inbox made by an earlier version (no `max_waiting`) is upgraded by `bus start`: what
    /// was acknowledged stays gone and what was not is delivered exactly once.
    async fn upgrade_old_inbox(
        bus: &Bus,
        agents_path: &Path,
        host: &NewKey,
        other: &NewKey,
    ) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        let other_channel = Channel::connect(&bus.endpoint(), &other.seed).await?;
        host_channel.make_inbox_old("other").await?;
        for n in 1..=5 {
            host_channel
                .send(&message("host", "other", &format!("m{n}")))
                .await?;
        }
        let mut inbox = other_channel.inbox("other").await?;
        for expected in ["m1", "m2"] {
            let delivery = timeout(wait, inbox.next())
                .await
                .context("no message for other")??
                .context("the inbox closed")?;
            ensure!(delivery.message.text == expected, "wrong message");
            delivery.ack().await?;
        }
        // m3 is delivered but never acknowledged; m4 and m5 are not read at all.
        let unacked = timeout(wait, inbox.next())
            .await
            .context("no third message")??
            .context("the inbox closed")?;
        ensure!(unacked.message.text == "m3", "wrong message");
        drop(unacked);
        drop(inbox);
        drop(other_channel);
        drop(host_channel);

        start(bus, agents_path, &host.seed).await?;

        let other_channel = Channel::connect(&bus.endpoint(), &other.seed).await?;
        let mut inbox = other_channel.inbox("other").await?;
        for expected in ["m3", "m4", "m5"] {
            let delivery = timeout(wait, inbox.next())
                .await
                .with_context(|| format!("{expected} did not arrive after the upgrade"))??
                .context("the inbox closed")?;
            ensure!(
                delivery.message.text == expected,
                "expected {expected}, got {}",
                delivery.message.text
            );
            delivery.ack().await?;
        }
        // Nothing else comes: not the acknowledged messages, not a second copy.
        ensure!(
            timeout(Duration::from_secs(3), inbox.next()).await.is_err(),
            "a message was delivered again after the upgrade"
        );
        drop(inbox);
        let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
        ensure!(
            host_channel.waiting("other").await? == 0,
            "messages are left in the inbox"
        );
        // The upgraded inbox has the new setting: a second reader is refused.
        let mut first = other_channel.inbox("other").await?;
        // The reader asks the bus for messages only once it waits for one.
        let _ = timeout(Duration::from_secs(1), first.next()).await;
        let second = Channel::connect(&bus.endpoint(), &other.seed).await?;
        ensure!(
            second.inbox("other").await.is_err(),
            "the upgraded inbox accepts a second reader"
        );
        drop(first);
        Ok(())
    }

    /// Needs Docker. Starts a throwaway bus on 127.0.0.1 and shows what a guest may and may not
    /// do, then that restarting the bus replaces the container and keeps the messages.
    /// Run with `cargo test -p agent-bus -- --ignored`.
    #[tokio::test]
    #[ignore = "needs Docker"]
    async fn a_guest_is_held_to_its_own_inbox_on_a_real_bus() {
        install_crypto_provider();
        let (host, dev, other) = (
            generate().unwrap(),
            generate().unwrap(),
            generate().unwrap(),
        );
        let text = format!(
            "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n\
             [[agent]]\nname = \"dev\"\nkey = \"{}\"\nrole = \"guest\"\n\
             [[agent]]\nname = \"other\"\nkey = \"{}\"\nrole = \"guest\"\n",
            host.public_key, dev.public_key, other.public_key
        );
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("agent-bus-it-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let agents_path = dir.join("agents.toml");
        std::fs::write(&agents_path, &text).unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // The bus is started for a lab whose gateway is the loopback address.
        let lab_dir = dir.join("lab");
        let lab_info = LabFolder::new(&lab_dir).lab_info();
        std::fs::create_dir_all(lab_info.parent().unwrap()).unwrap();
        std::fs::write(
            &lab_info,
            "id = \"abc123\"\nname = \"it\"\nuser = \"sherpa\"\n\
             ipv4_network = \"127.0.0.0/24\"\nipv4_gateway = \"127.0.0.1\"\n\
             ipv4_router = \"127.0.0.2\"\nloopback_network = \"127.0.1.0/24\"\n",
        )
        .unwrap();
        let (gateway, lab) = state::lab_gateway(&lab_dir).unwrap();
        let bus = Bus {
            container: format!("sherpa-agent-bus-test-{id}"),
            volume: format!("sherpa-agent-bus-test-{id}-data"),
            listen: gateway,
            port,
            state_dir: dir.join("state"),
            lab: Some(lab),
        };
        let agents = Agents::parse(&text).unwrap();

        let result = async {
            start(&bus, &agents_path, &host.seed).await?;
            // With no --server and no AGENT_BUS_URL, the recorded bus is found and connects.
            let found = state::find_server(None, None, &bus.record_path())?;
            ensure!(found == bus.url(), "the recorded URL is {found}");
            let recorded = state::read(&bus.record_path())?.context("no bus record")?;
            ensure!(
                recorded.lab.is_some_and(|lab| lab.name == "it"),
                "the lab was not recorded"
            );
            Channel::connect(
                &Endpoint {
                    url: found,
                    ca: bus.endpoint().ca,
                },
                &host.seed,
            )
            .await?;
            exercise(&bus, &agents, &host, &dev).await?;

            // A guest writes its own status, which the host reads; no one else's.
            status_on_bus(&bus, &host, &dev).await?;

            // A guest that stops is offline at once; one that is killed, once its status is stale.
            offline_detection(&bus, &host, &dev).await?;

            // Every message a guest sends reaches a host listener: progress, then the outcome
            // with its usage.
            reply_burst(&bus, &host, &dev).await?;

            // `usage` reads the kept outcomes without consuming the host's inbox.
            usage_history(&bus, &host, &dev).await?;

            // `listen` journals each message before acknowledging it.
            listener_journal(&bus, &host, &dev, &dir).await?;

            // A task waits through a restart: the container is replaced, the volume is kept.
            let host_channel = Channel::connect(&bus.endpoint(), &host.seed).await?;
            host_channel.send(&message("host", "dev", "kept")).await?;
            drop(host_channel);
            start(&bus, &agents_path, &host.seed).await?;
            let dev_channel = Channel::connect(&bus.endpoint(), &dev.seed).await?;
            let mut inbox = dev_channel.inbox("dev").await?;
            let delivery = timeout(Duration::from_secs(15), inbox.next())
                .await
                .context("no message for dev after the restart")??
                .context("the inbox closed")?;
            ensure!(delivery.message.text == "kept", "the message was lost");
            delivery.ack().await?;
            drop(inbox);
            drop(dev_channel);

            // An inbox made by an earlier version is upgraded without losing or replaying.
            upgrade_old_inbox(&bus, &agents_path, &host, &other).await?;

            // An inbox from before per-sender subjects, recreated by the `max_waiting` upgrade,
            // gets the new filter and keeps where it starts.
            upgrade_unsplit_inbox(&bus, &agents_path, &host, &other).await
        }
        .await;

        // Clean up whatever happened.
        let docker = docker_connection().unwrap();
        let _ = remove_container(&docker, &bus.container).await;
        let _ = docker
            .remove_volume(&bus.volume, None::<RemoveVolumeOptions>)
            .await;
        let _ = std::fs::remove_dir_all(&dir);
        result.unwrap();
    }

    /// A connection with `seed` that counts how often it is disconnected.
    async fn watched(url: &Endpoint, seed: &str) -> Result<(Client, Arc<AtomicUsize>)> {
        let drops = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&drops);
        let client = tls_options(ConnectOptions::with_nkey(seed.to_owned()), url)?
            .event_callback(move |event| {
                let counter = Arc::clone(&counter);
                async move {
                    if matches!(event, Event::Disconnected) {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
            .connect(&url.url)
            .await?;
        Ok((client, drops))
    }

    /// The host sends `text` to `name`, and `inbox` gets it.
    async fn delivered(host: &Channel, inbox: &mut Inbox, name: &str, text: &str) -> Result<()> {
        host.send(&message("host", name, text)).await?;
        let delivery = timeout(Duration::from_secs(15), inbox.next())
            .await
            .with_context(|| format!("no message for {name}"))??
            .context("the inbox closed")?;
        ensure!(delivery.message.text == text, "wrong message for {name}");
        delivery.ack().await
    }

    /// Whether the bus refuses `seed`.
    async fn refused(url: &Endpoint, seed: &str) -> bool {
        timeout(Duration::from_secs(15), Channel::connect(url, seed))
            .await
            .is_ok_and(|connected| connected.is_err())
    }

    /// What the enrollment test shows, run against a started bus.
    async fn enrollment(bus: &Bus, agents_path: &Path, host: &NewKey) -> Result<()> {
        let url = bus.endpoint();
        let host_channel = Channel::connect(&url, &host.seed).await?;

        // Two guests are enrolled; each connects with its own seed and reads its own inbox.
        let old_a = enroll(bus, agents_path, "worker-a", &host.seed).await?;
        let b = enroll(bus, agents_path, "worker-b", &host.seed).await?;
        ensure!(
            public_key_of(&old_a)? != public_key_of(&b)?,
            "two guests got one key"
        );
        let (a_client, a_drops) = watched(&url, &old_a).await?;
        let (b_client, b_drops) = watched(&url, &b).await?;
        let b_channel = Channel::connect(&url, &b).await?;
        let mut b_inbox = b_channel.inbox("worker-b").await?;
        delivered(&host_channel, &mut b_inbox, "worker-b", "before").await?;
        {
            let a_channel = Channel::connect(&url, &old_a).await?;
            let mut a_inbox = a_channel.inbox("worker-a").await?;
            delivered(&host_channel, &mut a_inbox, "worker-a", "first key").await?;
        }

        // Re-enrolling worker-a: its old key is refused and its connection dropped, while
        // worker-b stays connected and keeps reading on the connection it had.
        let new_a = enroll(bus, agents_path, "worker-a", &host.seed).await?;
        ensure!(
            public_key_of(&new_a)? != public_key_of(&old_a)?,
            "re-enrolling kept the key"
        );
        ensure!(
            refused(&url, &old_a).await,
            "the previous key still connects"
        );
        timeout(Duration::from_secs(10), async {
            while a_drops.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .context("the previous key's connection was not dropped")?;
        drop(a_client);
        delivered(&host_channel, &mut b_inbox, "worker-b", "after").await?;
        ensure!(
            b_drops.load(Ordering::SeqCst) == 0 && b_client.connection_state() == State::Connected,
            "worker-b was disconnected by the reload"
        );
        {
            let a_channel = Channel::connect(&url, &new_a).await?;
            let mut a_inbox = a_channel.inbox("worker-a").await?;
            delivered(&host_channel, &mut a_inbox, "worker-a", "second key").await?;
        }

        // The registry holds the public keys only, privately.
        let registry = std::fs::read_to_string(bus.registry_path())?;
        ensure!(
            registry.contains(&public_key_of(&new_a)?) && registry.contains(&public_key_of(&b)?),
            "the registry lacks a key"
        );
        for seed in [&old_a, &new_a, &b, &host.seed] {
            ensure!(
                !registry.contains(seed.as_str()),
                "a seed reached the registry"
            );
        }
        ensure!(
            std::fs::metadata(bus.registry_path())?.permissions().mode() & 0o777
                == PRIVATE_FILE_MODE,
            "the registry is not private"
        );

        // Unenrolled, worker-a can no longer connect; worker-b is untouched.
        unenroll(bus, agents_path, "worker-a").await?;
        ensure!(
            refused(&url, &new_a).await,
            "an unenrolled key still connects"
        );
        delivered(&host_channel, &mut b_inbox, "worker-b", "after unenroll").await?;
        ensure!(
            b_drops.load(Ordering::SeqCst) == 0,
            "worker-b was disconnected by the unenroll"
        );
        // Unenrolling again changes nothing; the agents file's names are not enrolled.
        unenroll(bus, agents_path, "worker-a").await?;
        ensure!(
            enroll(bus, agents_path, "host", &host.seed).await.is_err(),
            "the host's name was enrolled"
        );
        drop(b_inbox);
        drop(b_channel);

        failed_enrollment(bus, agents_path, host, &b).await?;

        // A restarted bus renders the enrolled guests too.
        start(bus, agents_path, &host.seed).await?;
        Channel::connect(&url, &b).await?;
        ensure!(
            refused(&url, &new_a).await,
            "the restart brought back an unenrolled key"
        );

        unenroll_while_stopped(bus, agents_path, host, &b).await
    }

    /// An enrollment that fails, here because the config cannot be written, leaves the registry
    /// as it was and the previous key of `worker-b`, `b`, accepted.
    async fn failed_enrollment(
        bus: &Bus,
        agents_path: &Path,
        host: &NewKey,
        b: &str,
    ) -> Result<()> {
        let url = bus.endpoint();
        // The config cannot be written: a directory stands where it goes.
        let before = std::fs::read_to_string(bus.registry_path())?;
        std::fs::remove_file(bus.config_path())?;
        std::fs::create_dir(bus.config_path())?;
        let error = enroll(bus, agents_path, "worker-b", &host.seed)
            .await
            .err()
            .context("enrolled with a config that cannot be written")?;
        ensure!(format!("{error:#}").contains("bus start"), "{error:#}");
        ensure!(
            std::fs::read_to_string(bus.registry_path())? == before,
            "the failed enrollment changed the registry"
        );
        ensure!(
            !refused(&url, b).await,
            "the failed enrollment refused the previous key"
        );
        std::fs::remove_dir(bus.config_path())?;

        Ok(())
    }

    /// `bus stop`, run twice, and `unenroll` of `worker-b`, whose seed is `b`, with the bus
    /// stopped: the next start refuses the key.
    async fn unenroll_while_stopped(
        bus: &Bus,
        agents_path: &Path,
        host: &NewKey,
        b: &str,
    ) -> Result<()> {
        let url = bus.endpoint();
        // `bus stop` twice: the second finds no container, and still succeeds.
        let docker = docker_connection()?;
        stop_bus(&docker, &bus.container, &bus.record_path()).await?;
        ensure!(!bus.record_path().exists(), "the record was kept");
        stop_bus(&docker, &bus.container, &bus.record_path()).await?;

        // With the bus stopped and its record gone, a key can still be revoked: the next start
        // refuses it.
        unenroll_in(
            &bus.container,
            &bus.volume,
            bus.state_dir.clone(),
            "worker-b",
        )
        .await?;
        ensure!(
            !std::fs::read_to_string(bus.registry_path())?.contains(&public_key_of(b)?),
            "the registry kept the unenrolled key"
        );
        start(bus, agents_path, &host.seed).await?;
        ensure!(
            refused(&url, b).await,
            "a key unenrolled while the bus was stopped connects"
        );
        Ok(())
    }

    /// Needs Docker. Starts a throwaway bus on 127.0.0.1 with only the host in the agents file,
    /// and enrolls, re-enrolls and unenrolls guests on it.
    /// Run with `cargo test -p agent-bus -- --ignored`.
    #[tokio::test]
    #[ignore = "needs Docker"]
    async fn enrolled_guests_get_their_own_keys_and_old_keys_are_refused() {
        install_crypto_provider();
        let host = generate().unwrap();
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("agent-bus-enroll-it-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let agents_path = dir.join("agents.toml");
        std::fs::write(
            &agents_path,
            format!(
                "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n",
                host.public_key
            ),
        )
        .unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let bus = Bus {
            container: format!("sherpa-agent-bus-enroll-test-{id}"),
            volume: format!("sherpa-agent-bus-enroll-test-{id}-data"),
            listen: Ipv4Addr::LOCALHOST,
            port,
            state_dir: dir.join("state"),
            lab: None,
        };

        let result = async {
            start(&bus, &agents_path, &host.seed).await?;
            enrollment(&bus, &agents_path, &host).await
        }
        .await;

        let docker = docker_connection().unwrap();
        let _ = remove_container(&docker, &bus.container).await;
        let _ = docker
            .remove_volume(&bus.volume, None::<RemoveVolumeOptions>)
            .await;
        let _ = std::fs::remove_dir_all(&dir);
        result.unwrap();
    }
    /// The bus takes TLS from the first byte, from a client that trusts its CA: a plaintext
    /// client gets nothing, not even the server's INFO, and is dropped; a client trusting
    /// another CA refuses the server; the bus's CA and a known key connect.
    async fn tls_only(bus: &Bus, host: &NewKey) -> Result<()> {
        let server = bus.endpoint();
        let address = SocketAddrV4::new(bus.listen, bus.port);

        // A raw plaintext connection: the server waits for a TLS ClientHello and says nothing.
        let mut raw = TcpStream::connect(address).await?;
        raw.write_all(b"CONNECT {}\r\nPING\r\n").await?;
        let mut heard = Vec::new();
        let _ = timeout(Duration::from_secs(5), raw.read_to_end(&mut heard)).await;
        let heard = String::from_utf8_lossy(&heard);
        ensure!(
            !heard.contains("INFO") && !heard.contains("PONG"),
            "the bus answered a plaintext client: {heard}"
        );

        // A NATS client that does not use TLS, with the host's own key, is refused.
        let plaintext = timeout(
            Duration::from_secs(15),
            ConnectOptions::with_nkey(host.seed.clone()).connect(&server.url),
        )
        .await;
        ensure!(
            !matches!(plaintext, Ok(Ok(_))),
            "a plaintext client connected"
        );

        // A client trusting another CA, for the same address, refuses the server.
        let stranger = bus.state_dir.join("stranger");
        let stranger_files = TlsFiles::new(&stranger.join(TLS_DIR), &stranger.join("nats"));
        tls::ensure(&stranger_files, &[IpAddr::V4(bus.listen)])?;
        let other_ca = Endpoint {
            url: server.url.clone(),
            ca: stranger_files.ca_cert.clone(),
        };
        let error = match timeout(
            Duration::from_secs(15),
            Channel::connect(&other_ca, &host.seed),
        )
        .await
        {
            Ok(Ok(_)) => bail!("a client trusting another CA connected"),
            Ok(Err(error)) => format!("{error:#}"),
            Err(_) => "timed out".to_owned(),
        };
        ensure!(
            error.contains("invalid peer certificate"),
            "another CA was refused, but not for its certificate: {error}"
        );

        // No CA file at all: refused before connecting, naming the path and the variable.
        let missing = Endpoint {
            url: server.url.clone(),
            ca: stranger.join("missing.pem"),
        };
        let error = match Channel::connect(&missing, &host.seed).await {
            Ok(_) => bail!("a client with no CA connected"),
            Err(error) => format!("{error:#}"),
        };
        ensure!(
            error.contains("missing.pem") && error.contains(tls::CA_ENV),
            "{error}"
        );

        // The bus's CA and a known key: connected, and it works.
        let channel = Channel::connect(&server, &host.seed).await?;
        channel.send(&message("host", "host", "over TLS")).await?;
        Ok(())
    }

    /// Starts a real bus and checks it takes TLS only, from clients trusting its own CA; then
    /// that a later start keeps the CA, and a start on another address issues a certificate
    /// for it that the same CA vouches for.
    /// Run with `cargo test -p agent-bus -- --ignored`.
    #[tokio::test]
    #[ignore = "needs Docker"]
    async fn the_bus_takes_only_tls_from_clients_trusting_its_ca() {
        install_crypto_provider();
        let host = generate().unwrap();
        let id = std::process::id();
        let dir = std::env::temp_dir().join(format!("agent-bus-tls-it-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let agents_path = dir.join("agents.toml");
        std::fs::write(
            &agents_path,
            format!(
                "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n",
                host.public_key
            ),
        )
        .unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut bus = Bus {
            container: format!("sherpa-agent-bus-tls-test-{id}"),
            volume: format!("sherpa-agent-bus-tls-test-{id}-data"),
            listen: Ipv4Addr::LOCALHOST,
            port,
            state_dir: dir.join("state"),
            lab: None,
        };

        let result = async {
            start(&bus, &agents_path, &host.seed).await?;
            tls_only(&bus, &host).await?;
            let ca = std::fs::read_to_string(bus.endpoint().ca)?;
            let server = std::fs::read_to_string(bus.tls_files().server_cert)?;

            // Started again on the same address: the same CA and certificate.
            start(&bus, &agents_path, &host.seed).await?;
            ensure!(
                std::fs::read_to_string(bus.endpoint().ca)? == ca,
                "a new CA"
            );
            ensure!(
                std::fs::read_to_string(bus.tls_files().server_cert)? == server,
                "a new server certificate for the same address"
            );

            // On another loopback address: a new certificate, the same CA.
            bus.listen = Ipv4Addr::new(127, 0, 0, 2);
            start(&bus, &agents_path, &host.seed).await?;
            ensure!(
                std::fs::read_to_string(bus.endpoint().ca)? == ca,
                "a new CA"
            );
            ensure!(
                std::fs::read_to_string(bus.tls_files().server_cert)? != server,
                "no new server certificate for the new address"
            );
            Channel::connect(&bus.endpoint(), &host.seed).await?;
            Ok(())
        }
        .await;

        let docker = docker_connection().unwrap();
        let _ = remove_container(&docker, &bus.container).await;
        let _ = docker
            .remove_volume(&bus.volume, None::<RemoveVolumeOptions>)
            .await;
        let _ = std::fs::remove_dir_all(&dir);
        result.unwrap();
    }
}
