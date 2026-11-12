//! The guest agent's entry point and its loop: read the inbox, queue tasks, run them one at a
//! time through Claude Code, and send back the replies.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{Stderr, stderr};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::time::{MissedTickBehavior, interval};
use tokio::try_join;
use tracing::instrument;

use super::base::{Base, Inputs};
use super::claude::{
    CLAUDE, Group, Launch, PRIME_CONFIRM, PRIME_PROMPT, claude_command, finish, rules, terminate,
};
use super::engine::{Control, Session, canceled_text, classify, task_github, task_model};
use super::github::{Access, GH_TOKEN_VARIABLE, check_environment};
use super::ledger::{Ledger, Run};
use super::lock::InstanceLock;
use super::pull_request::{GH, Note, checkouts, current_branch, update};
use super::record::ResultLog;
use super::secrets::{Secrets, TOKEN_VARIABLE, refuse_in_environment};
use super::state::{ApiMark, Queued, Saved, StateFile, TaskRecord, prune, restart};
use super::stream::{Update, interpret, outcome, priming_line, repeats_result, usage_line};
use crate::channel::{Channel, Endpoint, Inbox};
use crate::files::now_ms;
use crate::keys::SEED_VARIABLE;
use crate::message::{Kind, Message, TaskUsage, Usage, api_time_of_run, check_agent_name};
use crate::status::{HEARTBEAT, Running, Status};

/// Why a `--github` task fails on a guest agent started without a write token.
const NO_WRITE_TOKEN: &str = "This guest agent has no GitHub write token, so it cannot run a \
--github task: restart it with agent-bus-start-agent, with the write token in GH_TOKEN on the host";

/// Where replies about one task go, and the model it runs on, when known.
struct Reply<'a> {
    outgoing: UnboundedSender<Message>,
    task: String,
    from: String,
    to: String,
    model: Option<String>,
    /// Replaced in every text sent.
    secrets: &'a Secrets,
}

/// How a run of Claude Code ended.
enum Finish {
    Done,
    Canceled,
    /// The guest agent is shutting down.
    Stopped,
    /// The guest agent began shutting down before the task's run started: the task goes back
    /// to the queue, unreported, for the restart.
    Postponed,
}

/// How one attempt at a task ended.
enum Attempt {
    Done(Finish),
    /// Forking the base session failed, before the task did anything: the reason.
    ForkFailed(String),
}

/// One event while a task runs: Claude's output, the inbox, or the shutdown.
enum Event {
    Line(Option<String>),
    Incoming(Option<Box<Message>>),
    Stop,
}

/// A run's outcome: its text, whether it failed, what it used, and what its task has used so far.
struct Finished {
    text: String,
    failed: bool,
    usage: Usage,
    task_usage: TaskUsage,
}

/// What one run has produced so far.
#[derive(Default)]
struct RunState {
    /// Claude's last text is also its result, so each text waits for the next update to show
    /// whether it was the last.
    text: Option<String>,
    /// The last tool call, already sent as progress.
    tool: Option<String>,
    /// The outcome, text first, failed second.
    finished: Option<Finished>,
    /// Whether the run named its session, said something or used a tool: it did something, so
    /// it is not run again.
    progressed: bool,
}

/// Where and how Claude Code runs.
#[derive(Clone, Copy)]
struct Setup<'a> {
    program: &'a str,
    /// The GitHub CLI, which rewrites the Generation section of pull requests.
    gh: &'a str,
    workdir: &'a Path,
    /// The model of a task that names none.
    model: &'a str,
    /// The Claude token, the GitHub write token for `--github` tasks if the agent was given
    /// one, and the bus seed.
    secrets: &'a Secrets,
}

/// The guest agent's state: the tasks waiting, the one running, and what is known of each task.
/// All of it but the channels is kept in the state file after every change.
struct Engine<'a> {
    program: &'a str,
    gh: &'a str,
    workdir: &'a Path,
    /// The model of a task that names none.
    model: &'a str,
    /// The Claude token and the GitHub write token for runs, and all three secrets to redact.
    secrets: &'a Secrets,
    name: &'a str,
    reply_to: &'a str,
    incoming: UnboundedReceiver<Message>,
    outgoing: UnboundedSender<Message>,
    stop: watch::Receiver<bool>,
    queue: VecDeque<Message>,
    /// The task Claude Code works on now.
    running: Option<String>,
    /// When the running task started.
    running_since_ms: u64,
    /// What the heartbeat writes to the bus: whether a task runs, and the queue. It changes with
    /// the state file, in `save`.
    status: watch::Sender<Status>,
    /// The session, runs and totals of each task.
    tasks: HashMap<String, TaskRecord>,
    /// The primed base session of each rules variant and model: by whether it allows GitHub,
    /// and the model.
    bases: HashMap<(bool, String), Base>,
    results: Option<ResultLog>,
    /// What each task cost and the branch it ended on; `None` without a state directory.
    ledger: Option<Ledger>,
    /// Where the state is kept; `None` keeps it in memory only.
    store: Option<StateFile>,
}

/// Take tasks from `name`'s inbox on the bus at `server`, authenticating with the seed in
/// `secrets`, until the bus closes or the agent is told to stop (SIGTERM or SIGINT), and reply to
/// `reply_to`. Stopping also stops the running Claude Code and everything it started. Every run
/// gets the Claude token from `secrets`; `--github` tasks get its write token, and fail without
/// one.
///
/// # Errors
///
/// Fails when a name is invalid, another guest agent of this name is running, the seed or the
/// Claude token is in the agent's own environment, the write token is also its own `GH_TOKEN`,
/// or the bus fails. A task that fails is reported to `reply_to` instead.
#[instrument(skip_all, fields(agent = %name, url = %server.url))]
pub async fn run(
    server: &Endpoint,
    secrets: &Secrets,
    name: &str,
    reply_to: &str,
    workdir: &Path,
    model: &str,
) -> Result<()> {
    check_agent_name(name)?;
    check_agent_name(reply_to)?;
    let store = StateFile::standard(name)
        .inspect_err(|error| {
            eprintln!("{error:#}: the guest agent's state is not kept across restarts");
        })
        .ok();
    // Boxed: the running agent is a large future.
    Box::pin(start(
        server,
        name,
        reply_to,
        Setup {
            program: CLAUDE,
            gh: GH,
            workdir,
            model,
            secrets,
        },
        store,
    ))
    .await
}

/// `run`, with the state kept in `store`, or in memory only when it is `None`.
async fn start(
    server: &Endpoint,
    name: &str,
    reply_to: &str,
    setup: Setup<'_>,
    store: Option<StateFile>,
) -> Result<()> {
    // Taken first, before the bus, the inbox and the state, and held until this returns: a
    // second agent of this name fails at once, and never touches the bus or loads the state.
    let _lock = match &store {
        Some(file) => {
            let lock = InstanceLock::acquire(&file.path().with_extension("lock"))?;
            eprintln!("Guest agent lock: {}", lock.path().display());
            Some(lock)
        }
        None => None,
    };
    refuse_in_environment(
        env::var_os(SEED_VARIABLE).as_deref(),
        env::var_os(TOKEN_VARIABLE).as_deref(),
    )?;
    check_environment(
        env::var_os(GH_TOKEN_VARIABLE).as_deref(),
        setup.secrets.write_token(),
    )?;
    if setup.secrets.write_token().is_some() {
        eprintln!("GitHub write token: held for --github tasks");
    } else {
        eprintln!("No GitHub write token: --github tasks will fail");
    }
    let channel = &Channel::connect(server, setup.secrets.seed().expose()).await?;
    let inbox = channel.inbox(name, stderr()).await?;
    let mut terminate = signal(SignalKind::terminate()).context("Unable to watch for SIGTERM")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("Unable to watch for SIGINT")?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let (incoming_tx, incoming_rx) = unbounded_channel();
    let (outgoing_tx, outgoing_rx) = unbounded_channel();
    let mut engine = Engine::new(
        setup,
        name,
        reply_to,
        incoming_rx,
        outgoing_tx,
        stop_rx.clone(),
    );
    if let Some(file) = store {
        engine.resume(file)?;
    }
    engine.prune_results();
    match &engine.ledger {
        Some(ledger) => eprintln!("Generation ledger: {}", ledger.path().display()),
        None => eprintln!("No state directory: pull requests' Generation sections are not kept"),
    }
    let status = engine.status.subscribe();
    eprintln!("Guest agent '{name}' is waiting for tasks");
    let work = async {
        try_join!(
            read_inbox(inbox, incoming_tx, stop_rx),
            forward(channel, outgoing_rx),
            engine.serve(),
            async {
                heartbeat(channel, name, status, HEARTBEAT).await;
                Ok(())
            }
        )
        .map(|_| ())
    };
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result,
        _ = terminate.recv() => {
            eprintln!("SIGTERM: stopping");
            stop_tx.send_replace(true);
            work.await
        }
        _ = interrupt.recv() => {
            eprintln!("SIGINT: stopping");
            stop_tx.send_replace(true);
            work.await
        }
    }
}

/// Move messages from the inbox to the engine, acknowledging each at once: a task can run far
/// longer than the bus waits for an acknowledgement, and it must not be delivered twice.
async fn read_inbox(
    mut inbox: Inbox<Stderr>,
    incoming: UnboundedSender<Message>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    loop {
        let delivery = tokio::select! {
            delivery = inbox.next() => delivery?,
            () = stopped(&mut stop) => None,
        };
        let Some(delivery) = delivery else {
            return Ok(());
        };
        // Once stopping, a message is left unacknowledged, to be delivered again after the
        // restart, rather than handed to an engine that may no longer keep it.
        if *stop.borrow() {
            return Ok(());
        }
        delivery.ack().await?;
        if incoming.send(delivery.message).is_err() {
            return Ok(());
        }
    }
}

/// Send each reply to the bus, in order.
async fn forward(channel: &Channel, mut outgoing: UnboundedReceiver<Message>) -> Result<()> {
    while let Some(message) = outgoing.recv().await {
        channel.send(&message).await?;
    }
    Ok(())
}

/// Write the agent's status to the bus on every change and `every` (`HEARTBEAT`), until the
/// engine is gone; then once more, marked offline. A status the bus does not take is reported, once
/// until it takes one again, and the agent goes on: the status is for the host's view, the
/// tasks do not depend on it.
#[instrument(skip_all, fields(agent = %name))]
pub async fn heartbeat(
    channel: &Channel,
    name: &str,
    mut status: watch::Receiver<Status>,
    every: Duration,
) {
    let mut beat = interval(every);
    beat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut failing = false;
    loop {
        let closed = tokio::select! {
            _ = beat.tick() => false,
            changed = status.changed() => changed.is_err(),
        };
        let mut current = status.borrow_and_update().clone();
        current.online = !closed;
        current.at_ms = now_ms();
        match (channel.put_status(name, &current).await, failing) {
            (Ok(()), true) => {
                eprintln!("The bus takes the status again");
                failing = false;
            }
            (Err(error), false) => {
                eprintln!("{error:#}: the host does not see this agent's status");
                failing = true;
            }
            _ => {}
        }
        if closed {
            return;
        }
    }
}

/// Resolves when the agent is told to stop.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    // A closed channel also means stop.
    let _ = stop.wait_for(|stop| *stop).await;
}

/// Log what a finished run used, and send its outcome, with what the Generation step did, if
/// anything, in a paragraph before its text: a question stays on the last line.
fn report(reply: &Reply<'_>, model: &str, finished: Finished, note: Option<Note>) -> Result<()> {
    let kind = outcome(&finished.text, finished.failed);
    eprintln!(
        "{}",
        usage_line(
            &reply.task,
            kind,
            model,
            &finished.usage,
            &finished.task_usage
        )
    );
    let text = match note {
        Some(note) => format!("{}\n\n{}", note.text(), finished.text),
        None => finished.text,
    };
    reply.send(kind, text, Some((finished.usage, finished.task_usage)))
}

impl Reply<'_> {
    /// Send `text`, with the write token replaced.
    fn send(&self, kind: Kind, text: String, usage: Option<(Usage, TaskUsage)>) -> Result<()> {
        let (usage, task_usage) = usage.unzip();
        let text = match self.secrets.redact(&text) {
            Cow::Borrowed(_) => text,
            Cow::Owned(redacted) => redacted,
        };
        self.outgoing
            .send(Message {
                task: self.task.clone(),
                from: self.from.clone(),
                to: self.to.clone(),
                kind,
                text,
                usage,
                task_usage,
                github: false,
                model: self.model.clone(),
            })
            .map_err(|_| anyhow!("The bus connection is closed"))
    }
}

impl<'a> Engine<'a> {
    fn new(
        setup: Setup<'a>,
        name: &'a str,
        reply_to: &'a str,
        incoming: UnboundedReceiver<Message>,
        outgoing: UnboundedSender<Message>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            program: setup.program,
            gh: setup.gh,
            workdir: setup.workdir,
            model: setup.model,
            secrets: setup.secrets,
            name,
            reply_to,
            incoming,
            outgoing,
            stop,
            queue: VecDeque::new(),
            running: None,
            running_since_ms: 0,
            status: watch::Sender::new(Status {
                online: true,
                ..Status::default()
            }),
            tasks: HashMap::new(),
            bases: HashMap::new(),
            results: ResultLog::standard(),
            ledger: Ledger::standard().ok(),
            store: None,
        }
    }

    /// Take up the state saved in `file`, and keep the state there from now on. A task that was
    /// running when the agent stopped without reporting it is reported failed; its session is
    /// kept, so an answer can resume it.
    fn resume(&mut self, file: StateFile) -> Result<()> {
        let now = now_ms();
        eprintln!("Guest agent state: {}", file.path().display());
        let restart = restart(file.load(now), self.name, now);
        self.tasks = restart.tasks;
        self.queue = restart.queue;
        self.bases = restart.bases;
        self.store = Some(file);
        if !self.queue.is_empty() {
            eprintln!("Resuming {} queued tasks and answers", self.queue.len());
        }
        if let Some(task) = restart.interrupted {
            eprintln!("Task {task} was running when the guest agent stopped: reporting it failed");
            self.reply(&task, None).send(
                Kind::Failed,
                "The guest agent restarted while this task ran".to_owned(),
                None,
            )?;
        }
        self.save();
        Ok(())
    }

    /// Drop the result records older than the stream's retention. A failure is reported and the
    /// agent goes on; the next append tries again a day later.
    fn prune_results(&mut self) {
        if let Some(log) = &mut self.results {
            match log.prune(now_ms()) {
                Ok(0) => {}
                Ok(dropped) => eprintln!("Pruned {dropped} old result records"),
                Err(error) => eprintln!("{error:#}"),
            }
        }
    }

    /// What the state file keeps.
    fn snapshot(&self) -> Saved {
        let mut bases: Vec<Base> = self.bases.values().cloned().collect();
        bases.sort_by(|a, b| (a.github, &a.model).cmp(&(b.github, &b.model)));
        Saved {
            running: self.running.clone(),
            queue: self.queue.iter().map(Queued::of).collect(),
            bases,
            tasks: self
                .tasks
                .iter()
                .map(|(task, record)| (task.clone(), record.clone()))
                .collect(),
        }
    }

    /// Write the state file, and update the status. A write that fails is reported and the
    /// agent goes on: the state in memory is still right, only a restart would lose it.
    fn save(&self) {
        self.status.send_replace(Status {
            online: true,
            at_ms: 0,
            running: self.running.as_ref().map(|task| Running {
                task: task.clone(),
                since_ms: self.running_since_ms,
            }),
            queued: u64::try_from(self.queue.len()).unwrap_or(u64::MAX),
        });
        if let Some(store) = &self.store
            && let Err(error) = store.save(&self.snapshot())
        {
            eprintln!("Unable to save the guest agent's state: {error:#}");
        }
    }

    /// Take the messages that reached the engine but not the queue, so the final state keeps
    /// them.
    fn drain_incoming(&mut self) -> Result<()> {
        while let Ok(message) = self.incoming.try_recv() {
            self.control(&message, None)?;
        }
        Ok(())
    }

    fn reply(&self, task: &str, model: Option<&str>) -> Reply<'a> {
        Reply {
            outgoing: self.outgoing.clone(),
            task: task.to_owned(),
            from: self.name.to_owned(),
            to: self.reply_to.to_owned(),
            model: model.map(str::to_owned),
            secrets: self.secrets,
        }
    }

    /// The access a run gets: the write token for a `--github` task, which fails without one.
    fn access(&self, github: bool) -> Result<Access<'a>> {
        if github {
            Ok(Access::Write(
                self.secrets.write_token().context(NO_WRITE_TOKEN)?,
            ))
        } else {
            Ok(Access::ReadOnly)
        }
    }

    /// Run the queued tasks, and the ones that arrive, one at a time, until the inbox closes or
    /// the agent is told to stop.
    async fn serve(mut self) -> Result<()> {
        loop {
            if *self.stop.borrow() {
                self.drain_incoming()?;
                self.save();
                return Ok(());
            }
            let message = if let Some(message) = self.queue.pop_front() {
                message
            } else {
                let message = tokio::select! {
                    message = self.incoming.recv() => message,
                    () = stopped(&mut self.stop) => None,
                };
                let Some(message) = message else {
                    self.save();
                    return Ok(());
                };
                if classify(&message, None, &self.queue) != Control::Queue {
                    self.control(&message, None)?;
                    continue;
                }
                message
            };
            eprintln!(
                "{:?} {} from '{}'",
                message.kind, message.task, message.from
            );
            self.running = Some(message.task.clone());
            self.running_since_ms = now_ms();
            self.save();
            let result = self.work(&message).await;
            // Each way out of `work` has reported or postponed the task, so it is no longer
            // running.
            self.running = None;
            if matches!(result, Ok(Finish::Postponed)) {
                eprintln!(
                    "Task {} stays queued: the guest agent is stopping",
                    message.task
                );
                self.queue.push_front(message);
                self.drain_incoming()?;
                self.save();
                return Ok(());
            }
            let now = now_ms();
            self.tasks
                .entry(message.task.clone())
                .or_default()
                .finished_at_ms = Some(now);
            let queue = &self.queue;
            prune(&mut self.tasks, now, |task| {
                queue.iter().any(|queued| queued.task == task)
            });
            match result {
                Ok(Finish::Stopped) => {
                    // The group is stopped and the task reported: the final state.
                    self.drain_incoming()?;
                    self.save();
                    return Ok(());
                }
                Ok(Finish::Done | Finish::Canceled | Finish::Postponed) => self.save(),
                Err(error) => {
                    self.save();
                    self.reply(&message.task, None).send(
                        Kind::Failed,
                        format!("{error:#}"),
                        None,
                    )?;
                }
            }
        }
    }

    /// Handle a message that arrived while idle or while `running` runs. Returns whether it is
    /// a cancel for the running task, which the caller must carry out.
    fn control(&mut self, message: &Message, running: Option<&str>) -> Result<bool> {
        match classify(message, running, &self.queue) {
            Control::Queue => {
                self.queue.push_back(message.clone());
                self.save();
            }
            Control::CancelRunning => return Ok(true),
            Control::CancelQueued => {
                self.queue.retain(|queued| queued.task != message.task);
                self.save();
                eprintln!("Canceled queued task {}", message.task);
                self.reply(&message.task, None).send(
                    Kind::Canceled,
                    "Canceled before it started".to_owned(),
                    None,
                )?;
            }
            Control::CancelUnknown => {
                self.reply(&message.task, None).send(
                    Kind::Failed,
                    format!(
                        "Task {} is neither running nor queued: nothing to cancel",
                        message.task
                    ),
                    None,
                )?;
            }
            Control::Ignore => eprintln!(
                "Ignoring a {:?} message from '{}': guest agents take tasks, answers and cancels",
                message.kind, message.from
            ),
        }
        Ok(false)
    }

    /// Keep a `result` event line in the record. Other lines are not kept; a record that cannot
    /// be written is reported and the task goes on.
    fn record(&mut self, task: &str, run_number: u32, line: &str) {
        if let (Some(log), Some(session)) = (&mut self.results, ResultLog::result_session(line))
            && let Err(error) = log.append(
                task,
                &session,
                run_number,
                &self.secrets.redact(line),
                now_ms(),
            )
        {
            eprintln!("Unable to record the result event: {error:#}");
        }
    }

    /// Add a finished run to its task's totals, and turn the event's session-wide API time into
    /// the run's own. Returns the task's totals including this run.
    fn count(&mut self, task: &str, session: &str, usage: &mut Usage) -> TaskUsage {
        let record = self.tasks.entry(task.to_owned()).or_default();
        let reported = usage.duration_api_ms;
        let previous = record
            .api
            .as_ref()
            .filter(|mark| mark.session == session)
            .map(|mark| mark.api_ms);
        usage.duration_api_ms = api_time_of_run(reported, previous);
        record.api = Some(ApiMark {
            session: session.to_owned(),
            api_ms: reported,
        });
        record.usage.add(usage);
        record.usage.clone()
    }

    /// Forward one update from Claude Code and keep what the outcome needs.
    fn apply(
        &mut self,
        update: Update,
        message: &Message,
        github: bool,
        model: &str,
        reply: &Reply<'_>,
        run: &mut RunState,
    ) -> Result<()> {
        if !matches!(update, Update::Finished { .. }) {
            run.progressed = true;
        }
        match update {
            Update::Session(session) => {
                self.tasks.entry(message.task.clone()).or_default().session = Some(Session {
                    id: session,
                    github,
                    model: Some(model.to_owned()),
                });
                self.save();
            }
            Update::Text(said) => {
                if let Some(earlier) = run.text.replace(said) {
                    reply.send(Kind::Progress, earlier, None)?;
                }
            }
            Update::Tool(running) => {
                if let Some(earlier) = run.text.take() {
                    reply.send(Kind::Progress, earlier, None)?;
                }
                reply.send(Kind::Progress, running.clone(), None)?;
                run.tool = Some(running);
            }
            Update::Finished {
                session,
                text: result,
                failed,
                mut usage,
            } => {
                let task_usage = self.count(&message.task, &session, &mut usage);
                self.tasks.entry(message.task.clone()).or_default().session = Some(Session {
                    id: session,
                    github,
                    model: Some(model.to_owned()),
                });
                self.save();
                if let Some(last) = run.text.take()
                    && !repeats_result(&last, &result)
                {
                    reply.send(Kind::Progress, last, None)?;
                }
                run.finished = Some(Finished {
                    text: result,
                    failed,
                    usage,
                    task_usage,
                });
            }
        }
        Ok(())
    }

    /// The launch for a new task: a fork of the base session for its rules and model, primed
    /// first when there is none or it is out of date. When priming fails the task gets a fresh
    /// session. `None` when the agent began stopping while priming: the task must not run.
    async fn fork_of_base(
        &mut self,
        access: Access<'a>,
        model: &str,
        task: &str,
    ) -> Option<Launch> {
        let github = access.github();
        let inputs = Inputs::current(self.workdir, &rules(github), model).await;
        let now = now_ms();
        let key = (github, model.to_owned());
        if let Some(base) = self.bases.get_mut(&key)
            && !base.needs_rebuild(&inputs, now)
        {
            base.touch(now);
            let session = base.session.clone();
            self.save();
            return Some(Launch::Fork(session));
        }
        if self.bases.remove(&key).is_some() {
            self.save();
        }
        match self.prime(access, model, task).await {
            Ok(session) => {
                self.bases.insert(
                    key,
                    Base::new(github, model, session.clone(), &inputs, now_ms()),
                );
                self.save();
                Some(Launch::Fork(session))
            }
            Err(error) if *self.stop.borrow() => {
                let error = format!("{error:#}");
                eprintln!(
                    "Priming stopped ({}): task {task} stays queued",
                    self.secrets.redact(&error)
                );
                None
            }
            Err(error) => {
                let error = format!("{error:#}");
                eprintln!(
                    "Unable to prime a base session: {}: task {task} runs in a fresh session",
                    self.secrets.redact(&error)
                );
                Some(Launch::Fresh)
            }
        }
    }

    /// Make a base session with two turns: the priming prompt in a new session, then a resume of
    /// that session. Both runs are counted in the totals of `task`, the task that needed them.
    #[instrument(level = "debug", skip_all, fields(%task, %model, github = access.github()))]
    async fn prime(&mut self, access: Access<'a>, model: &str, task: &str) -> Result<String> {
        let session = self
            .priming_run(access, model, task, &Launch::Fresh, PRIME_PROMPT)
            .await?;
        self.priming_run(access, model, task, &Launch::Resume(session), PRIME_CONFIRM)
            .await
    }

    /// One priming run of Claude Code with `prompt`. Returns the session it ended in.
    #[instrument(
        level = "debug",
        skip_all,
        fields(%task, %model, github = access.github(), ?launch)
    )]
    async fn priming_run(
        &mut self,
        access: Access<'a>,
        model: &str,
        task: &str,
        launch: &Launch,
        prompt: &str,
    ) -> Result<String> {
        let mut command = claude_command(
            self.program,
            self.workdir,
            model,
            self.secrets.claude(),
            access,
            launch,
        );
        let mut child = command.spawn().context("Unable to start Claude Code")?;
        let mut group = Group::new(child.id());
        let mut stdin = child.stdin.take().context("Claude Code has no input")?;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .context("Unable to give Claude Code the priming prompt")?;
        drop(stdin);
        let stdout = child.stdout.take().context("Claude Code has no output")?;
        let mut lines = BufReader::new(stdout).lines();
        let mut finished = None;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line.context("Unable to read Claude Code's output")?,
                () = stopped(&mut self.stop) => {
                    terminate(&mut child, &mut group).await;
                    bail!("The guest agent is stopping");
                }
            };
            let Some(line) = line else {
                break;
            };
            // Run 0 in the record: the priming runs come before the task's first.
            self.record(task, 0, &line);
            for update in interpret(&line) {
                if let Update::Finished {
                    session,
                    text,
                    failed,
                    usage,
                } = update
                {
                    finished = Some((session, text, failed, usage));
                }
            }
        }
        let status = child.wait().await.context("Claude Code did not exit")?;
        finish(&mut group).await;
        let (session, text, failed, mut usage) =
            finished.with_context(|| format!("Claude Code exited ({status}) without a result"))?;
        self.count(task, &session, &mut usage);
        self.save();
        eprintln!(
            "{}",
            priming_line(task, access.github(), model, &session, &usage)
        );
        ensure!(!failed, "The priming run failed: {text}");
        Ok(session)
    }

    /// Run Claude Code on one task or answer, on the task's model, forwarding its progress and
    /// its outcome. A new task forks the base session; if that fails it runs in a fresh session
    /// instead.
    #[instrument(
        skip_all,
        fields(task = %message.task, kind = ?message.kind, from = %message.from)
    )]
    async fn work(&mut self, message: &Message) -> Result<Finish> {
        let known = self
            .tasks
            .get(&message.task)
            .and_then(|record| record.session.clone());
        let github = task_github(message, known.as_ref());
        let access = self.access(github)?;
        let model = task_model(message, known.as_ref(), self.model)?;
        let launch = match (known, message.kind) {
            (Some(session), _) => Launch::Resume(session.id),
            (None, Kind::Answer) => {
                bail!(
                    "No session for task {}: an answer must follow a question",
                    message.task
                )
            }
            (None, _) => match self.fork_of_base(access, &model, &message.task).await {
                Some(launch) => launch,
                None => return Ok(Finish::Postponed),
            },
        };
        match self.attempt(message, access, &model, &launch).await? {
            Attempt::Done(finish) => Ok(finish),
            Attempt::ForkFailed(reason) => {
                eprintln!(
                    "Unable to fork the base session for task {}: {}: running it in a fresh session",
                    message.task,
                    self.secrets.redact(&reason)
                );
                self.bases.remove(&(github, model.clone()));
                self.save();
                match self
                    .attempt(message, access, &model, &Launch::Fresh)
                    .await?
                {
                    Attempt::Done(finish) => Ok(finish),
                    Attempt::ForkFailed(reason) => bail!(reason),
                }
            }
        }
    }

    /// One run of Claude Code for a task. Messages that arrive meanwhile are queued, or cancel
    /// the task.
    #[instrument(
        level = "debug",
        skip_all,
        fields(task = %message.task, %model, github = access.github(), ?launch)
    )]
    async fn attempt(
        &mut self,
        message: &Message,
        access: Access<'a>,
        model: &str,
        launch: &Launch,
    ) -> Result<Attempt> {
        if *self.stop.borrow() {
            return Ok(Attempt::Done(Finish::Postponed));
        }
        let reply = self.reply(&message.task, Some(model));
        let mut command = claude_command(
            self.program,
            self.workdir,
            model,
            self.secrets.claude(),
            access,
            launch,
        );
        let run_number = {
            let record = self.tasks.entry(message.task.clone()).or_default();
            record.runs = record.runs.saturating_add(1);
            record.runs
        };
        let mut child = command.spawn().context("Unable to start Claude Code")?;
        let mut group = Group::new(child.id());
        // The prompt goes in on stdin, so the tool lists cannot swallow it as an argument.
        let mut stdin = child.stdin.take().context("Claude Code has no input")?;
        stdin
            .write_all(message.text.as_bytes())
            .await
            .context("Unable to give Claude Code the task")?;
        drop(stdin);
        let stdout = child.stdout.take().context("Claude Code has no output")?;
        let mut lines = BufReader::new(stdout).lines();
        let mut run = RunState::default();
        let mut inbox_open = true;
        loop {
            let event = tokio::select! {
                line = lines.next_line() => {
                    Event::Line(line.context("Unable to read Claude Code's output")?)
                }
                incoming = self.incoming.recv(), if inbox_open => Event::Incoming(incoming.map(Box::new)),
                () = stopped(&mut self.stop) => Event::Stop,
            };
            let line = match event {
                Event::Line(Some(line)) => line,
                Event::Line(None) => break,
                Event::Incoming(None) => {
                    inbox_open = false;
                    continue;
                }
                Event::Incoming(Some(incoming)) => {
                    if self.control(&incoming, Some(&message.task))? {
                        terminate(&mut child, &mut group).await;
                        eprintln!("Canceled task {}", message.task);
                        reply.send(
                            Kind::Canceled,
                            canceled_text(
                                run.text.take().or(run.tool.take()),
                                run.finished.is_some(),
                            ),
                            run.finished
                                .map(|finished| (finished.usage, finished.task_usage)),
                        )?;
                        return Ok(Attempt::Done(Finish::Canceled));
                    }
                    continue;
                }
                Event::Stop => {
                    terminate(&mut child, &mut group).await;
                    reply.send(
                        Kind::Failed,
                        "The guest agent stopped: the task was interrupted".to_owned(),
                        run.finished
                            .map(|finished| (finished.usage, finished.task_usage)),
                    )?;
                    return Ok(Attempt::Done(Finish::Stopped));
                }
            };
            self.record(&message.task, run_number, &line);
            for update in interpret(&line) {
                self.apply(update, message, access.github(), model, &reply, &mut run)?;
            }
        }
        let status = child.wait().await.context("Claude Code did not exit")?;
        finish(&mut group).await;
        // A fork that fails leaves no turn behind: the task can still run in a fresh session.
        // A run that did something and then ended without a result is the task's failure: run
        // again, it would repeat what was done.
        if matches!(launch, Launch::Fork(_))
            && match &run.finished {
                None => !run.progressed,
                Some(finished) => finished.failed && finished.usage.num_turns == 0,
            }
        {
            let reason = run.finished.map_or_else(
                || format!("Claude Code exited ({status}) without a result"),
                |finished| finished.text,
            );
            return Ok(Attempt::ForkFailed(reason));
        }
        let finished = run
            .finished
            .with_context(|| format!("Claude Code exited ({status}) without a result"))?;
        let note = self.generation(message, access, model, &finished).await;
        report(&reply, model, finished, note)?;
        Ok(Attempt::Done(Finish::Done))
    }

    /// Record a finished run in the generation ledger and, for a `--github` task, rewrite the
    /// Generation section of the open pull requests of the branch the run ended on, with every
    /// task the ledger has on it. `None` for a task without GitHub, which is only recorded: the
    /// write token is for `--github` tasks alone. Nothing here fails the task.
    #[instrument(level = "debug", skip_all, fields(task = %message.task, github = access.github()))]
    async fn generation(
        &self,
        message: &Message,
        access: Access<'a>,
        model: &str,
        finished: &Finished,
    ) -> Option<Note> {
        let task = message.task.as_str();
        let branch = current_branch(self.workdir).await;
        let mut models: Vec<String> = finished
            .usage
            .models
            .iter()
            .map(|used| used.model.clone())
            .filter(|used| !used.is_empty())
            .collect();
        if models.is_empty() {
            models.push(model.to_owned());
        }
        let run = Run {
            task,
            agent: self.name,
            branch: branch.as_ref().ok().and_then(Option::as_deref),
            models: &models,
            usage: &finished.task_usage,
        };
        let recorded = match &self.ledger {
            Some(ledger) => ledger.record(&run, now_ms()),
            None => Err(anyhow!(
                "there is no state directory to keep the generation ledger in"
            )),
        };
        let Access::Write(token) = access else {
            if let Err(error) = &recorded {
                eprintln!("Unable to record task {task} in the generation ledger: {error:#}");
            }
            return None;
        };
        let note = match (branch, recorded) {
            (Err(error), _) | (Ok(Some(_)), Err(error)) => Note::Skipped(format!("{error:#}")),
            (Ok(None), _) => Note::Skipped("the checkout is on a detached HEAD".to_owned()),
            (Ok(Some(branch)), Ok(rows)) => {
                let outcomes =
                    update(self.gh, &checkouts(self.workdir), &branch, &rows, token).await;
                Note::Checked { branch, outcomes }
            }
        };
        eprintln!("Task {task}: {}", self.secrets.redact(&note.text()));
        Some(note)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use tokio::sync::oneshot;
    use tokio::time::timeout;

    use crate::guest::github::{REDACTED, WriteToken};
    use crate::guest::secrets::{ClaudeToken, REDACTED_CLAUDE};
    use crate::keys::{Seed, generate};

    const CLAUDE_TOKEN: &str = "sk-ant-oat01-harness0123456789";

    /// The guest agent's secrets, with `write_token` if given.
    fn secrets(write_token: Option<&str>) -> Secrets {
        Secrets::new(
            ClaudeToken::parse(CLAUDE_TOKEN).unwrap(),
            write_token.and_then(|token| WriteToken::parse(token).unwrap()),
            Seed::parse(&generate().unwrap().seed).unwrap(),
        )
    }

    fn queued(kind: Kind, task: &str) -> Message {
        Message {
            task: task.to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind,
            text: String::new(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    #[test]
    fn a_priming_run_is_counted_in_the_task_that_needed_it() {
        let (_input, incoming) = unbounded_channel();
        let (outgoing, _output) = unbounded_channel();
        let (_stop, stop_rx) = watch::channel(false);
        let secrets = secrets(None);
        let setup = Setup {
            program: CLAUDE,
            gh: GH,
            workdir: Path::new("/work"),
            model: "test-model",
            secrets: &secrets,
        };
        let mut engine = Engine::new(setup, "dev", "host", incoming, outgoing, stop_rx);
        // The priming runs, in the base session; then the task's own run in its fork.
        let mut priming = Usage {
            cache_creation: 9148,
            cost_usd: 0.038,
            duration_api_ms: 1500,
            ..Usage::default()
        };
        engine.count("t1", "base", &mut priming);
        let mut confirm = Usage {
            cache_read: 18775,
            cost_usd: 0.017,
            duration_api_ms: 1000,
            ..Usage::default()
        };
        engine.count("t1", "base", &mut confirm);
        let mut task_run = Usage {
            cache_read: 22361,
            cost_usd: 0.002,
            duration_api_ms: 900,
            ..Usage::default()
        };
        let total = engine.count("t1", "fork", &mut task_run);
        assert_eq!(total.runs, 3);
        assert_eq!(total.cache_creation, 9148);
        assert_eq!(total.cache_read, 18775 + 22361);
        assert!((total.cost_usd - 0.057).abs() < 1e-9);
        // The fork is another session: its API time is not reduced by the base's.
        assert_eq!(task_run.duration_api_ms, 900);
        assert_eq!(total.duration_api_ms, 3400);
        // Another task is not charged for it.
        let mut other = Usage::default();
        assert_eq!(engine.count("t2", "fork-2", &mut other).runs, 1);
    }

    // ---- integration: a fake claude, real processes and signals ----

    /// A stand-in for `claude`: prints stream-json, and for a prompt starting `hang` starts
    /// `sleep 300` and waits, writing the pids of both to files named after the prompt.
    const FAKE_CLAUDE: &str = r#"#!/bin/bash
read -r prompt
echo "$*" >> args.log
case "$*" in
*--fork-session*)
    case "$prompt" in
    nofork*) exit 1 ;;
    esac
    ;;
esac
case "$prompt" in
"You are a guest agent"*)
    if [ -e hang-prime ]; then
        sleep 300 &
        echo $! > prime.sleep
        echo $$ > prime.claude
        wait
    fi
    ;;
esac
echo '{"type":"system","subtype":"init","session_id":"sess-1"}'
case "$prompt" in
tool*)
    echo '{"type":"assistant","session_id":"sess-1","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"python3 -c sleep"}}]}}'
    echo $$ > "$prompt.claude"
    sleep 300 &
    echo $! > "$prompt.sleep"
    wait
    ;;
esac
echo '{"type":"assistant","session_id":"sess-1","message":{"content":[{"type":"text","text":"working"}]}}'
case "$prompt" in
hang*)
    sleep 300 &
    echo $! > "$prompt.sleep"
    echo $$ > "$prompt.claude"
    wait
    ;;
ask*)
    echo '{"type":"result","is_error":false,"result":"QUESTION: which one?","session_id":"sess-1"}'
    ;;
crash*)
    exit 1
    ;;
token*)
    echo "{\"type\":\"result\",\"is_error\":false,\"result\":\"GH_TOKEN=${GH_TOKEN:-none}\",\"session_id\":\"sess-1\"}"
    ;;
oauth*)
    echo "{\"type\":\"assistant\",\"session_id\":\"sess-1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"saw ${CLAUDE_CODE_OAUTH_TOKEN:-none}\"}]}}"
    echo "{\"type\":\"result\",\"is_error\":false,\"result\":\"CLAUDE_CODE_OAUTH_TOKEN=${CLAUDE_CODE_OAUTH_TOKEN:-none} AGENT_BUS_SEED=${AGENT_BUS_SEED:-none}\",\"session_id\":\"sess-1\"}"
    ;;
bg*)
    ( trap '' TERM; exec sleep 300 ) >/dev/null 2>&1 &
    echo $! > "$prompt.sleep"
    echo '{"type":"result","is_error":false,"result":"done","session_id":"sess-1"}'
    ;;
*)
    echo '{"type":"result","is_error":false,"result":"done","session_id":"sess-1"}'
    ;;
esac
"#;

    /// A stand-in for `gh`: `pr list` prints `prs.json` (no pull request without it), `pr edit
    /// <n>` saves its stdin as `body-<n>.md` or fails when `edit-fails` exists. Every call is
    /// logged to `gh.log` with its arguments and whether `GH_TOKEN` was set.
    const FAKE_GH: &str = r#"#!/bin/bash
echo "$* GH_TOKEN=${GH_TOKEN:+set}" >> gh.log
case "$1 $2" in
"pr list")
    if [ -e prs.json ]; then cat prs.json; else echo '[]'; fi
    ;;
"pr edit")
    if [ -e edit-fails ]; then echo "GraphQL: Resource not accessible" >&2; exit 1; fi
    cat > "body-$3.md"
    ;;
esac
"#;

    /// A guest agent's engine started by `Harness::spawn`, and its ends.
    struct Spawned {
        input: UnboundedSender<Message>,
        output: UnboundedReceiver<Message>,
        stop: watch::Sender<bool>,
        done: tokio::task::JoinHandle<Result<()>>,
        status: oneshot::Receiver<watch::Receiver<Status>>,
    }

    struct Harness {
        dir: std::path::PathBuf,
        input: Option<UnboundedSender<Message>>,
        output: UnboundedReceiver<Message>,
        stop: watch::Sender<bool>,
        done: tokio::task::JoinHandle<Result<()>>,
        /// The engine's status, once it has started.
        status: oneshot::Receiver<watch::Receiver<Status>>,
        /// The GitHub write token the agent was started with.
        write_token: Option<String>,
    }

    impl Harness {
        fn start(label: &str) -> Self {
            Self::start_with(label, None)
        }

        /// A guest agent started with `write_token`.
        fn start_with(label: &str, write_token: Option<&str>) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                env::temp_dir().join(format!("agent-bus-cancel-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let program = dir.join("fake-claude");
            std::fs::write(&program, FAKE_CLAUDE).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let gh = dir.join("fake-gh");
            std::fs::write(&gh, FAKE_GH).unwrap();
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
            let Spawned {
                input,
                output,
                stop,
                done,
                status,
            } = Self::spawn(&dir, write_token);
            Self {
                dir,
                input: Some(input),
                output,
                stop,
                done,
                status,
                write_token: write_token.map(str::to_owned),
            }
        }

        /// A guest agent on `dir`, keeping its state in `dir/state/`.
        fn spawn(dir: &Path, write_token: Option<&str>) -> Spawned {
            let write_token = write_token.map(str::to_owned);
            let (status_tx, status) = oneshot::channel();
            let (input, incoming) = unbounded_channel();
            let (outgoing, output) = unbounded_channel();
            let (stop, stop_rx) = watch::channel(false);
            let work = dir.to_owned();
            let program = dir.join("fake-claude").to_string_lossy().into_owned();
            let gh = dir.join("fake-gh").to_string_lossy().into_owned();
            let done = tokio::spawn(async move {
                let secrets = secrets(write_token.as_deref());
                let setup = Setup {
                    program: &program,
                    gh: &gh,
                    workdir: &work,
                    model: "test-model",
                    secrets: &secrets,
                };
                let mut engine = Engine::new(setup, "dev", "host", incoming, outgoing, stop_rx);
                engine.results = Some(ResultLog::new(work.join("results.jsonl")));
                engine.ledger = Some(Ledger::new(work.join("state").join("generation.toml")));
                engine.resume(StateFile::new(work.join("state").join("guest-dev.toml")))?;
                let _ = status_tx.send(engine.status.subscribe());
                engine.serve().await
            });
            Spawned {
                input,
                output,
                stop,
                done,
                status,
            }
        }

        /// Start the guest agent again on the same checkout and state file, as a deploy does.
        fn restart(&mut self) {
            let Spawned {
                input,
                output,
                stop,
                done,
                status,
            } = Self::spawn(&self.dir, self.write_token.as_deref());
            self.input = Some(input);
            self.output = output;
            self.stop = stop;
            self.done = done;
            self.status = status;
        }

        /// Make the checkout a git repository on `branch`.
        fn checkout(&self, branch: &str) {
            let status = std::process::Command::new("git")
                .args(["init", "-q", "-b", branch])
                .current_dir(&self.dir)
                .status()
                .unwrap();
            assert!(status.success());
        }

        fn saved(&self) -> Saved {
            StateFile::new(self.dir.join("state").join("guest-dev.toml")).load(now_ms())
        }

        /// Wait until the state file has the session of `task`: Claude Code names it before
        /// the fake writes its pids, but the engine may not have read that line yet.
        async fn session_saved(&self, task: &str) {
            for _ in 0..100 {
                if self
                    .saved()
                    .tasks
                    .get(task)
                    .is_some_and(|record| record.session.is_some())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("the session of {task} was never saved");
        }

        fn send(&self, kind: Kind, task: &str, text: &str) {
            self.send_on(kind, task, text, None);
        }

        fn send_on(&self, kind: Kind, task: &str, text: &str, model: Option<&str>) {
            let mut message = queued(kind, task);
            message.text = text.to_owned();
            message.model = model.map(str::to_owned);
            self.input.as_ref().unwrap().send(message).unwrap();
        }

        /// Send a task with GitHub write access.
        fn send_github(&self, task: &str, text: &str) {
            let mut message = queued(Kind::Task, task);
            message.text = text.to_owned();
            message.github = true;
            self.input.as_ref().unwrap().send(message).unwrap();
        }

        /// The next reply that is not progress.
        async fn outcome(&mut self) -> Message {
            loop {
                let message = timeout(Duration::from_secs(10), self.output.recv())
                    .await
                    .expect("no reply in time")
                    .expect("the agent stopped");
                if message.kind != Kind::Progress {
                    return message;
                }
            }
        }

        /// Every reply that is not progress, until the agent's side of the channel closes:
        /// call it once the agent has stopped.
        async fn rest(&mut self) -> Vec<Message> {
            let mut rest = Vec::new();
            while let Some(message) = timeout(Duration::from_secs(10), self.output.recv())
                .await
                .expect("the reply channel did not close in time")
            {
                if message.kind != Kind::Progress {
                    rest.push(message);
                }
            }
            rest
        }

        /// The pids a hanging prompt wrote, once it has.
        async fn pids(&self, prompt: &str) -> (i32, i32) {
            for _ in 0..100 {
                let read = |suffix: &str| {
                    std::fs::read_to_string(self.dir.join(format!("{prompt}.{suffix}")))
                        .ok()
                        .and_then(|text| text.trim().parse::<i32>().ok())
                };
                if let (Some(claude), Some(sleep)) = (read("claude"), read("sleep")) {
                    return (claude, sleep);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("{prompt} never started");
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Whether the process is gone (a zombie waiting for its parent counts as gone).
    async fn gone(pid: i32) -> bool {
        for _ in 0..60 {
            let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
            let dead = state.is_none_or(|stat| {
                stat.rsplit(')')
                    .next()
                    .is_some_and(|rest| rest.trim_start().starts_with('Z'))
            });
            if dead {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn canceling_stops_the_whole_group_and_the_queue_goes_on() {
        let mut harness = Harness::start("cancel");
        harness.send(Kind::Task, "t1", "hang-a");
        harness.send(Kind::Task, "t2", "next");
        harness.send(Kind::Task, "t4", "never");
        let (claude, sleep) = harness.pids("hang-a").await;

        // A queued task is dropped, and reported.
        harness.send(Kind::Cancel, "t4", "");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Canceled, "t4"));

        // The running task is stopped, with its commands, in a few seconds.
        let started = std::time::Instant::now();
        harness.send(Kind::Cancel, "t1", "");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Canceled, "t1"));
        assert!(reply.text.contains("working"), "{}", reply.text);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(gone(claude).await, "the fake claude is still running");
        assert!(gone(sleep).await, "its sleep is still running");

        // The task queued behind it runs.
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t2"));

        // The canceled task's session is still usable: an answer resumes it.
        harness.send(Kind::Answer, "t1", "go on");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t1"));

        // Each result event is recorded with its task and run: t1's answer is its second run.
        let record = std::fs::read_to_string(harness.dir.join("results.jsonl")).unwrap();
        let lines: Vec<&str> = record.lines().collect();
        assert_eq!(lines.len(), 4, "{record}");
        // The first two are the priming runs, which t1 triggered: run 0.
        for line in &lines[..2] {
            assert!(line.contains(r#""task":"t1""#) && line.contains(r#""run":0"#));
        }
        assert!(lines[3].contains(r#""task":"t1""#) && lines[3].contains(r#""run":2"#));

        // A task that is neither running nor queued.
        harness.send(Kind::Cancel, "nope", "");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Failed, "nope"));
        assert!(reply.text.contains("neither running nor queued"));

        // Nothing else ran: t4 never started.
        harness.input = None;
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        while let Ok(message) = harness.output.try_recv() {
            assert_ne!(message.task, "t4", "{message:?}");
        }
        assert!(!harness.dir.join("never.claude").exists());
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn the_status_follows_the_running_task_and_the_queue() {
        let mut harness = Harness::start("status");
        let mut status = timeout(Duration::from_secs(10), &mut harness.status)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (status.borrow().running.clone(), status.borrow().queued),
            (None, 0)
        );
        let before = now_ms();
        harness.send(Kind::Task, "t1", "hang-a");
        harness.send(Kind::Task, "t2", "next");
        harness.pids("hang-a").await;
        let current = timeout(
            Duration::from_secs(10),
            status.wait_for(|status| status.queued == 1 && status.running.is_some()),
        )
        .await
        .unwrap()
        .unwrap()
        .clone();
        let running = current.running.unwrap();
        assert_eq!(running.task, "t1");
        assert!(running.since_ms >= before && running.since_ms <= now_ms());
        assert!(current.online);

        // The queued task runs once the running one is canceled, and the queue empties.
        harness.send(Kind::Cancel, "t1", "");
        timeout(
            Duration::from_secs(10),
            status.wait_for(|status| {
                status.queued == 0 && status.running.as_ref().is_none_or(|r| r.task != "t1")
            }),
        )
        .await
        .unwrap()
        .unwrap();

        // Once the engine stops, the status ends: the heartbeat writes its last one, offline.
        harness.stop.send_replace(true);
        timeout(Duration::from_secs(10), async {
            while status.changed().await.is_ok() {}
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn stopping_the_guest_agent_stops_the_running_group() {
        let mut harness = Harness::start("stop");
        harness.send(Kind::Task, "t1", "hang-a");
        let (claude, sleep) = harness.pids("hang-a").await;
        harness.stop.send_replace(true);
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(gone(claude).await, "the fake claude is still running");
        assert!(gone(sleep).await, "its sleep is still running");
        let reply = harness.outcome().await;
        assert_eq!(reply.kind, Kind::Failed);
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn canceling_a_run_whose_only_progress_was_a_tool_names_the_tool() {
        let mut harness = Harness::start("tool");
        harness.send(Kind::Task, "t1", "tool-a");
        harness.pids("tool-a").await;
        harness.send(Kind::Cancel, "t1", "");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Canceled, "t1"));
        assert!(
            reply.text.contains("Bash: python3 -c sleep"),
            "{}",
            reply.text
        );
    }

    /// The arguments each fake claude run was started with, one line per run.
    fn started_with(harness: &Harness) -> Vec<String> {
        std::fs::read_to_string(harness.dir.join("args.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn tasks_fork_a_primed_base_and_answers_resume_their_own_session() {
        let mut harness = Harness::start("fork");
        harness.send(Kind::Task, "t1", "plain-a");
        assert_eq!(harness.outcome().await.kind, Kind::Completed);
        harness.send(Kind::Task, "t2", "plain-b");
        let second = harness.outcome().await;
        assert_eq!((second.kind, second.task.as_str()), (Kind::Completed, "t2"));
        harness.send(Kind::Answer, "t1", "go on");
        assert_eq!(harness.outcome().await.kind, Kind::Completed);

        let runs = started_with(&harness);
        assert_eq!(runs.len(), 5, "{runs:?}");
        // Priming: a fresh session, then a resume of it. Then each task forks that base.
        assert!(!runs[0].contains("--resume"), "{}", runs[0]);
        assert!(runs[1].contains("--resume sess-1"), "{}", runs[1]);
        assert!(!runs[1].contains("--fork-session"), "{}", runs[1]);
        for fork in &runs[2..4] {
            assert!(fork.contains("--resume sess-1 --fork-session"), "{fork}");
        }
        // The answer resumes the task's session, without forking.
        assert!(runs[4].contains("--resume sess-1"), "{}", runs[4]);
        assert!(!runs[4].contains("--fork-session"), "{}", runs[4]);

        // The priming runs are in the first task's totals, and the second task is not charged.
        assert_eq!(harness_runs(&second), 1);
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn each_model_has_its_own_base_and_an_answer_keeps_its_tasks_model() {
        let mut harness = Harness::start("models");
        harness.send(Kind::Task, "t1", "plain-a");
        let reply = harness.outcome().await;
        assert_eq!(reply.model.as_deref(), Some("test-model"));
        harness.send_on(Kind::Task, "t2", "ask-b", Some("sonnet"));
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Question, "t2"));
        assert_eq!(reply.model.as_deref(), Some("claude-sonnet-5-5"));
        // Back on the default model: its base is still there.
        harness.send(Kind::Task, "t3", "plain-c");
        assert_eq!(harness.outcome().await.kind, Kind::Completed);
        // An answer cannot move its task to another model.
        harness.send_on(Kind::Answer, "t2", "go on", Some("haiku"));
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t2"));
        assert_eq!(reply.model.as_deref(), Some("claude-sonnet-5-5"));
        // A task naming a model that is not one fails without running.
        harness.send_on(Kind::Task, "t4", "plain-d", Some("gpt"));
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Failed, "t4"));
        assert!(reply.text.contains("Model 'gpt'"), "{}", reply.text);

        let runs = started_with(&harness);
        assert_eq!(runs.len(), 8, "{runs:?}");
        let on = |run: &str, model: &str| run.contains(&format!("--model {model} "));
        // t1: the default model's base is primed, then forked.
        for run in &runs[0..3] {
            assert!(on(run, "test-model"), "{run}");
        }
        assert!(!runs[0].contains("--resume"), "{}", runs[0]);
        // t2: sonnet primes its own base, then forks it.
        for run in &runs[3..6] {
            assert!(on(run, "claude-sonnet-5-5"), "{run}");
        }
        assert!(!runs[3].contains("--resume"), "{}", runs[3]);
        assert!(runs[5].contains("--fork-session"), "{}", runs[5]);
        // t3 forks the default model's base without priming again.
        assert!(on(&runs[6], "test-model"), "{}", runs[6]);
        assert!(runs[6].contains("--fork-session"), "{}", runs[6]);
        // The answer resumes t2 on sonnet.
        assert!(on(&runs[7], "claude-sonnet-5-5"), "{}", runs[7]);
        assert!(!runs[7].contains("--fork-session"), "{}", runs[7]);

        let saved = harness.saved();
        let mut bases: Vec<&str> = saved.bases.iter().map(|base| base.model.as_str()).collect();
        bases.sort_unstable();
        assert_eq!(bases, ["claude-sonnet-5-5", "test-model"]);
        let session = saved.tasks["t2"].session.as_ref().unwrap();
        assert_eq!(session.model.as_deref(), Some("claude-sonnet-5-5"));
    }

    fn harness_runs(message: &Message) -> u64 {
        message.task_usage.as_ref().map_or(0, |usage| usage.runs)
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn a_failed_fork_runs_the_task_in_a_fresh_session() {
        let mut harness = Harness::start("forkfail");
        harness.send(Kind::Task, "t1", "nofork-a");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t1"));
        let runs = started_with(&harness);
        assert_eq!(runs.len(), 4, "{runs:?}");
        assert!(runs[2].contains("--fork-session"), "{}", runs[2]);
        assert!(!runs[3].contains("--resume"), "{}", runs[3]);
        // The base was dropped, so the next task primes a new one.
        harness.send(Kind::Task, "t2", "plain-b");
        assert_eq!(harness.outcome().await.kind, Kind::Completed);
        let runs = started_with(&harness);
        assert!(!runs[4].contains("--resume"), "{}", runs[4]);
        assert!(runs[5].contains("--resume sess-1"), "{}", runs[5]);
        assert!(runs[6].contains("--fork-session"), "{}", runs[6]);
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn a_forked_run_that_made_progress_without_a_result_fails_and_is_not_rerun() {
        let mut harness = Harness::start("crash");
        harness.send(Kind::Task, "t1", "crash-a");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Failed, "t1"));
        assert!(reply.text.contains("without a result"), "{}", reply.text);
        // Priming twice, then the fork, which said something and crashed: no fresh run.
        let runs = started_with(&harness);
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert!(runs[2].contains("--fork-session"), "{}", runs[2]);
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn stopping_while_priming_keeps_the_task_queued() {
        let mut harness = Harness::start("primestop");
        std::fs::write(harness.dir.join("hang-prime"), "").unwrap();
        harness.send(Kind::Task, "t1", "plain-p");
        let (claude, sleep) = harness.pids("prime").await;
        harness.stop.send_replace(true);
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(gone(claude).await, "the priming claude is still running");
        assert!(gone(sleep).await, "its sleep is still running");
        // Not reported, not run fresh: the task waits in the queue for the restart.
        let replies = harness.rest().await;
        assert!(replies.is_empty(), "{replies:?}");
        assert_eq!(started_with(&harness).len(), 1);
        let saved = harness.saved();
        assert_eq!(saved.running, None);
        let queued: Vec<&str> = saved.queue.iter().map(|q| q.task.as_str()).collect();
        assert_eq!(queued, ["t1"]);

        std::fs::remove_file(harness.dir.join("hang-prime")).unwrap();
        harness.restart();
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t1"));
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn commands_left_running_after_a_normal_exit_are_stopped() {
        let mut harness = Harness::start("background");
        harness.send(Kind::Task, "t1", "bg-a");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t1"));
        let pid: i32 = std::fs::read_to_string(harness.dir.join("bg-a.sleep"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // It ignores SIGTERM, so only the SIGKILL after the grace period stops it.
        assert!(gone(pid).await, "the background command outlived the task");
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn a_restart_keeps_the_queue_and_the_sessions() {
        let mut harness = Harness::start("restart");
        // A task that asks a question, answered after the restart.
        harness.send(Kind::Task, "t0", "ask-a");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Question, "t0"));

        // Two tasks; the agent is stopped while the first runs.
        harness.send(Kind::Task, "t1", "hang-r");
        harness.send(Kind::Task, "t2", "plain-r");
        let (claude, _) = harness.pids("hang-r").await;
        harness.session_saved("t1").await;
        harness.stop.send_replace(true);
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(gone(claude).await, "the fake claude is still running");
        // The stopped agent's replies: t1 failed, once, and nothing else.
        let before: Vec<(Kind, String)> = harness
            .rest()
            .await
            .into_iter()
            .map(|reply| (reply.kind, reply.task))
            .collect();
        assert_eq!(before, [(Kind::Failed, "t1".to_owned())]);

        // The final state: t2 waits, nothing runs, and every task's session is kept.
        let saved = harness.saved();
        assert_eq!(saved.running, None);
        let queued: Vec<&str> = saved.queue.iter().map(|q| q.task.as_str()).collect();
        assert_eq!(queued, ["t2"]);
        assert_eq!(saved.bases.len(), 1);
        for task in ["t0", "t1"] {
            assert!(saved.tasks[task].session.is_some(), "{task}: {saved:?}");
        }
        let t0_runs = saved.tasks["t0"].usage.runs;

        // Started again: t1 was reported already, so the next outcome is t2's, which forks the
        // kept base rather than priming a new one.
        harness.restart();
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t2"));
        assert_eq!(harness_runs(&reply), 1);

        // The answer to the question asked before the restart resumes t0's session, and its
        // totals go on from before.
        harness.send(Kind::Answer, "t0", "go on");
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t0"));
        assert_eq!(harness_runs(&reply), t0_runs + 1);
        // Priming twice, then t0, t1, t2 forked, then the answer resumed.
        let runs = started_with(&harness);
        assert_eq!(runs.len(), 6, "{runs:?}");
        assert!(runs[4].contains("--fork-session"), "{}", runs[4]);
        assert!(runs[5].contains("--resume sess-1"), "{}", runs[5]);
        assert!(!runs[5].contains("--fork-session"), "{}", runs[5]);

        // The restarted agent never reported t1: across both, the host got one failure for it.
        harness.input = None;
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let after = harness.rest().await;
        assert!(after.iter().all(|reply| reply.task != "t1"), "{after:?}");
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn a_clean_stop_clears_the_running_task() {
        let mut harness = Harness::start("clean");
        harness.send(Kind::Task, "t1", "hang-c");
        harness.pids("hang-c").await;
        harness.session_saved("t1").await;
        assert_eq!(harness.saved().running.as_deref(), Some("t1"));
        harness.stop.send_replace(true);
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // Reported by the agent that stopped, and no longer running in the file, so a restart
        // has nothing to report.
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Failed, "t1"));
        assert!(
            reply.text.contains("task was interrupted"),
            "{}",
            reply.text
        );
        assert_eq!(harness.saved().running, None);
        harness.restart();
        harness.input = None;
        timeout(Duration::from_secs(5), &mut harness.done)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let after = harness.rest().await;
        assert!(after.is_empty(), "{after:?}");
    }

    #[tokio::test]
    #[ignore = "starts real processes and signals them; run with -- --ignored"]
    async fn a_task_running_when_the_agent_died_is_failed_on_restart() {
        let mut harness = Harness::start("died");
        harness.send(Kind::Task, "t1", "hang-d");
        harness.send(Kind::Task, "t2", "plain-d");
        let (claude, sleep) = harness.pids("hang-d").await;
        harness.session_saved("t1").await;
        // Killed without its shutdown path: nothing is reported, the file says t1 runs.
        harness.done.abort();
        let _ = (&mut harness.done).await;
        assert!(gone(claude).await, "the fake claude is still running");
        assert!(gone(sleep).await, "its sleep is still running");
        assert_eq!(harness.saved().running.as_deref(), Some("t1"));

        harness.restart();
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Failed, "t1"));
        assert!(
            reply
                .text
                .contains("guest agent restarted while this task ran"),
            "{}",
            reply.text
        );
        let reply = harness.outcome().await;
        assert_eq!((reply.kind, reply.task.as_str()), (Kind::Completed, "t2"));
        // Its session is kept: an answer resumes it.
        harness.send(Kind::Answer, "t1", "go on");
        let reply = harness.outcome().await;
        assert_eq!(
            (reply.kind, reply.task.as_str()),
            (Kind::Completed, "t1"),
            "{}",
            reply.text
        );
        assert_eq!(harness.saved().running, None);
    }

    const WRITE_TOKEN: &str = "github_pat_harnesswrite0123456789";

    /// Every reply until the outcome of the current run, progress included.
    async fn replies_until_outcome(harness: &mut Harness) -> Vec<Message> {
        let mut replies = Vec::new();
        loop {
            let message = timeout(Duration::from_secs(10), harness.output.recv())
                .await
                .expect("no reply in time")
                .expect("the agent stopped");
            let done = message.kind != Kind::Progress;
            replies.push(message);
            if done {
                return replies;
            }
        }
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn only_a_github_task_gets_the_write_token_and_it_never_leaves_the_agent() {
        let mut harness = Harness::start_with("write-token", Some(WRITE_TOKEN));

        harness.send(Kind::Task, "read-only", "token-ro");
        let replies = replies_until_outcome(&mut harness).await;
        let outcome = replies.last().unwrap();
        assert_eq!(outcome.kind, Kind::Completed);
        // The run saw the agent's own GH_TOKEN, whatever it is here, never the write token: it
        // is not there before redaction either.
        assert!(outcome.text.starts_with("GH_TOKEN="), "{}", outcome.text);
        for reply in &replies {
            assert!(!reply.text.contains(WRITE_TOKEN), "{}", reply.text);
            assert!(!reply.text.contains(REDACTED), "{}", reply.text);
        }

        harness.send_github("write", "token-rw");
        let replies = replies_until_outcome(&mut harness).await;
        let outcome = replies.last().unwrap();
        assert_eq!(outcome.kind, Kind::Completed);
        // The run had the write token, and the reply carries it redacted, after the line on the
        // Generation section.
        assert!(
            outcome.text.ends_with(&format!("\n\nGH_TOKEN={REDACTED}")),
            "{}",
            outcome.text
        );
        for reply in &replies {
            assert!(!reply.text.contains(WRITE_TOKEN), "{}", reply.text);
        }

        // Nor is it in the result records, the state or the command lines.
        for file in ["results.jsonl", "state/guest-dev.toml", "args.log"] {
            let kept = fs::read_to_string(harness.dir.join(file)).unwrap();
            assert!(!kept.contains(WRITE_TOKEN), "{file}: {kept}");
        }
    }

    #[tokio::test]
    #[ignore = "starts real processes; run with -- --ignored"]
    async fn every_run_gets_the_claude_token_and_it_never_leaves_the_agent() {
        let mut harness = Harness::start("claude-token");

        harness.send(Kind::Task, "claude-token", "oauth");
        let replies = replies_until_outcome(&mut harness).await;
        let outcome = replies.last().unwrap();
        assert_eq!(outcome.kind, Kind::Completed);
        // The run had the token in its environment, and no seed; the reply carries it redacted.
        assert_eq!(
            outcome.text,
            format!("CLAUDE_CODE_OAUTH_TOKEN={REDACTED_CLAUDE} AGENT_BUS_SEED=none")
        );
        assert!(
            replies
                .iter()
                .any(|reply| reply.text == format!("saw {REDACTED_CLAUDE}")),
            "{replies:#?}"
        );
        for reply in &replies {
            assert!(!reply.text.contains(CLAUDE_TOKEN), "{}", reply.text);
        }

        // Nor is it in the result records, the state or the command lines.
        for file in ["results.jsonl", "state/guest-dev.toml", "args.log"] {
            let kept = fs::read_to_string(harness.dir.join(file)).unwrap();
            assert!(!kept.contains(CLAUDE_TOKEN), "{file}: {kept}");
        }
    }

    #[tokio::test]
    async fn a_github_task_fails_clearly_without_a_write_token() {
        let mut harness = Harness::start("no-write-token");
        harness.send_github("write", "token-rw");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Failed);
        assert!(
            outcome.text.contains("no GitHub write token"),
            "{}",
            outcome.text
        );
        // Claude Code was never started for it.
        assert!(!harness.dir.join("args.log").exists());

        // A task without GitHub still runs.
        harness.send(Kind::Task, "read-only", "plain");
        assert_eq!(harness.outcome().await.kind, Kind::Completed);
    }

    #[tokio::test]
    async fn the_lock_is_checked_before_the_bus_is_reached() {
        let dir = env::temp_dir().join(format!("agent-bus-lock-first-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let state = dir.join("state").join("guest-dev.toml");
        let secrets = secrets(None);
        // Port 1 on loopback: nothing listens, so connecting is refused.
        let server = Endpoint {
            url: "nats://127.0.0.1:1".to_owned(),
            ca: dir.join("ca.pem"),
        };
        let attempt = || {
            start(
                &server,
                "dev",
                "host",
                Setup {
                    program: CLAUDE,
                    gh: GH,
                    workdir: &dir,
                    model: "model",
                    secrets: &secrets,
                },
                Some(StateFile::new(state.clone())),
            )
        };

        let lock = InstanceLock::acquire(&state.with_extension("lock")).unwrap();
        let error = format!("{:#}", Box::pin(attempt()).await.unwrap_err());
        assert!(
            error.contains("Another guest agent of this name is running"),
            "{error}"
        );
        assert!(!state.exists(), "the state was written");

        // With the lock free the same start gets past it, to the token or the bus.
        drop(lock);
        let error = format!("{:#}", Box::pin(attempt()).await.unwrap_err());
        assert!(
            !error.contains("Another guest agent of this name is running"),
            "{error}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A pull request of `branch` with `body`, as `gh pr list --json` gives it.
    fn open_pull_request(harness: &Harness, number: u64, body: &str) {
        let prs = serde_json::json!([{
            "number": number,
            "url": format!("https://example.invalid/pull/{number}"),
            "body": body,
            "isCrossRepository": false,
        }]);
        fs::write(harness.dir.join("prs.json"), prs.to_string()).unwrap();
    }

    #[tokio::test]
    async fn a_github_run_writes_the_section_with_the_branchs_earlier_tasks() {
        let mut harness = Harness::start_with("generation", Some(WRITE_TOKEN));
        harness.checkout("feat/x");
        open_pull_request(&harness, 7, "Body");

        // A task without GitHub is recorded, and never runs gh: it has no write token.
        harness.send(Kind::Task, "review", "plain");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Completed);
        assert_eq!(outcome.text, "done");
        assert!(!harness.dir.join("gh.log").exists());

        harness.send_github("write", "plain");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Completed);
        assert_eq!(
            outcome.text,
            "Generation section for branch feat/x: updated https://example.invalid/pull/7\n\ndone"
        );
        let body = fs::read_to_string(harness.dir.join("body-7.md")).unwrap();
        assert!(
            body.starts_with("Body\n\n<!-- agent-bus:generation -->"),
            "{body}"
        );
        // Each task's runs include the two priming runs of its base.
        for row in [
            "| review | dev | test-model | 3 | $0.0000 | n/a | 0 | 0 | 0 | 0 |",
            "| write | dev | test-model | 3 | $0.0000 | n/a | 0 | 0 | 0 | 0 |",
        ] {
            assert!(body.contains(row), "{row}: {body}");
        }
        let log = fs::read_to_string(harness.dir.join("gh.log")).unwrap();
        assert!(
            log.contains("pr edit 7 --body-file - GH_TOKEN=set"),
            "{log}"
        );
        assert!(!log.contains(WRITE_TOKEN), "{log}");
        assert!(!body.contains(WRITE_TOKEN), "{body}");
        let ledger = fs::read_to_string(harness.dir.join("state").join("generation.toml")).unwrap();
        assert!(ledger.contains("[tasks.review]"), "{ledger}");
        assert!(!ledger.contains(WRITE_TOKEN), "{ledger}");
    }

    #[tokio::test]
    async fn a_question_keeps_its_last_line_after_the_note() {
        let mut harness = Harness::start_with("generation-question", Some(WRITE_TOKEN));
        harness.checkout("feat/x");
        harness.send_github("ask", "ask");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Question);
        assert_eq!(
            outcome.text,
            "Generation section: no open pull request for branch feat/x\n\nQUESTION: which one?"
        );
    }

    #[tokio::test]
    async fn a_failed_section_update_is_reported_and_the_task_still_completes() {
        let mut harness = Harness::start_with("generation-fails", Some(WRITE_TOKEN));
        harness.checkout("feat/x");
        open_pull_request(&harness, 7, "Body");
        fs::write(harness.dir.join("edit-fails"), "").unwrap();
        harness.send_github("write", "plain");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Completed);
        assert!(
            outcome.text.starts_with(
                "Generation section for branch feat/x: not updated: https://example.invalid/pull/7: gh pr edit failed: GraphQL: Resource not accessible"
            ),
            "{}",
            outcome.text
        );
        assert!(outcome.text.ends_with("\n\ndone"), "{}", outcome.text);
    }

    #[tokio::test]
    async fn a_github_run_outside_a_checkout_says_why_and_completes() {
        let mut harness = Harness::start_with("generation-no-branch", Some(WRITE_TOKEN));
        harness.send_github("write", "plain");
        let outcome = harness.outcome().await;
        assert_eq!(outcome.kind, Kind::Completed);
        assert!(
            outcome
                .text
                .starts_with("Generation section not updated: Unable to find the branch of"),
            "{}",
            outcome.text
        );
        assert!(!harness.dir.join("gh.log").exists());
    }
}
