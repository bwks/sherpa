//! The guest agent's entry point and its loop: read the inbox, queue tasks, run them one at a
//! time through Claude Code, and send back the replies.

use std::collections::{HashMap, VecDeque};
use std::env;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::try_join;

use super::claude::{CLAUDE, Group, claude_command, terminate};
use super::engine::{Control, Session, canceled_text, classify, task_github};
use super::record::ResultLog;
use super::stream::{Update, interpret, outcome, repeats_result, usage_line};
use crate::channel::{Channel, Inbox};
use crate::message::{Kind, Message, TaskUsage, Usage, api_time_of_run, check_agent_name};

/// The Claude subscription token, from `claude setup-token`.
const TOKEN_VARIABLE: &str = "CLAUDE_CODE_OAUTH_TOKEN";

/// Where replies about one task go.
struct Reply {
    outgoing: UnboundedSender<Message>,
    task: String,
    from: String,
    to: String,
}

/// How a run of Claude Code ended.
enum Finish {
    Done,
    Canceled,
    /// The guest agent is shutting down.
    Stopped,
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

/// What a task has used over its runs, and where its session's API time stood after the last.
#[derive(Default)]
struct TaskTotals {
    usage: TaskUsage,
    /// The session and its `duration_api_ms` as the last run reported them. The figure counts
    /// the whole session, so a run's own is the increase.
    api: Option<(String, u64)>,
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
}

/// The guest agent's state: the tasks waiting, and the Claude sessions of tasks so far.
struct Engine<'a> {
    program: &'a str,
    workdir: &'a Path,
    name: &'a str,
    reply_to: &'a str,
    incoming: UnboundedReceiver<Message>,
    outgoing: UnboundedSender<Message>,
    stop: watch::Receiver<bool>,
    queue: VecDeque<Message>,
    sessions: HashMap<String, Session>,
    /// The runs so far of each task, for the record of result events.
    runs: HashMap<String, u32>,
    /// What each task has used, over the runs that sent a result event.
    totals: HashMap<String, TaskTotals>,
    results: Option<ResultLog>,
}

/// Take tasks from `name`'s inbox until the bus closes or the agent is told to stop (SIGTERM or
/// SIGINT), and reply to `reply_to`. Stopping also stops the running Claude Code and everything
/// it started.
///
/// # Errors
///
/// Fails when a name is invalid, the subscription token is not set, or the bus fails. A task
/// that fails is reported to `reply_to` instead.
pub async fn run(channel: &Channel, name: &str, reply_to: &str, workdir: &Path) -> Result<()> {
    check_agent_name(name)?;
    check_agent_name(reply_to)?;
    ensure!(
        env::var_os(TOKEN_VARIABLE).is_some(),
        "Set {TOKEN_VARIABLE}: guest agents use the Claude subscription, never API billing"
    );
    let inbox = channel.inbox(name).await?;
    let mut terminate = signal(SignalKind::terminate()).context("Unable to watch for SIGTERM")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("Unable to watch for SIGINT")?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let (incoming_tx, incoming_rx) = unbounded_channel();
    let (outgoing_tx, outgoing_rx) = unbounded_channel();
    let engine = Engine::new(
        CLAUDE,
        workdir,
        name,
        reply_to,
        incoming_rx,
        outgoing_tx,
        stop_rx.clone(),
    );
    eprintln!("Guest agent '{name}' is waiting for tasks");
    let work = async {
        try_join!(
            read_inbox(inbox, incoming_tx, stop_rx),
            forward(channel, outgoing_rx),
            engine.serve()
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
    mut inbox: Inbox,
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

/// Resolves when the agent is told to stop.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    // A closed channel also means stop.
    let _ = stop.wait_for(|stop| *stop).await;
}

impl Reply {
    fn send(&self, kind: Kind, text: String, usage: Option<(Usage, TaskUsage)>) -> Result<()> {
        let (usage, task_usage) = usage.unzip();
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
            })
            .map_err(|_| anyhow!("The bus connection is closed"))
    }
}

impl<'a> Engine<'a> {
    fn new(
        program: &'a str,
        workdir: &'a Path,
        name: &'a str,
        reply_to: &'a str,
        incoming: UnboundedReceiver<Message>,
        outgoing: UnboundedSender<Message>,
        stop: watch::Receiver<bool>,
    ) -> Self {
        Self {
            program,
            workdir,
            name,
            reply_to,
            incoming,
            outgoing,
            stop,
            queue: VecDeque::new(),
            sessions: HashMap::new(),
            runs: HashMap::new(),
            totals: HashMap::new(),
            results: ResultLog::standard(),
        }
    }

    fn reply(&self, task: &str) -> Reply {
        Reply {
            outgoing: self.outgoing.clone(),
            task: task.to_owned(),
            from: self.name.to_owned(),
            to: self.reply_to.to_owned(),
        }
    }

    /// Run the queued tasks, and the ones that arrive, one at a time, until the inbox closes or
    /// the agent is told to stop.
    async fn serve(mut self) -> Result<()> {
        loop {
            if *self.stop.borrow() {
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
            match self.work(&message).await {
                Ok(Finish::Stopped) => return Ok(()),
                Ok(Finish::Done | Finish::Canceled) => {}
                Err(error) => {
                    self.reply(&message.task)
                        .send(Kind::Failed, format!("{error:#}"), None)?;
                }
            }
        }
    }

    /// Handle a message that arrived while idle or while `running` runs. Returns whether it is
    /// a cancel for the running task, which the caller must carry out.
    fn control(&mut self, message: &Message, running: Option<&str>) -> Result<bool> {
        match classify(message, running, &self.queue) {
            Control::Queue => self.queue.push_back(message.clone()),
            Control::CancelRunning => return Ok(true),
            Control::CancelQueued => {
                self.queue.retain(|queued| queued.task != message.task);
                eprintln!("Canceled queued task {}", message.task);
                self.reply(&message.task).send(
                    Kind::Canceled,
                    "Canceled before it started".to_owned(),
                    None,
                )?;
            }
            Control::CancelUnknown => {
                self.reply(&message.task).send(
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
    fn record(&self, task: &str, run_number: u32, line: &str) {
        if let (Some(log), Some(session)) = (&self.results, ResultLog::result_session(line))
            && let Err(error) = log.append(task, &session, run_number, line)
        {
            eprintln!("Unable to record the result event: {error:#}");
        }
    }

    /// Add a finished run to its task's totals, and turn the event's session-wide API time into
    /// the run's own. Returns the task's totals including this run.
    fn count(&mut self, task: &str, session: &str, usage: &mut Usage) -> TaskUsage {
        let totals = self.totals.entry(task.to_owned()).or_default();
        let reported = usage.duration_api_ms;
        let previous = totals
            .api
            .as_ref()
            .filter(|(known, _)| known == session)
            .map(|(_, api_ms)| *api_ms);
        usage.duration_api_ms = api_time_of_run(reported, previous);
        totals.api = Some((session.to_owned(), reported));
        totals.usage.add(usage);
        totals.usage.clone()
    }

    /// Forward one update from Claude Code and keep what the outcome needs.
    fn apply(
        &mut self,
        update: Update,
        message: &Message,
        github: bool,
        reply: &Reply,
        run: &mut RunState,
    ) -> Result<()> {
        match update {
            Update::Session(session) => {
                self.sessions.insert(
                    message.task.clone(),
                    Session {
                        id: session,
                        github,
                    },
                );
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
                self.sessions.insert(
                    message.task.clone(),
                    Session {
                        id: session,
                        github,
                    },
                );
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

    /// Run Claude Code on one task or answer, forwarding its progress and its outcome. Messages
    /// that arrive meanwhile are queued, or cancel the task.
    async fn work(&mut self, message: &Message) -> Result<Finish> {
        let reply = self.reply(&message.task);
        let known = self.sessions.get(&message.task);
        let github = task_github(message, known);
        let mut command = claude_command(self.program, self.workdir, github);
        match (known, message.kind) {
            (Some(session), _) => {
                command.args(["--resume", &session.id]);
            }
            (None, Kind::Answer) => {
                bail!(
                    "No session for task {}: an answer must follow a question",
                    message.task
                )
            }
            (None, _) => {}
        }
        let run_number = {
            let runs = self.runs.entry(message.task.clone()).or_insert(0);
            *runs = runs.saturating_add(1);
            *runs
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
                        return Ok(Finish::Canceled);
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
                    return Ok(Finish::Stopped);
                }
            };
            self.record(&message.task, run_number, &line);
            for update in interpret(&line) {
                self.apply(update, message, github, &reply, &mut run)?;
            }
        }
        let status = child.wait().await.context("Claude Code did not exit")?;
        group.disarm();
        let finished = run
            .finished
            .with_context(|| format!("Claude Code exited ({status}) without a result"))?;
        let kind = outcome(&finished.text, finished.failed);
        eprintln!(
            "{}",
            usage_line(&message.task, kind, &finished.usage, &finished.task_usage)
        );
        reply.send(
            kind,
            finished.text,
            Some((finished.usage, finished.task_usage)),
        )?;
        Ok(Finish::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use tokio::time::timeout;

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
        }
    }

    // ---- integration: a fake claude, real processes and signals ----

    /// A stand-in for `claude`: prints stream-json, and for a prompt starting `hang` starts
    /// `sleep 300` and waits, writing the pids of both to files named after the prompt.
    const FAKE_CLAUDE: &str = r#"#!/bin/bash
read -r prompt
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
*)
    echo '{"type":"result","is_error":false,"result":"done","session_id":"sess-1"}'
    ;;
esac
"#;

    struct Harness {
        dir: std::path::PathBuf,
        input: Option<UnboundedSender<Message>>,
        output: UnboundedReceiver<Message>,
        stop: watch::Sender<bool>,
        done: tokio::task::JoinHandle<Result<()>>,
    }

    impl Harness {
        fn start(label: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                env::temp_dir().join(format!("agent-bus-cancel-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let program = dir.join("fake-claude");
            std::fs::write(&program, FAKE_CLAUDE).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let (input, incoming) = unbounded_channel();
            let (outgoing, output) = unbounded_channel();
            let (stop, stop_rx) = watch::channel(false);
            let work = dir.clone();
            let program = program.to_string_lossy().into_owned();
            let done = tokio::spawn(async move {
                let mut engine =
                    Engine::new(&program, &work, "dev", "host", incoming, outgoing, stop_rx);
                engine.results = Some(ResultLog::new(work.join("results.jsonl")));
                engine.serve().await
            });
            Self {
                dir,
                input: Some(input),
                output,
                stop,
                done,
            }
        }

        fn send(&self, kind: Kind, task: &str, text: &str) {
            let mut message = queued(kind, task);
            message.text = text.to_owned();
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
        assert_eq!(lines.len(), 2, "{record}");
        assert!(lines[1].contains(r#""task":"t1""#) && lines[1].contains(r#""run":2"#));

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
}
