//! The bus connection: one `JetStream` stream holds every agent's inbox, so a message waits until
//! its agent reads it.

use std::time::Duration;

use anyhow::{Context, Result};
use async_nats::jetstream::{self, consumer, stream};
use futures_util::StreamExt;

use crate::message::{Message, check_agent_name};

/// The stream holding every inbox.
const STREAM: &str = "AGENTS";
/// Each agent's inbox is `agents.<name>`.
const SUBJECT_PREFIX: &str = "agents";
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

/// A message read from an inbox; acknowledge it once handled.
pub struct Delivery {
    pub message: Message,
    raw: jetstream::Message,
}

impl Channel {
    /// Connect to the bus at `server` and make sure the inbox stream exists.
    ///
    /// # Errors
    ///
    /// Fails when the bus cannot be reached or the stream cannot be created.
    pub async fn connect(server: &str) -> Result<Self> {
        let client = async_nats::connect(server)
            .await
            .with_context(|| format!("Unable to reach the bus at {server}"))?;
        let jetstream = jetstream::new(client);
        jetstream
            .get_or_create_stream(stream::Config {
                name: STREAM.to_owned(),
                subjects: vec![format!("{SUBJECT_PREFIX}.>")],
                max_age: MAX_AGE,
                ..Default::default()
            })
            .await
            .context("Unable to create the inbox stream")?;
        Ok(Self { jetstream })
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
    /// Fails when the name is invalid or the inbox cannot be opened.
    pub async fn inbox(&self, agent: &str) -> Result<Inbox> {
        let subject = inbox_subject(agent)?;
        let stream = self
            .jetstream
            .get_stream(STREAM)
            .await
            .context("Unable to open the inbox stream")?;
        let reader = stream
            .get_or_create_consumer(
                agent,
                consumer::pull::Config {
                    durable_name: Some(agent.to_owned()),
                    filter_subject: subject,
                    ..Default::default()
                },
            )
            .await
            .with_context(|| format!("Unable to open the inbox of '{agent}'"))?;
        let messages = reader
            .messages()
            .await
            .with_context(|| format!("Unable to read the inbox of '{agent}'"))?;
        Ok(Inbox { messages })
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
}
