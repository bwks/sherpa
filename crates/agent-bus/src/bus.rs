//! The bus itself: a NATS server with `JetStream`, in a container on the host.
//!
//! It uses the host's network and listens on one address, such as a lab's management bridge, so
//! only that lab's guests and the host reach it. Every agent authenticates with an `NKey`, and the
//! server config that says what each may do is rendered from the agents file (see `config`).

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, ContainerStateStatusEnum, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum,
};
use bollard::query_parameters::{CreateContainerOptions, CreateImageOptionsBuilder};
use container::{
    container_status, docker_connection, remove_container, start_container, stop_container,
};
use futures_util::TryStreamExt;

use crate::agents::Agents;
use crate::channel::Channel;
use crate::config;
use crate::keys::public_key_of;
use crate::state::{self, BusRecord, LabRecord};

/// The bus container.
const CONTAINER_NAME: &str = "sherpa-agent-bus";
/// `nats:2.15-alpine`, nats-server 2.15.2.
const IMAGE: &str = "nats@sha256:afae2fdbe2e326c65e1246cb83b9d05d319188734f3bd577ac8072daae78a10f";
/// The volume holding `JetStream`'s stored messages; it outlives the container.
const VOLUME: &str = "sherpa-agent-bus-data";
const DATA_DIR: &str = "/data";
/// Where the rendered server config is mounted, read-only.
const CONFIG_TARGET: &str = "/etc/agent-bus/nats.conf";
const CONFIG_FILE: &str = "nats.conf";
/// Where `start` records the bus, next to the config.
const RECORD_FILE: &str = "bus.toml";
const STATE_SUBDIR: &str = "agent-bus";
const PORT: u16 = 4222;
/// How often, and how long, `start` waits for the server to accept connections.
const CONNECT_ATTEMPTS: u32 = 20;
const CONNECT_DELAY: Duration = Duration::from_millis(500);

/// Where and how the bus runs.
pub struct Bus {
    pub container: String,
    pub volume: String,
    pub listen: Ipv4Addr,
    pub port: u16,
    /// Where the server config is written; outside the repository.
    pub config_path: PathBuf,
    /// Where `start` records where the bus is.
    pub record_path: PathBuf,
    /// The lab the bus serves, when started for one.
    pub lab: Option<LabRecord>,
}

impl Bus {
    /// The bus as users run it: the standard container, volume and port, listening on
    /// `listen`, with its config and record under `$XDG_STATE_HOME/agent-bus/`.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub fn standard(listen: Ipv4Addr, lab: Option<LabRecord>) -> Result<Self> {
        let state = standard_state_dir()?;
        Ok(Self {
            container: CONTAINER_NAME.to_owned(),
            volume: VOLUME.to_owned(),
            listen,
            port: PORT,
            config_path: state.join(CONFIG_FILE),
            record_path: state.join(RECORD_FILE),
            lab,
        })
    }

    /// Where `start` records the bus and the other commands look for it.
    ///
    /// # Errors
    ///
    /// Fails when neither `XDG_STATE_HOME` nor `HOME` gives a state directory.
    pub fn record_path() -> Result<PathBuf> {
        Ok(standard_state_dir()?.join(RECORD_FILE))
    }

    fn url(&self) -> String {
        format!("nats://{}:{}", self.listen, self.port)
    }
}

/// Start the bus for the agents in `agents_path`, and prepare it. This renders the server
/// config, replaces any existing bus container (the config may have changed; the messages live
/// in the volume), then connects as the host with `host_seed` and creates the inbox stream and
/// every agent's inbox.
///
/// # Errors
///
/// Fails when the agents file is not valid, `host_seed` is not the host's key, or Docker or the
/// bus fails.
pub async fn start(bus: &Bus, agents_path: &Path, host_seed: &str) -> Result<()> {
    let agents = Agents::load(agents_path)?;
    ensure!(
        public_key_of(host_seed)? == agents.host.key,
        "AGENT_BUS_SEED is not the seed of the host '{}' in the agents file",
        agents.host.name
    );
    config::write(&bus.config_path, &agents)?;
    println!("Wrote {}", bus.config_path.display());

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

    let channel = connect_when_ready(bus, host_seed).await?;
    let outcomes = channel.create_inboxes(&agents).await?;
    state::write(
        &bus.record_path,
        &BusRecord {
            url: bus.url(),
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

/// Stop and remove the bus container. The stored messages stay in their volume.
///
/// # Errors
///
/// Fails when Docker cannot stop or remove the container.
pub async fn stop() -> Result<()> {
    let docker = docker_connection()?;
    stop_container(&docker, CONTAINER_NAME).await?;
    remove_container(&docker, CONTAINER_NAME).await?;
    state::remove(&Bus::record_path()?)?;
    println!("Removed {CONTAINER_NAME}; stored messages are kept in {VOLUME}");
    Ok(())
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
    match state::read(&Bus::record_path()?)? {
        Some(record) => {
            println!("URL: {}", record.url);
            match record.lab {
                Some(lab) => println!("Lab: {} ({})", lab.name, lab.id),
                None => println!("Lab: none (started with --listen)"),
            }
        }
        None => println!("URL: not recorded (run `agent-bus bus start`)"),
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

/// Connect as the host, waiting for the freshly started server to accept connections.
async fn connect_when_ready(bus: &Bus, seed: &str) -> Result<Channel> {
    let url = bus.url();
    let mut last = None;
    for _ in 0..CONNECT_ATTEMPTS {
        match Channel::connect(&url, seed).await {
            Ok(channel) => return Ok(channel),
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(CONNECT_DELAY).await;
    }
    match last {
        Some(error) => Err(error).context("The bus did not accept the host's connection"),
        None => bail!("The bus did not accept the host's connection"),
    }
}

/// The state directory of this user, from the environment.
pub(crate) fn standard_state_dir() -> Result<PathBuf> {
    state_dir(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// The directory for the bus's state: `$XDG_STATE_HOME/agent-bus`, or
/// `~/.local/state/agent-bus`. A relative `XDG_STATE_HOME` is ignored, as the spec says.
fn state_dir(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let base = match xdg_state_home.map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => {
            let home = home
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .context("Neither XDG_STATE_HOME nor HOME is set to an absolute path")?;
            home.join(".local").join("state")
        }
    };
    Ok(base.join(STATE_SUBDIR))
}

fn container_config(bus: &Bus) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(IMAGE.to_owned()),
        cmd: Some(vec![
            "-c".to_owned(),
            CONFIG_TARGET.to_owned(),
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
                    target: Some(CONFIG_TARGET.to_owned()),
                    source: Some(bus.config_path.display().to_string()),
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
    use std::time::Duration;

    use bollard::query_parameters::RemoveVolumeOptions;
    use tokio::time::timeout;

    use super::*;
    use crate::keys::{NewKey, generate};
    use crate::message::{Kind, Message, TaskUsage, Usage};
    use crate::report;

    fn test_bus(config_path: PathBuf) -> Bus {
        Bus {
            container: "sherpa-agent-bus-test".to_owned(),
            volume: "sherpa-agent-bus-test-data".to_owned(),
            listen: Ipv4Addr::new(172, 31, 3, 1),
            port: 4222,
            record_path: config_path.with_file_name(RECORD_FILE),
            config_path,
            lab: None,
        }
    }

    #[test]
    fn the_bus_listens_only_on_the_given_address_with_jetstream_and_its_config() {
        let bus = test_bus(PathBuf::from("/home/u/.local/state/agent-bus/nats.conf"));
        let config = container_config(&bus);
        let command = config.cmd.unwrap();
        assert_eq!(command[..2], ["-c", CONFIG_TARGET]);
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
        assert_eq!(
            mounts[1].source.as_deref(),
            Some("/home/u/.local/state/agent-bus/nats.conf")
        );
        assert_eq!(mounts[1].target.as_deref(), Some(CONFIG_TARGET));
        assert_eq!(mounts[1].read_only, Some(true));
    }

    #[test]
    fn the_state_dir_follows_xdg_and_falls_back_to_home() {
        let dir = |xdg: Option<&str>, home: Option<&str>| {
            state_dir(xdg.map(OsString::from), home.map(OsString::from))
        };
        assert_eq!(
            dir(Some("/x/state"), Some("/home/u")).unwrap(),
            PathBuf::from("/x/state/agent-bus")
        );
        assert_eq!(
            dir(None, Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/agent-bus")
        );
        assert_eq!(
            dir(Some("relative"), Some("/home/u")).unwrap(),
            PathBuf::from("/home/u/.local/state/agent-bus")
        );
        assert!(dir(None, None).is_err());
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
        }
    }

    /// What the integration test shows, run against a started bus.
    async fn exercise(bus: &Bus, agents: &Agents, host: &NewKey, dev: &NewKey) -> Result<()> {
        let url = bus.url();
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
            timeout(wait, dev_channel.create_inboxes(agents))
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

    /// A guest sends progress messages and then a completed one with usage; a host listener
    /// must get every one, also after it restarts.
    async fn reply_burst(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let wait = Duration::from_secs(15);
        let mut host_channel = Channel::connect(&bus.url(), &host.seed).await?;
        let dev_channel = Channel::connect(&bus.url(), &dev.seed).await?;
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
            let second = Channel::connect(&bus.url(), &host.seed).await?;
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
            host_channel = Channel::connect(&bus.url(), &host.seed).await?;
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

    /// The host reads the outcomes still kept in the stream, with its own permissions, and the
    /// read leaves its inbox as it was.
    async fn usage_history(bus: &Bus, host: &NewKey, dev: &NewKey) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.url(), &host.seed).await?;
        let dev_channel = Channel::connect(&bus.url(), &dev.seed).await?;
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

    /// An inbox made by an earlier version (no `max_waiting`) is upgraded by `bus start`: what
    /// was acknowledged stays gone and what was not is delivered exactly once.
    async fn upgrade_old_inbox(
        bus: &Bus,
        agents_path: &Path,
        host: &NewKey,
        other: &NewKey,
    ) -> Result<()> {
        let wait = Duration::from_secs(15);
        let host_channel = Channel::connect(&bus.url(), &host.seed).await?;
        let other_channel = Channel::connect(&bus.url(), &other.seed).await?;
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

        let other_channel = Channel::connect(&bus.url(), &other.seed).await?;
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
        let host_channel = Channel::connect(&bus.url(), &host.seed).await?;
        ensure!(
            host_channel.waiting("other").await? == 0,
            "messages are left in the inbox"
        );
        // The upgraded inbox has the new setting: a second reader is refused.
        let mut first = other_channel.inbox("other").await?;
        // The reader asks the bus for messages only once it waits for one.
        let _ = timeout(Duration::from_secs(1), first.next()).await;
        let second = Channel::connect(&bus.url(), &other.seed).await?;
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
        std::fs::create_dir_all(&lab_dir).unwrap();
        std::fs::write(
            lab_dir.join("lab-info.toml"),
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
            config_path: dir.join("state").join(CONFIG_FILE),
            record_path: dir.join("state").join(RECORD_FILE),
            lab: Some(lab),
        };
        let agents = Agents::parse(&text).unwrap();

        let result = async {
            start(&bus, &agents_path, &host.seed).await?;
            // With no --server and no AGENT_BUS_URL, the recorded bus is found and connects.
            let found = state::find_server(None, None, &bus.record_path)?;
            ensure!(found == bus.url(), "the recorded URL is {found}");
            let recorded = state::read(&bus.record_path)?.context("no bus record")?;
            ensure!(
                recorded.lab.is_some_and(|lab| lab.name == "it"),
                "the lab was not recorded"
            );
            Channel::connect(&found, &host.seed).await?;
            exercise(&bus, &agents, &host, &dev).await?;

            // Every message a guest sends reaches a host listener: progress, then the outcome
            // with its usage.
            reply_burst(&bus, &host, &dev).await?;

            // `usage` reads the kept outcomes without consuming the host's inbox.
            usage_history(&bus, &host, &dev).await?;

            // A task waits through a restart: the container is replaced, the volume is kept.
            let host_channel = Channel::connect(&bus.url(), &host.seed).await?;
            host_channel.send(&message("host", "dev", "kept")).await?;
            drop(host_channel);
            start(&bus, &agents_path, &host.seed).await?;
            let dev_channel = Channel::connect(&bus.url(), &dev.seed).await?;
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
            upgrade_old_inbox(&bus, &agents_path, &host, &other).await
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
}
