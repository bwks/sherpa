//! The bus connection: one `JetStream` stream holds every agent's inbox, so a message waits until
//! its agent reads it.
//!
//! Every connection authenticates with an `NKey` seed. Only the host may create the stream and the
//! consumers (`create_inboxes`, run by `bus start`); every other agent opens the consumer that
//! exists for it.
//!
//! A message to `to` from `from` is published to `agents.<to>.<from>`, and each inbox takes
//! `agents.<name>.>`. The bus lets a guest publish only under its own name, and a reader drops a
//! message whose `from` is not the subject's, so a guest cannot pose as another. Each connection
//! takes its replies under its own prefix, `_INBOX_<public key>`, the only one its user may
//! subscribe to, so no agent reads another's replies.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use async_nats::jetstream::ErrorCode;
use async_nats::jetstream::consumer::FromConsumer;
use async_nats::jetstream::consumer::pull::MessagesErrorKind;
use async_nats::jetstream::context::{ConsumerInfoErrorKind, GetStreamErrorKind};
use async_nats::jetstream::kv::{self, Operation};
use async_nats::jetstream::{self, consumer, stream};
use async_nats::{ConnectOptions, Event};
use futures_util::StreamExt;

use crate::keys::public_key_of;
use crate::message::{Message, check_agent_name};
use crate::status::{BUCKET, Heartbeat, InboxState, Status};
use crate::tls::CA_ENV;

/// The stream holding every inbox.
pub const STREAM: &str = "AGENTS";
/// Each agent's inbox is `agents.<name>.>`, one subject per sender.
pub const SUBJECT_PREFIX: &str = "agents";
/// How long the stream keeps a message, read or not. Files that follow the messages (the
/// guest's task records, `results.jsonl`, the listener's journal) keep them as long.
pub const RETENTION: Duration = Duration::from_hours(7 * 24);

/// How long the bus keeps the consumer that reads the history after its last use.
const HISTORY_INACTIVE: Duration = Duration::from_secs(30);
/// How many messages one read of the history asks for.
const HISTORY_BATCH: usize = 256;
/// How long one read of the history waits for messages.
const HISTORY_WAIT: Duration = Duration::from_secs(5);

/// Where the bus is, and the CA certificate its server certificate must chain to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `nats://<address>:<port>`; the address must be one the server certificate names.
    pub url: String,
    pub ca: PathBuf,
}

/// A connection to the bus.
pub struct Channel {
    jetstream: jetstream::Context,
}

/// The messages sent to one agent, in order, kept by the bus until acknowledged.
pub struct Inbox {
    messages: consumer::pull::Stream,
}

/// What `create_inboxes` did to one inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxChange {
    Created,
    Updated,
    /// Deleted and created again, delivering from this stream sequence.
    Recreated {
        from_sequence: u64,
    },
    Unchanged,
}

impl fmt::Display for InboxChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Updated => write!(f, "updated"),
            Self::Recreated { from_sequence } => {
                write!(f, "recreated from sequence {from_sequence}")
            }
            Self::Unchanged => write!(f, "unchanged"),
        }
    }
}

/// How to bring an existing consumer to the wanted config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Keep,
    Update,
    Recreate,
}

/// A message read from an inbox; acknowledge it once handled.
pub struct Delivery {
    pub message: Message,
    raw: jetstream::Message,
}

impl Channel {
    /// Connect to the bus at `server`, authenticating with `seed`. The connection is TLS from
    /// its first byte, and the server's certificate must chain to `server.ca` and name the
    /// URL's address; no system root is trusted. Replies come back under the key's own inbox
    /// prefix (`inbox_prefix`). The connection's asynchronous errors are printed: a permission
    /// violation is reported only there, and the call that caused it just times out.
    ///
    /// # Errors
    ///
    /// Fails when the seed is not a user seed, there is no CA certificate, or the bus cannot be
    /// reached or does not prove it is the bus.
    pub async fn connect(server: &Endpoint, seed: &str) -> Result<Self> {
        let key = public_key_of(seed)?;
        let client = tls_options(ConnectOptions::with_nkey(seed.trim().to_owned()), server)?
            .custom_inbox_prefix(inbox_prefix(&key))
            .event_callback(|event| async move { report(&event) })
            .connect(&server.url)
            .await
            .with_context(|| {
                format!(
                    "Unable to reach the bus at {} over TLS, trusting the CA {}",
                    server.url,
                    server.ca.display()
                )
            })?;
        Ok(Self {
            jetstream: jetstream::new(client),
        })
    }

    /// Create the inbox stream, the status bucket, and an inbox consumer for every agent in
    /// `names`. This is the host's job: only the host may do it. It is safe to repeat.
    ///
    /// # Errors
    ///
    /// Fails when the bus refuses the stream or a consumer.
    pub async fn create_inboxes<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<(String, InboxChange)>> {
        let stream = self
            .jetstream
            .get_or_create_stream(stream::Config {
                name: STREAM.to_owned(),
                subjects: vec![format!("{SUBJECT_PREFIX}.>")],
                max_age: RETENTION,
                ..Default::default()
            })
            .await
            .context("Unable to create the inbox stream")?;
        self.jetstream
            .create_or_update_key_value(kv::Config {
                bucket: BUCKET.to_owned(),
                description: "The status of each guest agent, under its name".to_owned(),
                history: 1,
                max_age: RETENTION,
                ..Default::default()
            })
            .await
            .context("Unable to create the status bucket")?;
        let mut outcomes = Vec::new();
        for name in names {
            let wanted = consumer_config(name)?;
            let change = self
                .ensure_inbox(&stream, &wanted)
                .await
                .with_context(|| format!("Unable to prepare the inbox of '{name}'"))?;
            outcomes.push((name.to_owned(), change));
        }
        Ok(outcomes)
    }

    /// Bring one inbox consumer to the wanted config without losing or replaying messages.
    async fn ensure_inbox(
        &self,
        stream: &stream::Stream,
        wanted: &consumer::pull::Config,
    ) -> Result<InboxChange> {
        let name = wanted.durable_name.clone().unwrap_or_default();
        let info = match stream.consumer_info(&name).await {
            Ok(info) => info,
            Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => {
                stream
                    .create_consumer(wanted.clone())
                    .await
                    .context("Unable to create the inbox")?;
                return Ok(InboxChange::Created);
            }
            Err(error) => return Err(error).context("Unable to read the inbox"),
        };
        match plan(&info.config, wanted) {
            Plan::Keep => Ok(InboxChange::Unchanged),
            Plan::Update => {
                stream
                    .update_consumer(update_config(&info.config, wanted)?)
                    .await
                    .context("Unable to update the inbox")?;
                Ok(InboxChange::Updated)
            }
            Plan::Recreate => {
                // The bus cannot change this setting in place. Recreate the consumer so it starts
                // right after everything acknowledged so far: acknowledged messages stay gone,
                // the rest (also those delivered but not acknowledged) are delivered again.
                let from_sequence = info.ack_floor.stream_sequence + 1;
                let mut recreated = wanted.clone();
                recreated.deliver_policy = consumer::DeliverPolicy::ByStartSequence {
                    start_sequence: from_sequence,
                };
                stream
                    .delete_consumer(&name)
                    .await
                    .context("Unable to delete the old inbox")?;
                stream.create_consumer(recreated).await.with_context(|| {
                    format!(
                        "The old inbox was deleted but its replacement was not created. Create it \
                         from stream sequence {from_sequence} to lose nothing"
                    )
                })?;
                Ok(InboxChange::Recreated { from_sequence })
            }
        }
    }

    /// Put `message` in its receiver's inbox, and wait until the bus has stored it.
    ///
    /// # Errors
    ///
    /// Fails when a name is invalid or the bus does not store the message.
    pub async fn send(&self, message: &Message) -> Result<()> {
        let subject = message_subject(&message.to, &message.from)?;
        let payload = serde_json::to_vec(message).context("Unable to encode the message")?;
        self.jetstream
            .publish(subject, payload.into())
            .await
            .context("Unable to send the message")?
            .await
            .context("The bus did not store the message")?;
        Ok(())
    }

    /// Open `agent`'s inbox. The inbox remembers what was read, so a restarted reader goes on
    /// where it stopped.
    ///
    /// # Errors
    ///
    /// Fails when the name is invalid, or the inbox does not exist: the host creates it when it
    /// starts the bus with the agent in its agents file.
    pub async fn inbox(&self, agent: &str) -> Result<Inbox> {
        check_agent_name(agent)?;
        let mut reader = self
            .jetstream
            .get_consumer_from_stream::<consumer::pull::Config, _, _>(agent, STREAM)
            .await
            .with_context(|| {
                format!(
                    "Unable to open the inbox of '{agent}'. It exists only for an agent in the \
                     agents file the bus was started with, and only while the bus is running"
                )
            })?;
        let info = reader
            .info()
            .await
            .with_context(|| format!("Unable to read the inbox of '{agent}'"))?;
        ensure!(
            info.num_waiting == 0,
            "Another reader is attached to the inbox of '{agent}': readers share its messages, \
             so each would get only some. Stop the other reader (for example an earlier \
             `agent-bus listen {agent}`) and try again"
        );
        let messages = reader
            .messages()
            .await
            .with_context(|| format!("Unable to read the inbox of '{agent}'"))?;
        Ok(Inbox { messages })
    }

    /// Every message still kept in the stream for `agent`, oldest first, read without consuming
    /// the inbox: an ephemeral pull consumer with no acknowledgement reads them, and the bus drops
    /// it after use. The host's permissions already allow this (the `JetStream` API and its own
    /// inbox pulls), so nothing was widened. Messages that are not agent messages, or whose
    /// sender is not the subject's, are skipped.
    ///
    /// # Errors
    ///
    /// Fails when the name is invalid or the bus refuses the consumer or a read.
    pub async fn history(&self, agent: &str) -> Result<Vec<Message>> {
        let subject = inbox_filter(agent)?;
        let stream = self
            .jetstream
            .get_stream(STREAM)
            .await
            .context("Unable to open the inbox stream")?;
        let reader = stream
            .create_consumer(consumer::pull::Config {
                filter_subject: subject,
                ack_policy: consumer::AckPolicy::None,
                deliver_policy: consumer::DeliverPolicy::All,
                inactive_threshold: HISTORY_INACTIVE,
                ..Default::default()
            })
            .await
            .context("Unable to create a consumer to read the history")?;
        let result = read_all(&reader).await;
        // Best effort: an unused ephemeral consumer is dropped by the bus anyway.
        let _ = stream.delete_consumer(&reader.cached_info().name).await;
        result
    }

    /// Store `status` as `agent`'s, and wait until the bus has stored it. This is a plain
    /// publish to the bucket's subject for `agent`, the only status subject a guest may publish
    /// to; it needs no `JetStream` API call.
    ///
    /// # Errors
    ///
    /// Fails when the name is invalid, or the bus does not store the status (no status bucket:
    /// the bus was prepared by an older `agent-bus`).
    pub async fn put_status(&self, agent: &str, status: &Status) -> Result<()> {
        let subject = status_subject(agent)?;
        let payload = serde_json::to_vec(status).context("Unable to encode the status")?;
        self.jetstream
            .publish(subject, payload.into())
            .await
            .context("Unable to send the status")?
            .await
            .context("The bus did not store the status")?;
        Ok(())
    }

    /// The last status each agent in `names` stored, by name. `None` when the bus has no status
    /// bucket. A status that is not one is left out, and reported. This is for the host.
    ///
    /// # Errors
    ///
    /// Fails when the bucket cannot be read.
    pub async fn statuses<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Option<Vec<(String, Heartbeat)>>> {
        match self.jetstream.get_stream(format!("KV_{BUCKET}")).await {
            Ok(_) => {}
            Err(error)
                if matches!(error.kind(), GetStreamErrorKind::JetStream(ref inner)
                    if inner.error_code() == ErrorCode::STREAM_NOT_FOUND) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("Unable to open the status bucket"),
        }
        let store = self
            .jetstream
            .get_key_value(BUCKET)
            .await
            .context("Unable to open the status bucket")?;
        let mut found = Vec::new();
        for name in names {
            let entry = store
                .entry(name)
                .await
                .with_context(|| format!("Unable to read the status of '{name}'"))?;
            let Some(entry) = entry.filter(|entry| entry.operation == Operation::Put) else {
                continue;
            };
            match serde_json::from_slice::<Status>(&entry.value) {
                Ok(status) => found.push((
                    name.to_owned(),
                    Heartbeat {
                        status,
                        stored_ms: unix_ms(entry.created.unix_timestamp_nanos()),
                    },
                )),
                Err(error) => {
                    eprintln!("Skipping the status of '{name}', which is not one: {error}");
                }
            }
        }
        Ok(Some(found))
    }

    /// What the bus says about `agent`'s inbox consumer; `None` when it has none. This is for
    /// the host.
    ///
    /// # Errors
    ///
    /// Fails when the name is invalid or the consumer cannot be read.
    pub async fn inbox_state(&self, agent: &str) -> Result<Option<InboxState>> {
        check_agent_name(agent)?;
        let stream = self
            .jetstream
            .get_stream(STREAM)
            .await
            .context("Unable to open the inbox stream")?;
        let info = match stream.consumer_info(agent).await {
            Ok(info) => info,
            Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Unable to read the inbox of '{agent}'"));
            }
        };
        Ok(Some(InboxState {
            unread: info
                .num_pending
                .saturating_add(u64::try_from(info.num_ack_pending).unwrap_or(u64::MAX)),
            reader_attached: info.num_waiting > 0,
            last_delivered_ms: info
                .delivered
                .last_active
                .map(|at| unix_ms(at.unix_timestamp_nanos())),
        }))
    }

    /// Replace `agent`'s inbox with one made the way an earlier version made it, without
    /// `max_waiting`. This is for testing the upgrade.
    #[cfg(test)]
    pub async fn make_inbox_old(&self, agent: &str) -> Result<()> {
        let stream = self.jetstream.get_stream(STREAM).await?;
        stream.delete_consumer(agent).await?;
        let mut old = consumer_config(agent)?;
        old.max_waiting = 0;
        stream.create_consumer(old).await?;
        Ok(())
    }

    /// Replace `agent`'s inbox with one in the shape from before per-sender subjects (filtered
    /// on `agents.<agent>`) that was since recreated to start at the next stream sequence, as
    /// the `max_waiting` upgrade leaves it. Returns that start sequence. This is for testing the
    /// upgrade.
    #[cfg(test)]
    pub async fn make_inbox_unsplit(&self, agent: &str) -> Result<u64> {
        let mut stream = self.jetstream.get_stream(STREAM).await?;
        stream.delete_consumer(agent).await?;
        let start_sequence = stream.info().await?.state.last_sequence + 1;
        let mut old = consumer_config(agent)?;
        old.filter_subject = format!("{SUBJECT_PREFIX}.{agent}");
        old.deliver_policy = consumer::DeliverPolicy::ByStartSequence { start_sequence };
        stream.create_consumer(old).await?;
        Ok(start_sequence)
    }

    /// The config the bus holds for `agent`'s inbox. This is for testing the upgrade.
    #[cfg(test)]
    pub async fn inbox_config(&self, agent: &str) -> Result<consumer::Config> {
        let stream = self.jetstream.get_stream(STREAM).await?;
        Ok(stream.consumer_info(agent).await?.config)
    }

    /// How many messages wait in `agent`'s inbox, unread or unacknowledged. This is for the host.
    ///
    /// # Errors
    ///
    /// Fails when the inbox cannot be read.
    #[cfg(test)]
    pub async fn waiting(&self, agent: &str) -> Result<u64> {
        check_agent_name(agent)?;
        let mut reader = self
            .jetstream
            .get_consumer_from_stream::<consumer::pull::Config, _, _>(agent, STREAM)
            .await
            .with_context(|| format!("Unable to open the inbox of '{agent}'"))?;
        let info = reader
            .info()
            .await
            .with_context(|| format!("Unable to read the inbox of '{agent}'"))?;
        Ok(info.num_pending + u64::try_from(info.num_ack_pending).unwrap_or(u64::MAX))
    }
}

impl Inbox {
    /// Wait for the next message. A message that is not one of ours, or whose `from` is not
    /// the sender its subject names, is reported, acknowledged and skipped. An error the reader
    /// recovers from (`recoverable`), such as the bus being away for a while, is reported and
    /// reading goes on.
    ///
    /// # Errors
    ///
    /// Fails when the reader cannot go on: the inbox was deleted, or the bus refuses its pulls.
    pub async fn next(&mut self) -> Result<Option<Delivery>> {
        while let Some(raw) = self.messages.next().await {
            let raw = match raw {
                Ok(raw) => raw,
                Err(error) if recoverable(error.kind()) => {
                    eprintln!("Reading the inbox: {error}; still reading");
                    continue;
                }
                Err(error) => {
                    eprintln!("Reading the inbox: {error}; the reader cannot go on");
                    return Err(error).context("Unable to read the inbox");
                }
            };
            match serde_json::from_slice::<Message>(&raw.payload) {
                Ok(message) if sent_by(raw.subject.as_str(), &message) => {
                    return Ok(Some(Delivery { message, raw }));
                }
                Ok(message) => eprintln!(
                    "Refusing a message on {} that says it is from '{}' to '{}': the sender is not \
                     the subject's",
                    raw.subject, message.from, message.to
                ),
                Err(error) => {
                    eprintln!("Skipping a message that is not an agent message: {error}");
                }
            }
            raw.ack()
                .await
                .map_err(anyhow::Error::from_boxed)
                .context("Unable to acknowledge a skipped message")?;
        }
        Ok(None)
    }
}

impl Delivery {
    /// Tell the bus this message is handled, so it is not delivered again.
    ///
    /// # Errors
    ///
    /// Fails when the bus does not take the acknowledgement.
    pub async fn ack(&self) -> Result<()> {
        self.raw
            .ack()
            .await
            .map_err(anyhow::Error::from_boxed)
            .context("Unable to acknowledge the message")
    }
}

/// Read everything `reader` has pending.
async fn read_all(reader: &consumer::PullConsumer) -> Result<Vec<Message>> {
    let mut pending = reader.cached_info().num_pending;
    let mut messages = Vec::new();
    while pending > 0 {
        let mut batch = reader
            .fetch()
            .max_messages(HISTORY_BATCH)
            .expires(HISTORY_WAIT)
            .messages()
            .await
            .context("Unable to read the history")?;
        let mut read: u64 = 0;
        while let Some(raw) = batch.next().await {
            let raw = raw
                .map_err(anyhow::Error::from_boxed)
                .context("Unable to read the history")?;
            read = read.saturating_add(1);
            if let Ok(message) = serde_json::from_slice::<Message>(&raw.payload)
                && sent_by(raw.subject.as_str(), &message)
            {
                messages.push(message);
            }
        }
        if read == 0 {
            break;
        }
        pending = pending.saturating_sub(read);
    }
    Ok(messages)
}

/// The consumer behind `agent`'s inbox.
fn consumer_config(agent: &str) -> Result<consumer::pull::Config> {
    Ok(consumer::pull::Config {
        durable_name: Some(agent.to_owned()),
        filter_subject: inbox_filter(agent)?,
        // One reader at a time. Readers of a durable consumer share its messages, so a second
        // reader (a listener that was never stopped) would silently take some of them. The
        // bus refuses the second reader's pull request instead.
        max_waiting: 1,
        ..Default::default()
    })
}

/// Compare an existing consumer with the wanted one. Only the settings we choose are compared;
/// where the consumer starts is fixed when it is created, so a recreated consumer is not
/// recreated again.
fn plan(existing: &consumer::Config, wanted: &consumer::pull::Config) -> Plan {
    // The bus cannot update `max_waiting` on an existing consumer.
    if existing.max_waiting != wanted.max_waiting {
        Plan::Recreate
    } else if existing.filter_subject != wanted.filter_subject {
        Plan::Update
    } else {
        Plan::Keep
    }
}

/// The config that updates `existing` to `wanted`: the existing config with only the settings
/// we manage (`filter_subject`, `max_waiting`) taken from `wanted`. The bus refuses to change
/// what is fixed when a consumer is created (where it starts, its ack and replay policies), and
/// a recreated consumer starts at a sequence, not with the default `DeliverPolicy::All`, so
/// everything else is sent back as the bus has it.
///
/// # Errors
///
/// Fails when the existing consumer is not a pull consumer.
fn update_config(
    existing: &consumer::Config,
    wanted: &consumer::pull::Config,
) -> Result<consumer::pull::Config> {
    let mut updated = consumer::pull::Config::try_from_consumer_config(existing.clone())
        .map_err(|error| anyhow!(error))
        .context("The existing inbox is not a pull consumer")?;
    updated.filter_subject.clone_from(&wanted.filter_subject);
    updated.max_waiting = wanted.max_waiting;
    Ok(updated)
}

/// `options` made to connect to `server` with TLS first, trusting only the CA certificate
/// `server.ca`. The bus refuses a client that does not start with a TLS handshake.
///
/// rustls takes its crypto provider from the process default, which `main` installs.
///
/// # Errors
///
/// Fails when there is no CA certificate at `server.ca`.
pub fn tls_options(options: ConnectOptions, server: &Endpoint) -> Result<ConnectOptions> {
    ensure!(
        server.ca.is_file(),
        "No bus CA certificate at {}. On the host, `agent-bus bus start` makes it; a guest gets \
         it from dev/agent-bus/start-agent.sh; elsewhere, set {} to a copy of the host's",
        server.ca.display(),
        CA_ENV
    );
    Ok(options
        .tls_first()
        .require_tls(true)
        .add_root_certificates(server.ca.clone()))
}

/// Print a connection event that is not routine.
fn report(event: &Event) {
    match event {
        Event::Connected | Event::Draining | Event::Closed => {}
        other => eprintln!("Bus connection event: {other}"),
    }
}

/// The subject of `agent`'s status in the status bucket.
pub fn status_subject(agent: &str) -> Result<String> {
    check_agent_name(agent)?;
    Ok(format!("$KV.{BUCKET}.{agent}"))
}

/// Nanoseconds since the Unix epoch as milliseconds; 0 before it.
fn unix_ms(nanos: i128) -> u64 {
    u64::try_from(nanos / 1_000_000).unwrap_or(0)
}

/// The prefix of the subjects the connection of the user with `public_key` takes its replies
/// on. Its user may subscribe to `<prefix>.>` and nothing else.
pub fn inbox_prefix(public_key: &str) -> String {
    format!("_INBOX_{public_key}")
}

/// The subject a message to `to` from `from` is published to.
pub fn message_subject(to: &str, from: &str) -> Result<String> {
    check_agent_name(to)?;
    check_agent_name(from)?;
    Ok(format!("{SUBJECT_PREFIX}.{to}.{from}"))
}

/// The subjects of `agent`'s inbox: one per sender.
fn inbox_filter(agent: &str) -> Result<String> {
    check_agent_name(agent)?;
    Ok(format!("{SUBJECT_PREFIX}.{agent}.>"))
}

/// Whether `message`, published to `subject`, was sent there by the agent it names: its
/// subject is the one its `to` and `from` give. The bus lets a guest publish only under its own
/// name, so this holds a guest's `from` to the key it connected with.
fn sent_by(subject: &str, message: &Message) -> bool {
    message_subject(&message.to, &message.from).is_ok_and(|expected| expected == subject)
}

/// Whether an inbox's pull stream goes on after an error of this kind. async-nats ends the
/// stream after a deleted consumer, a push consumer, or a pull over the consumer's limits. After
/// any other error (a missed heartbeat, a pull that could not be sent, no responders while the
/// bus is away) it keeps pulling, and pulls again once it has reconnected.
fn recoverable(kind: MessagesErrorKind) -> bool {
    !matches!(
        kind,
        MessagesErrorKind::ConsumerDeleted
            | MessagesErrorKind::PushBasedConsumer
            | MessagesErrorKind::RequestLimitExceeded
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::keys::generate;
    use crate::message::Kind;

    fn message_on(from: &str, to: &str) -> Message {
        Message {
            task: "t1".to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind: Kind::Progress,
            text: String::new(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    #[test]
    fn each_sender_has_its_own_subject_in_each_inbox() {
        assert_eq!(message_subject("host", "dev").unwrap(), "agents.host.dev");
        assert_eq!(inbox_filter("dev").unwrap(), "agents.dev.>");
        assert!(inbox_filter("dev.>").is_err());
        assert!(message_subject("host", "dev.*").is_err());
        assert!(message_subject("host.>", "dev").is_err());
    }

    #[test]
    fn a_message_whose_sender_is_not_the_subject_s_is_refused() {
        assert!(sent_by("agents.host.dev", &message_on("dev", "host")));
        // A guest that may publish only as dev claims to be another.
        assert!(!sent_by("agents.host.dev", &message_on("other", "host")));
        assert!(!sent_by("agents.host.dev", &message_on("dev", "other")));
        // A subject without a sender, or with more than one.
        assert!(!sent_by("agents.host", &message_on("dev", "host")));
        assert!(!sent_by("agents.host.dev.x", &message_on("dev", "host")));
    }

    #[test]
    fn each_key_has_its_own_reply_prefix() {
        let key = generate().unwrap().public_key;
        assert_eq!(inbox_prefix(&key), format!("_INBOX_{key}"));
        assert_ne!(
            inbox_prefix(&key),
            inbox_prefix(&generate().unwrap().public_key)
        );
    }

    #[test]
    fn only_errors_that_end_the_pull_stream_end_the_reader() {
        for kind in [
            MessagesErrorKind::MissingHeartbeat,
            MessagesErrorKind::Pull,
            MessagesErrorKind::NoResponders,
            MessagesErrorKind::Other,
        ] {
            assert!(recoverable(kind), "{kind}");
        }
        for kind in [
            MessagesErrorKind::ConsumerDeleted,
            MessagesErrorKind::PushBasedConsumer,
            MessagesErrorKind::RequestLimitExceeded,
        ] {
            assert!(!recoverable(kind), "{kind}");
        }
    }

    #[test]
    fn each_agent_has_its_own_status_key() {
        assert_eq!(status_subject("dev").unwrap(), "$KV.AGENT_STATUS.dev");
        assert!(status_subject("dev.*").is_err());
    }

    #[test]
    fn bus_times_become_milliseconds() {
        assert_eq!(unix_ms(1_700_000_000_123_456_789), 1_700_000_000_123);
        assert_eq!(unix_ms(-5), 0);
    }

    fn existing(max_waiting: i64, filter: &str) -> consumer::Config {
        consumer::Config {
            max_waiting,
            filter_subject: filter.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn an_inbox_takes_every_sender_s_subject() {
        assert_eq!(
            consumer_config("dev").unwrap().filter_subject,
            "agents.dev.>"
        );
    }

    #[test]
    fn an_unchanged_consumer_is_kept_whatever_its_start_policy() {
        let wanted = consumer_config("dev").unwrap();
        let mut current = existing(1, "agents.dev.>");
        assert_eq!(plan(&current, &wanted), Plan::Keep);
        current.deliver_policy = consumer::DeliverPolicy::ByStartSequence { start_sequence: 7 };
        assert_eq!(plan(&current, &wanted), Plan::Keep);
    }

    #[test]
    fn an_updatable_difference_is_updated() {
        let wanted = consumer_config("dev").unwrap();
        assert_eq!(plan(&existing(1, "agents.old"), &wanted), Plan::Update);
    }

    #[test]
    fn an_update_keeps_where_the_consumer_starts() {
        let wanted = consumer_config("dev").unwrap();
        // An inbox in the old shape, recreated earlier at a start sequence.
        let mut current = existing(1, "agents.dev");
        current.durable_name = Some("dev".to_owned());
        current.deliver_policy = consumer::DeliverPolicy::ByStartSequence { start_sequence: 7 };
        current.replay_policy = consumer::ReplayPolicy::Original;
        current.ack_policy = consumer::AckPolicy::All;
        assert_eq!(plan(&current, &wanted), Plan::Update);
        let updated = update_config(&current, &wanted).unwrap();
        // What the bus fixes at creation is kept.
        assert_eq!(
            updated.deliver_policy,
            consumer::DeliverPolicy::ByStartSequence { start_sequence: 7 }
        );
        assert_eq!(updated.replay_policy, consumer::ReplayPolicy::Original);
        assert_eq!(updated.ack_policy, consumer::AckPolicy::All);
        // What we manage takes the wanted value.
        assert_eq!(updated.durable_name.as_deref(), Some("dev"));
        assert_eq!(updated.filter_subject, "agents.dev.>");
        assert_eq!(updated.max_waiting, 1);
    }

    #[test]
    fn a_max_waiting_difference_is_recreated() {
        let wanted = consumer_config("dev").unwrap();
        assert_eq!(
            plan(&existing(512, "agents.dev.>"), &wanted),
            Plan::Recreate
        );
        // A difference that cannot be updated wins over one that can.
        assert_eq!(plan(&existing(512, "agents.old"), &wanted), Plan::Recreate);
    }

    #[test]
    fn the_changes_read_for_the_log() {
        assert_eq!(InboxChange::Created.to_string(), "created");
        assert_eq!(InboxChange::Updated.to_string(), "updated");
        assert_eq!(InboxChange::Unchanged.to_string(), "unchanged");
        assert_eq!(
            InboxChange::Recreated { from_sequence: 5 }.to_string(),
            "recreated from sequence 5"
        );
    }
}
