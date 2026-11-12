//! One reader's pulls from a durable consumer, one at a time, each for one message.
//!
//! async-nats's pull stream retries what the bus answers without saying so: a 409 refusing the
//! pull (another pull holds the consumer's one waiting slot, `max_waiting = 1`) is pulled again
//! at once, and a pull the bus never answers is noticed only by its own timer. A reader stuck
//! that way prints nothing. This loop sends the next pull only once the last one is over, and
//! writes a notice for every answer other than a message or an expiry, and for a pull the bus
//! says nothing about for `SILENCE_LIMIT`.
//!
//! The pull asks for no idle heartbeat. nats-server 2.15.2 answers a refused pull with 409
//! `Exceeded MaxWaiting` only when it has none; a refused pull with a heartbeat gets no answer
//! at all, so the reader could not say why it gets nothing. Instead a pull expires soon, and a
//! pull with no answer by its expiry and some slack is counted lost.

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_nats::jetstream::consumer::pull::BatchConfig;
use async_nats::jetstream::{self, Message};
use async_nats::{Client, StatusCode, Subscriber};
use futures_util::StreamExt;
use tokio::time::{Instant, sleep_until, timeout_at};
use tracing::instrument;

/// How long the bus keeps a pull waiting for a message before it ends it with a 408.
const PULL_EXPIRES: Duration = Duration::from_secs(10);
/// How long a pull may go without a word from the bus (its expiry and 5 s) before it is counted
/// lost and pulled again.
const SILENCE_LIMIT: Duration = Duration::from_secs(15);
/// How long to wait before pulling again after the bus refused a pull or answered with an error.
const RETRY_PAUSE: Duration = Duration::from_secs(1);
/// The description of the 409 the bus answers a pull with while another pull holds the
/// consumer's waiting slot.
const MAX_WAITING: &str = "Exceeded MaxWaiting";

/// What the bus said about a pull, or that it said nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A message: the pull is over.
    Message,
    /// The bus still holds the pull. Not asked for; taken if the bus sends one.
    Heartbeat,
    /// 408: the pull expired with no message, as an idle pull does every `PULL_EXPIRES`.
    Expired,
    /// 409 `Exceeded MaxWaiting`: another pull holds the consumer's one waiting slot.
    Refused,
    /// An answer after which the reader cannot go on: the consumer was deleted or is a push
    /// consumer, or the pull is over its limits.
    Ended(String),
    /// Any other status, such as 503 while the bus has no `JetStream`.
    Status(String),
    /// Nothing for `SILENCE_LIMIT`: the bus lost the pull, or the connection is gone.
    Silence,
}

/// What to do after a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Keep waiting on the same pull.
    Wait,
    /// The pull is over: send the next.
    Pull,
    /// The pull is over: send the next after `RETRY_PAUSE`.
    PullAfterPause,
    /// The reader cannot go on.
    Stop,
}

/// What a reader has reported, so a trouble that repeats is reported once, and its end once.
#[derive(Debug, Default)]
pub struct Watch {
    /// Say so the first time the bus takes a pull.
    announce: bool,
    attached: bool,
    /// The trouble last reported, until a pull is taken again.
    trouble: Option<String>,
}

/// One reader's pulls from the durable consumer `name`, with its notices written to `notices`.
pub struct Puller<W> {
    client: Client,
    context: jetstream::Context,
    name: String,
    /// The `JetStream` API subject a pull for `name` is sent to.
    subject: String,
    /// The pull request: one message, with an expiry.
    request: Vec<u8>,
    /// The replies to the pull that is open, and when the bus last said anything about it.
    open: Option<(Subscriber, Instant)>,
    /// When the next pull may be sent.
    pause_until: Option<Instant>,
    watch: Watch,
    /// Where the reader's notices go: stderr, for a command.
    notices: W,
}

/// What a reply from the bus to a pull is, from its status and description.
pub fn classify(status: Option<StatusCode>, description: Option<&str>) -> Reply {
    let description = description.unwrap_or_default();
    match status {
        None | Some(StatusCode::OK) => Reply::Message,
        Some(StatusCode::IDLE_HEARTBEAT) => Reply::Heartbeat,
        Some(StatusCode::TIMEOUT) => Reply::Expired,
        Some(StatusCode::REQUEST_TERMINATED) if description == MAX_WAITING => Reply::Refused,
        Some(StatusCode::REQUEST_TERMINATED)
            if description == "Consumer Deleted"
                || description == "Consumer is push based"
                || description.starts_with("Exceeded MaxRequest") =>
        {
            Reply::Ended(format!("409 {description}"))
        }
        Some(code) => Reply::Status(format!("{code} {description}").trim_end().to_owned()),
    }
}

/// What to do after `reply`.
pub fn step(reply: &Reply) -> Step {
    match reply {
        Reply::Heartbeat => Step::Wait,
        Reply::Message | Reply::Expired | Reply::Silence => Step::Pull,
        Reply::Refused | Reply::Status(_) => Step::PullAfterPause,
        Reply::Ended(_) => Step::Stop,
    }
}

impl Watch {
    /// A watch that also says when the bus first takes a pull.
    pub fn announcing() -> Self {
        Self {
            announce: true,
            ..Self::default()
        }
    }

    /// The line to report after `reply` on the consumer `name`, if any.
    pub fn note(&mut self, reply: &Reply, name: &str) -> Option<String> {
        let trouble = match reply {
            Reply::Message | Reply::Heartbeat | Reply::Expired => {
                if self.trouble.take().is_some() {
                    self.attached = true;
                    return Some(format!("Reading '{name}' again: the bus took the pull"));
                }
                if self.announce && !self.attached {
                    self.attached = true;
                    return Some(format!("Listening on '{name}': the bus took the pull"));
                }
                self.attached = true;
                return None;
            }
            Reply::Refused => format!(
                "The bus refuses the pulls on '{name}' (409 {MAX_WAITING}): another reader \
                 holds it, so this one gets nothing until it lets go. Stop the other reader \
                 (for example an earlier `agent-bus listen {name}`); still trying"
            ),
            Reply::Status(status) => format!(
                "The bus answered the pull on '{name}' with {status}; pulling again in {} s",
                RETRY_PAUSE.as_secs()
            ),
            Reply::Silence => format!(
                "The bus said nothing about the pull on '{name}' for {} s (no message or \
                 expiry); pulling again",
                SILENCE_LIMIT.as_secs()
            ),
            Reply::Ended(status) => {
                format!("The bus ended the pulls on '{name}' with {status}; the reader stops")
            }
        };
        if self.trouble.as_deref() == Some(trouble.as_str()) {
            return None;
        }
        self.trouble = Some(trouble.clone());
        Some(trouble)
    }
}

impl<W: Write> Puller<W> {
    /// Pulls from the consumer `name` of `stream`, writing its notices to `notices`.
    ///
    /// # Errors
    ///
    /// Fails when the pull request cannot be encoded.
    pub fn new(context: &jetstream::Context, stream: &str, name: &str, notices: W) -> Result<Self> {
        let request = serde_json::to_vec(&BatchConfig {
            batch: 1,
            expires: Some(PULL_EXPIRES),
            ..BatchConfig::default()
        })
        .context("Unable to encode the pull request")?;
        Ok(Self {
            client: context.client(),
            context: context.clone(),
            name: name.to_owned(),
            subject: format!("$JS.API.CONSUMER.MSG.NEXT.{stream}.{name}"),
            request,
            open: None,
            pause_until: None,
            watch: Watch::default(),
            notices,
        })
    }

    /// Say once, when the bus first takes a pull, that the reader is attached.
    pub fn announce(&mut self) {
        self.watch = Watch::announcing();
    }

    /// Wait for the next message, pulling as needed. `None` once the connection is closed.
    /// Safe to cancel: a pull left open is waited on by the next call.
    ///
    /// # Errors
    ///
    /// Fails when a pull cannot be sent, a notice cannot be written, or the bus ends the pulls
    /// (the consumer was deleted or is a push consumer, or the pull is over its limits).
    #[instrument(level = "debug", skip_all, fields(name = %self.name))]
    pub async fn next(&mut self) -> Result<Option<Message>> {
        loop {
            if let Some(until) = self.pause_until {
                sleep_until(until).await;
                self.pause_until = None;
            }
            if self.open.is_none() {
                self.open = Some(self.pull().await?);
            }
            let Some((replies, heard)) = self.open.as_mut() else {
                continue;
            };
            let (reply, message) = match timeout_at(*heard + SILENCE_LIMIT, replies.next()).await {
                Err(_) => (Reply::Silence, None),
                Ok(None) => return Ok(None),
                Ok(Some(message)) => {
                    *heard = Instant::now();
                    (
                        classify(message.status, message.description.as_deref()),
                        Some(message),
                    )
                }
            };
            if let Some(line) = self.watch.note(&reply, &self.name) {
                self.notice(&line)?;
            }
            match step(&reply) {
                Step::Wait => {}
                Step::Pull => self.open = None,
                Step::PullAfterPause => {
                    self.open = None;
                    self.pause_until = Some(Instant::now() + RETRY_PAUSE);
                }
                Step::Stop => {
                    self.open = None;
                    bail!("The bus ended the pulls on '{}': {reply:?}", self.name);
                }
            }
            if let (Reply::Message, Some(message)) = (reply, message) {
                return Ok(Some(Message {
                    message,
                    context: self.context.clone(),
                }));
            }
        }
    }

    /// Write `line` to the notices.
    ///
    /// # Errors
    ///
    /// Fails when the notices cannot be written.
    pub fn notice(&mut self, line: &str) -> Result<()> {
        writeln!(self.notices, "{line}")
            .and_then(|()| self.notices.flush())
            .context("Unable to write a reader's notice")
    }

    /// Send one pull, with its own reply subject, so a reply to an earlier pull is never taken
    /// for one to this pull.
    #[instrument(level = "debug", skip_all, fields(name = %self.name))]
    async fn pull(&self) -> Result<(Subscriber, Instant)> {
        let reply = self.client.new_inbox();
        let replies = self
            .client
            .subscribe(reply.clone())
            .await
            .with_context(|| format!("Unable to take the replies to a pull on '{}'", self.name))?;
        self.client
            .publish_with_reply(self.subject.clone(), reply, self.request.clone().into())
            .await
            .with_context(|| format!("Unable to send a pull on '{}'", self.name))?;
        Ok((replies, Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_are_told_apart_by_status_and_description() {
        assert_eq!(classify(None, None), Reply::Message);
        assert_eq!(
            classify(Some(StatusCode::IDLE_HEARTBEAT), Some("Idle Heartbeat")),
            Reply::Heartbeat
        );
        assert_eq!(
            classify(Some(StatusCode::TIMEOUT), Some("Request Timeout")),
            Reply::Expired
        );
        assert_eq!(
            classify(Some(StatusCode::REQUEST_TERMINATED), Some(MAX_WAITING)),
            Reply::Refused
        );
        for ending in [
            "Consumer Deleted",
            "Consumer is push based",
            "Exceeded MaxRequestBatch of 1",
            "Exceeded MaxRequestExpires of 1s",
        ] {
            assert_eq!(
                classify(Some(StatusCode::REQUEST_TERMINATED), Some(ending)),
                Reply::Ended(format!("409 {ending}"))
            );
        }
        assert_eq!(
            classify(
                Some(StatusCode::REQUEST_TERMINATED),
                Some("Leadership Change")
            ),
            Reply::Status("409 Leadership Change".to_owned())
        );
        assert_eq!(
            classify(Some(StatusCode::NO_RESPONDERS), None),
            Reply::Status("503".to_owned())
        );
    }

    #[test]
    fn only_a_heartbeat_keeps_the_pull_open() {
        assert_eq!(step(&Reply::Heartbeat), Step::Wait);
        for over in [Reply::Message, Reply::Expired, Reply::Silence] {
            assert_eq!(step(&over), Step::Pull, "{over:?}");
        }
        // A refusal or an error is not pulled again at once, so a held inbox is not spun on.
        for refused in [Reply::Refused, Reply::Status("503".to_owned())] {
            assert_eq!(step(&refused), Step::PullAfterPause, "{refused:?}");
        }
        assert_eq!(
            step(&Reply::Ended("409 Consumer Deleted".to_owned())),
            Step::Stop
        );
    }

    #[test]
    fn routine_replies_are_quiet() {
        let mut watch = Watch::default();
        for reply in [Reply::Heartbeat, Reply::Expired, Reply::Message] {
            assert_eq!(watch.note(&reply, "host"), None, "{reply:?}");
        }
    }

    #[test]
    fn an_announcing_watch_says_once_when_the_bus_takes_the_pull() {
        let mut watch = Watch::announcing();
        let line = watch.note(&Reply::Heartbeat, "host").unwrap();
        assert!(line.starts_with("Listening on 'host'"), "{line}");
        assert_eq!(watch.note(&Reply::Expired, "host"), None);
        assert_eq!(watch.note(&Reply::Message, "host"), None);
    }

    #[test]
    fn a_reader_that_never_attaches_is_not_announced_but_its_trouble_is() {
        let mut watch = Watch::announcing();
        let line = watch.note(&Reply::Silence, "host").unwrap();
        assert!(line.contains("said nothing"), "{line}");
        let line = watch.note(&Reply::Heartbeat, "host").unwrap();
        assert!(line.starts_with("Reading 'host' again"), "{line}");
        assert_eq!(watch.note(&Reply::Heartbeat, "host"), None);
    }

    #[test]
    fn every_trouble_is_reported_once_until_a_pull_is_taken() {
        let mut watch = Watch::default();
        let refused = watch.note(&Reply::Refused, "host").unwrap();
        assert!(refused.contains("409 Exceeded MaxWaiting"), "{refused}");
        assert!(refused.contains("agent-bus listen host"), "{refused}");
        assert_eq!(watch.note(&Reply::Refused, "host"), None);
        // A different trouble is reported, and the first again after it.
        let status = watch
            .note(&Reply::Status("503".to_owned()), "host")
            .unwrap();
        assert!(status.contains("with 503"), "{status}");
        assert_eq!(watch.note(&Reply::Refused, "host"), Some(refused.clone()));
        // Once a pull is taken, the reader says so, once; a later refusal is new.
        let again = watch.note(&Reply::Expired, "host").unwrap();
        assert!(again.starts_with("Reading 'host' again"), "{again}");
        assert_eq!(watch.note(&Reply::Expired, "host"), None);
        assert_eq!(watch.note(&Reply::Refused, "host"), Some(refused));
    }

    #[test]
    fn the_end_of_the_pulls_is_reported() {
        let mut watch = Watch::default();
        let line = watch
            .note(&Reply::Ended("409 Consumer Deleted".to_owned()), "host")
            .unwrap();
        assert!(line.contains("409 Consumer Deleted"), "{line}");
    }
}
