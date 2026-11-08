//! The bus connection: one `JetStream` stream holds every agent's inbox, so a message waits until
//! its agent reads it.
//!
//! Every connection authenticates with an `NKey` seed. Only the host may create the stream and the
//! consumers (`create_inboxes`, run by `bus start`); every other agent opens the consumer that
//! exists for it.

use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use async_nats::jetstream::context::ConsumerInfoErrorKind;
use async_nats::jetstream::{self, consumer, stream};
use async_nats::{ConnectOptions, Event};
use futures_util::StreamExt;

use crate::agents::Agents;
use crate::keys::public_key_of;
use crate::message::{Message, check_agent_name};

/// The stream holding every inbox.
pub const STREAM: &str = "AGENTS";
/// Each agent's inbox is `agents.<name>`.
pub const SUBJECT_PREFIX: &str = "agents";
/// How long an unread message is kept.
const MAX_AGE: Duration = Duration::from_hours(7 * 24);

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
    /// Connect to the bus at `server`, authenticating with `seed`. The connection's
    /// asynchronous errors are printed: a permission violation is reported only there, and the
    /// call that caused it just times out.
    ///
    /// # Errors
    ///
    /// Fails when the seed is not a user seed or the bus cannot be reached.
    pub async fn connect(server: &str, seed: &str) -> Result<Self> {
        public_key_of(seed)?;
        let client = ConnectOptions::with_nkey(seed.trim().to_owned())
            .event_callback(|event| async move { report(&event) })
            .connect(server)
            .await
            .with_context(|| format!("Unable to reach the bus at {server}"))?;
        Ok(Self {
            jetstream: jetstream::new(client),
        })
    }

    /// Create the inbox stream and an inbox consumer for every agent in `agents`. This is the
    /// host's job: only the host may do it. It is safe to repeat.
    ///
    /// # Errors
    ///
    /// Fails when the bus refuses the stream or a consumer.
    pub async fn create_inboxes(&self, agents: &Agents) -> Result<Vec<(String, InboxChange)>> {
        let stream = self
            .jetstream
            .get_or_create_stream(stream::Config {
                name: STREAM.to_owned(),
                subjects: vec![format!("{SUBJECT_PREFIX}.>")],
                max_age: MAX_AGE,
                ..Default::default()
            })
            .await
            .context("Unable to create the inbox stream")?;
        let mut outcomes = Vec::new();
        for agent in agents.all() {
            let wanted = consumer_config(&agent.name)?;
            let change = self
                .ensure_inbox(&stream, &wanted)
                .await
                .with_context(|| format!("Unable to prepare the inbox of '{}'", agent.name))?;
            outcomes.push((agent.name.clone(), change));
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
                    .update_consumer(wanted.clone())
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
    /// Fails when the receiver's name is invalid or the bus does not store the message.
    pub async fn send(&self, message: &Message) -> Result<()> {
        let subject = inbox_subject(&message.to)?;
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
    /// Wait for the next message. A message that is not one of ours is reported, acknowledged
    /// and skipped.
    ///
    /// # Errors
    ///
    /// Fails when the bus connection fails.
    pub async fn next(&mut self) -> Result<Option<Delivery>> {
        while let Some(raw) = self.messages.next().await {
            let raw = raw.context("Unable to read the inbox")?;
            match serde_json::from_slice::<Message>(&raw.payload) {
                Ok(message) => return Ok(Some(Delivery { message, raw })),
                Err(error) => {
                    eprintln!("Skipping a message that is not an agent message: {error}");
                    raw.ack()
                        .await
                        .map_err(anyhow::Error::from_boxed)
                        .context("Unable to acknowledge a skipped message")?;
                }
            }
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

/// The consumer behind `agent`'s inbox.
fn consumer_config(agent: &str) -> Result<consumer::pull::Config> {
    Ok(consumer::pull::Config {
        durable_name: Some(agent.to_owned()),
        filter_subject: inbox_subject(agent)?,
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

/// Print a connection event that is not routine.
fn report(event: &Event) {
    match event {
        Event::Connected | Event::Draining | Event::Closed => {}
        other => eprintln!("Bus connection event: {other}"),
    }
}

/// The subject of `agent`'s inbox.
fn inbox_subject(agent: &str) -> Result<String> {
    check_agent_name(agent)?;
    Ok(format!("{SUBJECT_PREFIX}.{agent}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_agent_has_its_own_subject_inside_the_stream() {
        assert_eq!(inbox_subject("dev").unwrap(), "agents.dev");
        assert!(inbox_subject("dev.>").is_err());
    }

    fn existing(max_waiting: i64, filter: &str) -> consumer::Config {
        consumer::Config {
            max_waiting,
            filter_subject: filter.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn an_unchanged_consumer_is_kept_whatever_its_start_policy() {
        let wanted = consumer_config("dev").unwrap();
        let mut current = existing(1, "agents.dev");
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
    fn a_max_waiting_difference_is_recreated() {
        let wanted = consumer_config("dev").unwrap();
        assert_eq!(plan(&existing(512, "agents.dev"), &wanted), Plan::Recreate);
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
