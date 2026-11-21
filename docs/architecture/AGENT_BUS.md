# Agent Bus

The agent bus is a message channel between the AI coding agent the user works with on the host
(the host agent) and agents running in lab guests (guest agents). The host agent hands a guest
agent a whole task, and gets back its progress, its questions for the user, and its outcome.

The code is the `agent-bus` crate, `crates/agent-bus`. The specs in `tests/specs/` leave this
crate out, so this document is where its design is written down.

## Purpose and scope

- **A development tool, not released.** `agent-bus` has `publish = false`, is not installed by
  the install scripts, is not part of `sherpad` or the `sherpa` CLI, and has no user-facing
  documentation in `../sherpa-docs`.
- **Two roles.** The host agent sends tasks, answers and cancels, and reads the replies. A guest
  agent (`agent-bus guest`), in a lab guest such as the dev server (`tests/environments/dev`),
  takes each task to Claude Code in its checkout and replies.
- **Why.** Without the bus the host agent works over SSH one command at a time, and every build
  log lands in its context. With it, the host agent sees only progress lines and the outcome.
- **Not A2A.** The message kinds are named after A2A's task states, but the protocol is plain
  JSON on NATS subjects.

## Components

| Part | What it is |
|---|---|
| Bus container | `sherpa-agent-bus`, a `nats` image pinned by digest, run with JetStream on the host's network. It listens on one address, port 4222: with `--lab`, the lab's management gateway, which is also each guest's default gateway. Its store is the Docker volume `sherpa-agent-bus-data`, which outlives it. |
| Server config | `nats.conf`, rendered from the agents file and the registry of enrolled guests (`enrolled.toml`). Its directory is bind-mounted read-only into the container, and SIGHUP reloads it. |
| TLS | The bus's own private CA and the server certificate it signs for the listen address. See [TLS](#tls). |
| Stream | One JetStream stream, `AGENTS`, on `agents.>`, keeping messages for 30 days. |
| Inboxes | One durable pull consumer per agent, named after it, on `agents.<name>.>`. A message waits until its agent reads and acknowledges it, and a restarted reader goes on where it stopped. |
| Status bucket | The key-value bucket `AGENT_STATUS`: each guest agent's heartbeat. See [Agent status](#agent-status). |
| Record | `_record`, a durable consumer on every message, feeding `agent-bus.db`, the host's embedded Turso database of every message and what is derived from them. See [The record](#the-record). |
| Host commands | `key new`, `bus start/stop/status`, `enroll`, `unenroll`, `send`, `cancel`, `listen`, `agents`, and, with the `record` feature, `usage`, `show`, `history`, `open`, `link`, `work`, `host` and `generation`. |
| Guest agent | `agent-bus guest`, started by `dev/agent-bus/start-agent.sh` on the host through `start-guest.sh` in the guest: reads its inbox, runs tasks one at a time through `claude -p`, and replies. |

The only path between the host and a running guest agent is NATS over TLS on the lab's bridge.

```mermaid
flowchart LR
    subgraph host["Host"]
        hostagent["Host agent<br/>Claude Code"]
        cmds["agent-bus commands"]
        record[("agent-bus.db")]
        conf["nats.conf"]
        nats["sherpa-agent-bus container<br/>nats-server + JetStream<br/>on lab gateway:4222"]
    end
    subgraph guest["Lab guest"]
        gagent["agent-bus guest"]
        claude["claude -p"]
    end
    hostagent --> cmds
    cmds <-->|"NATS over TLS, NKey auth"| nats
    cmds --- record
    cmds -->|"render, SIGHUP"| conf
    conf -.->|"read-only mount"| nats
    gagent <-->|"NATS over TLS, NKey auth"| nats
    gagent <-->|"stdin, stream-json"| claude
```

### Starting the bus

`agent-bus bus start --lab <lab dir> --agents <file>`, with the host's seed in `AGENT_BUS_SEED`,
listens on the gateway in the lab's `.sherpa/lab-info.toml` (`--listen <ipv4>` names one). It
merges the agents file and the registry under the registry lock, writes `nats.conf`, makes the
CA and server certificate if needed (see [TLS](#tls)), and replaces the container: an existing
one is never just restarted, because the config may have changed, and the messages are in the
volume. It then creates whatever is missing (the stream, the status bucket, each agent's inbox
and `_record`), leaving what exists as the bus has it, so it is safe to repeat, and records the
URL, the agents file and the lab in `bus.toml`. `0.0.0.0` is refused: the certificate names the
one address clients connect to.

`bus stop` removes the container and `bus.toml`; the volume stays. `bus start` is needed again
only when the agents file changes: starting a guest enrolls it on the running bus.

### Finding the bus

`enroll` and `unenroll` use `bus.toml` for the URL and the agents file. Every other command that
connects takes `--server`, else `AGENT_BUS_URL`, else the URL in `bus.toml`. The guest agent has
no `bus.toml`: `start-agent.sh` passes the bus's URL as `AGENT_BUS_URL`, which for a guest in the
bus's lab is its own default gateway, and which a guest in another lab reaches through the host.
Every client trusts one CA: the file named by `AGENT_BUS_CA`, else `tls/ca.pem` in the host's
state directory.

### One reader per inbox

Readers of a durable consumer share its messages, so a second reader on one inbox would silently
take some of them. Inboxes have `max_waiting = 1`, and opening one whose consumer already has a
pull waiting is refused, naming the likely cause (an earlier `listen` still running). A killed
reader leaves no claim behind.

Every reader uses its own pull loop (`pull` module), not async-nats's pull stream, which overlaps
its pulls and silently retries what a `max_waiting = 1` consumer refuses. A pull asks for one
message and expires after 10 s; the next is sent only when the last is over. The reader writes a
notice to stderr for every answer that is not routine: a refusal, any other status, and a pull
the bus says nothing about for 15 s, which is counted lost and pulled again. A deleted or push
consumer ends the reader with an error. Each acknowledgement is flushed before the reader goes
on, so a command that exits right after its last message leaves none unacknowledged.

## Messages

Every message is one JSON object (`message::Message`), published to `agents.<to>.<from>` and
stored by JetStream before `send` returns. A guest may publish only under its own name, and every
reader drops, logs and acknowledges a message whose `to` and `from` are not its subject's, so a
guest cannot pose as another agent. Agent names are 1 to 32 lowercase letters, digits and
hyphens, so a name is one subject token. `Message` does not deny unknown fields and every
optional field defaults, so host and guest builds that differ by a field still read each other.

| Kind | Direction | Meaning |
|---|---|---|
| `task` | host to guest | Work to do, with optional `github`, `model` and `work` (the issues and pull requests it is for). |
| `answer` | host to guest | The answer to a question; resumes the task's Claude session. |
| `cancel` | host to guest | Stop the task, running or queued. |
| `progress` | guest to host | A text Claude said, or a tool call, cut to 300 characters. |
| `question` | guest to host | The run's result has a line starting `QUESTION:`. |
| `completed` | guest to host | The run ended without an error and without a question. |
| `failed` | guest to host | The run failed, Claude Code could not run, the agent stopped or restarted during the task, or a cancel named an unknown task. |
| `canceled` | guest to host | The task was canceled; the text holds the last progress. |

Outcomes carry the run's `usage`, the task's `task_usage` so far, the model it ran on, and
`ended`, where the guest's checkout was when the run ended (see [Work items](#work-items)).

### A task's lifecycle

The guest agent keeps no "waiting for an answer" state: an answer resumes any task whose session
it knows, after any outcome. "Queued" covers the inbox and the guest agent's local queue.

```mermaid
stateDiagram-v2
    [*] --> Queued: task
    Queued --> Running: its turn
    Queued --> Canceled: cancel while queued
    Running --> Running: progress
    Running --> Completed: completed
    Running --> Failed: failed
    Running --> Asked: question
    Running --> Canceled: cancel while running
    Asked --> Queued: answer
    Canceled --> Queued: answer, session kept
    Completed --> [*]
    Failed --> [*]
    Canceled --> [*]
```

## Flows

### A task with progress and a result

The guest agent runs the task through Claude Code, and each line of Claude Code's output that
means something to the host becomes a message. Claude's last text is also its result, so each
text is held until the next update shows whether it was the last; the final text is not sent
twice.

```mermaid
sequenceDiagram
    participant H as Host agent
    participant B as Bus
    participant G as Guest agent
    participant C as Claude Code
    H->>B: send --to dev "task" (kind task)
    B->>G: delivered from inbox dev
    G->>B: ack
    G->>C: claude -p --resume base --fork-session, task on stdin
    C-->>G: system event: session_id
    C-->>G: assistant: text, tool_use
    G->>B: progress
    B->>H: listen host prints one JSON line
    C-->>G: result event
    G->>B: completed, with usage and task_usage
    B->>H: listen host prints one JSON line
```

The guest agent acknowledges each inbox message as soon as it reads it, before running it: a
task runs far longer than the bus waits for an acknowledgement, and must not be delivered twice.

`agent-bus listen host`, run under Claude Code's Monitor on the host, appends each message to its
journal, then prints it as one JSON line, then acknowledges it, so each becomes a notification
for the host agent and one printed while no Monitor read it is still in the journal (`listen
host --since 30m` prints it again). A message that cannot be journaled or printed is not
acknowledged, and the listener exits; the bus delivers it again after 30 s. On stderr the
listener says once that the bus took its pull, within 10 s of starting; one that says nothing in
15 s never reached its inbox, so keep its stderr in the same log. Readers outlive a short bus
outage, such as `bus start` run again: the lost pull is noticed after 15 s and pulled again.

### A question and its answer

A result with a line starting `QUESTION:` makes the outcome a `question`. The host agent asks the
user and sends the answer; the guest agent resumes the task's own session with it, with the
GitHub flag and model stored for that session.

### Cancelling a running task

While a task runs, the guest agent reads its inbox alongside Claude Code's output. Tasks and
answers that arrive wait in the local queue; a cancel for the running task is acted on at once.
Claude Code runs in its own process group, so the signals reach every command it started:
SIGTERM to the group, up to 2 s for its leader to exit, then SIGKILL for anything left. The reply
is `canceled`, with usage only if a `result` event arrived first; the session is kept. The group
is also killed when its guard is dropped, and whatever Claude Code left in it when it exits on
its own (a command started with `&` or `nohup`) is killed the same way, so no way out of a run
leaves a process behind. The guest agent is not in the group.

### A guest restart

The guest agent is restarted on every deploy (`start-guest.sh`). Its state file and lock make
the restart lose nothing and report an interrupted task exactly once. On SIGTERM the old agent
stops Claude Code's group, replies `failed`, writes its final state with nothing running and its
queue kept, and acknowledges nothing more, so the bus delivers what came after the stop again. An
agent killed after 15 s may leave a running task in its state file; the new agent reports that
task `failed`, then runs the saved queue before the inbox. The task's session is kept, so an
answer still resumes it. A stop while a base session is being primed is not an interruption: the
task goes back to the front of the queue.

## Authentication and permissions

### NKeys

Every agent has an NKey user key pair. On connect, the server sends a nonce, the client signs it
with its seed, and the server checks the signature against the public keys in its config; no
password or token goes over the wire. NKeys authenticate; TLS encrypts. Public keys are in the
agents file, the registry and `nats.conf`. The host's seed lives in the user's secret store and
reaches one command at a time as `AGENT_BUS_SEED`; a guest's seed is made fresh by `enroll` each
time it starts and is never at rest. No two agents may share a key, because the bus authorizes
keys, not names. A seed is never a flag, a file or a log line, and no error quotes it.

### The guest agent's secrets

A guest agent holds three secrets: the Claude subscription token, the GitHub write token and its
bus seed. Its tasks run as the same user, so they can read `/proc/<agent pid>/environ` and
`cmdline`, its state files and its log. None of the three is in any of them:

- **The path.** `start-agent.sh` sends them down the SSH stdin pipe, one per line (an empty line
  for no write token), then the bus's CA certificate. `start-guest.sh` reads each into an
  unexported variable, pipes the secrets with the `printf` builtin into the stdin of `agent-bus
  guest`, started without `AGENT_BUS_SEED` or `CLAUDE_CODE_OAUTH_TOKEN` in its environment, and
  unsets its copies.
- **In the agent** (`guest::secrets`). Each line is checked for its shape and a line out of order
  is refused, without quoting it. Each secret's type has a redacted `Debug` and no `Display`. The
  agent refuses to start with either variable in its environment, or with the write token as its
  own `GH_TOKEN`.
- **Where each goes.** The seed only signs the bus's challenge. The Claude token is set as
  `CLAUDE_CODE_OAUTH_TOKEN` in each Claude Code run's environment, the only place Claude Code
  takes it from; the write token as `GH_TOKEN` in a `--github` run's alone.
- **Redaction.** Every text the agent sends or records, and its own log lines that can carry
  Claude Code's text, have each secret replaced by a placeholder.

`kernel.yama.ptrace_scope = 1` on the guests keeps a task from reading the agent's memory. What a
task can still reach is in [Limits](#limits).

### GitHub tokens

`bwks/sherpa` is public, so text from strangers reaches guest agents that run with
`--dangerously-skip-permissions` and root, and the GitHub token in reach decides what an
injected instruction can do. So there are two tokens, and the token's permissions, not the deny
lists, are the boundary.

| Token | What it can do | Where it lives in a guest | Which tasks have it |
|---|---|---|---|
| Read-only (`GH_TOKEN_RO` on the host) | Read issues, pull requests, checks, runs and their logs | `~/.env.d/github.env` as `GH_TOKEN`, written by provisioning; so the login shell and the guest agent's environment | Every task without `--github` |
| Write (`GH_TOKEN` on the host) | Commit, push, open pull requests | Nowhere on disk: the guest agent's memory | `--github` tasks only, as `GH_TOKEN` in their Claude Code's environment |

The write token is never in a file, a log, a command line, the bus, or an environment the agent
or a task can read back. The guest agent never uses it itself: the host writes the Generation
section with its own token. A guest agent started without one runs, and its `--github` tasks
fail at once. Git pushes through `gh auth git-credential`, which takes `GH_TOKEN` from each git
command's environment, so in a task without `--github` GitHub refuses the push.

### TLS

The bus has its own private CA, made by `bus start` (`tls` module). It is not the Sherpa lab CA:
the bus is a development tool, and no other certificate should be able to pose as it. Every
connection is TLS from its first byte, and every client trusts only that CA.

- **The CA** is made once, when neither `tls/ca.pem` nor `tls/ca-key.pem` exists, and reused;
  one without the other is an error. A new CA means restarting every guest agent. The key never
  leaves `tls/`, which is not mounted.
- **The server certificate** sits beside `nats.conf`, so the container reads it through the same
  mount. Its one IP SAN is the listen address, the one address everyone connects to. It is
  valid a year, from an hour before issue so a guest whose clock lags still accepts it.
  `tls/server.toml` records what it names and its expiry, so `bus start` reissues it, without
  parsing it, when the CA or the address changed or it expires within 30 days.
- **The server** uses `handshake_first`, so it sends nothing in plaintext, not even its INFO.
  There are no client certificates: NKeys authenticate clients.
- **The clients** connect TLS-first with only the CA as a root, so no system roots are loaded.
  The rustls provider is aws-lc-rs, as in the rest of the workspace; async-nats's default
  features are off, so `ring` is not built (see `BUILD.md`).
- **The guests** get `tls/ca.pem` down the SSH stdin pipe after the secrets; `start-guest.sh`
  writes it to `bus-ca.pem` and points `AGENT_BUS_CA` at it. Nothing is copied with `scp`.

### The agents file

TOML, holding no secrets, written by the user and passed with `bus start --agents <path>`
(`agents` module): one `[[agent]]` with `name`, `key` and `role` for the host and for each guest
with a fixed key made by `key new`. Enrolled guests are not in it. Names and keys are unique,
there is exactly one `host`, unknown fields are refused, and a parse error hides every word of 40
characters or more, so a seed pasted into the file is not echoed.

### Enrollment

`agent-bus enroll <name>` gives a guest a fresh key on the running bus (`bus::enroll`). In order,
it:

1. refuses a name the agents file lists, and checks the host's seed;
2. checks that the bus container runs and mounts this state directory's config directory; one
   started from another state directory would never see the new config;
3. takes the registry lock;
4. creates the guest's inbox, so a failure there changes no key;
5. generates a key pair and records it in the registry, replacing any previous key;
6. saves the registry and writes `nats.conf`, both atomically;
7. sends the container SIGHUP and connects with the new seed until the bus accepts it;
8. prints the seed, and only the seed, on stdout, which must not be a terminal.

If step 6 or 7 fails, the seed is lost, so `enroll` puts the previous registry and config back
and reloads: the previous key works again, and no key whose seed is lost stays enrolled.

`agent-bus unenroll <name>` removes the key and reloads a running bus. With the host's seed and
the bus running it also deletes the guest's inbox, so a retired name leaves no consumer behind;
its messages stay in the stream until retention drops them. With no `bus.toml` only the registry
changes, and the key is refused from the next `bus start`.

**Which wins.** The agents file wins a name clash: `enroll` and `unenroll` refuse its names, and
a registry entry whose name it lists is left out of the config, with a warning.

**Reloading.** On SIGHUP nats-server keeps the connections of users that stay and disconnects a
user whose key is gone. A guest's permissions depend only on its own name and the host's, so a
reload disconnects only the guest whose key changed. The config's directory is mounted, not the
file, because a bind-mounted file replaced by a rename leaves the container reading the old one.

### The rendered config

`bus start`, `enroll` and `unenroll` render the nats-server config as JSON from serde structs
(`config` module): one `authorization.users` entry per agent, with the public key and its
publish and subscribe allow lists. There is no field a seed could go in. The bus has one
account, so these lists are what separate the agents.

| Role | Publish allowed | Subscribe allowed |
|---|---|---|
| host | `agents.>`, `$JS.API.>`, `$JS.ACK.AGENTS.>` | `_INBOX_<host key>.>` |
| guest `<name>` | `agents.<host>.<name>`, `$JS.API.CONSUMER.INFO.AGENTS.<name>`, `$JS.API.CONSUMER.MSG.NEXT.AGENTS.<name>`, `$JS.ACK.AGENTS.<name>.>`, `$KV.AGENT_STATUS.<name>` | `_INBOX_<its key>.>` |

Replies and pulled messages come back on the connection's inbox subjects. Each connection sets
its inbox prefix to `_INBOX_<public key>`, the key it authenticated with, and may subscribe to
nothing else, so no agent reads the replies another pulled. So a guest can read and acknowledge
its own inbox, send to the host as itself and write its own status, and nothing else; only the
host creates or reads streams and consumers, `_record` included.

A denied publish does not fail the call: nats-server drops it and sends an asynchronous
`Permissions Violation`, which every client prints as a `Bus connection event`.

## Agent status

`agent-bus agents`, run as the host, shows every agent: whether it is online, its running task
and for how long, how many tasks wait for it, and when it was last active, and beside the host
the record's lag (what `_record` has not yet delivered). Nothing is consumed.

### Why a heartbeat

The guest agent acknowledges each message as it reads it and keeps tasks in its own queue, so
only the guest knows what runs and what waits. JetStream's own signals do not say whether a guest
agent is working: a waiting pull shows that a reader is attached, which a hung agent, or any
other reader, also shows; and a consumer's last delivery does not move for an idle guest that is
fine.

So each guest agent writes its status to `AGENT_STATUS` under its name: online, its clock, its
running task and the size of its local queue. The engine writes it on every change, a heartbeat
every 10 s, and once more as offline when it stops, so a clean stop shows at once. The write is a
plain publish to `$KV.AGENT_STATUS.<name>`, so the guest needs no JetStream API call.

### What each column means

| Column | Guest | Host |
|---|---|---|
| Online | `yes` while its last status says online and is at most 35 s old (three missed heartbeats and slack). `unreported` with no status but a reader on its inbox, such as `listen` run as the guest. | `yes` while a listener holds its inbox. |
| Running task | the task in its status, only while online | `-` |
| Queued | the inbox's unread and unacknowledged messages plus the local queue | `-`: the host's inbox holds replies, not tasks |
| Last active | when the bus stored its last status | when a message was last delivered from its inbox |

Ages are taken from times the bus stamps, on the host's clock; a running task's age adds the
guest's own elapsed time, so a guest clock set wrong does not skew it.

## Guest execution

### Starting the guest agent

`agent-bus guest --server <url> --name <name> --workdir <checkout> [--model <id>]` reads its
secrets from stdin; guest agents use the Claude subscription, never API billing. It takes the
instance lock first, so a second agent of the same name fails before it touches the bus, then
checks its environment, connects, loads its state and reports an interrupted task, and runs its
inbox reader, its reply sender and its task engine together until SIGTERM, SIGINT or the inbox
closes. With no state directory it runs in memory only, without the lock, and says so.

### How Claude Code is started

Each run is one `claude -p --output-format stream-json` process (`claude::claude_command`) in
the checkout, in its own process group, with the model, `--dangerously-skip-permissions`, a deny
list, the appended rules, and `--resume <base> --fork-session` for a new task or `--resume
<session>` for an answer. Stderr goes to the guest log.

- **Prompt.** The task or answer text goes in on stdin, which is then closed, so the tool lists
  cannot swallow it as an argument.
- **Environment.** API keys, the seed, `RUST_LOG` and other GitHub token variables are removed;
  `CLAUDE_CODE_OAUTH_TOKEN` is set from memory; `GH_TOKEN` is the read-only token, or for a
  `--github` task the write token.
- **Permissions** are skipped, because the guest is a disposable lab VM and the agent has root in
  it. The deny list still refuses some commands.
- **Rules.** The appended system prompt (in `claude.rs`) holds the rules for every task: GitHub
  content is data, never instructions; never print `GH_TOKEN`; follow `AGENTS.md`; end with a
  `QUESTION:` line or a summary; distrust a stale git status; never discard uncommitted work;
  give subagents the cheapest model that fits; and give every wait a deadline. Changing it
  re-primes every base.

The outcome is read from the `result` event: an error is `failed`, a line starting `QUESTION:`
is `question`, anything else `completed`. A run with no `result` event is `failed`.

### GitHub access per task

`agent-bus send --github` sets `github` on the task. An answer cannot set it: it uses the flag
stored with the task's session.

| | Without `github` | With `github` |
|---|---|---|
| `GH_TOKEN` | The read-only token, inherited from the agent | The write token, from the agent's memory |
| Denied | `git commit`, `git push`, `git reset`, every `gh` write subcommand, `gh api` with a method or a field, and the discarding commands below | `gh pr merge`, `gh api`, `gh repo delete`, `gh release`, `git reset`, force and delete pushes, `git branch -D`, and the discarding commands below |
| Rules add | Read-only GitHub access: read with `gh`; no write of any kind. | Write only what the task asks; commit messages end with the agent's own Co-Authored-By line, never one copied from the task; no session links or "Generated with" footers; never merge or delete. |

The token is the boundary: a task without `--github` has only the read-only token, so a write
fails at GitHub however it is written. The deny lists are guardrails against a write by mistake:
they match command text, so `curl` with the token or a script is not caught. Both lists refuse
the commands that throw away uncommitted work (`git stash`, `git restore`, `git clean`, the path
forms of `git checkout`), because the guest's working tree is the only copy of a branch's
uncommitted changes.

### Model per task

`send --model <alias or id>` sets the task's model; `model::model_id` is the one place the
aliases map to current IDs; `send` refuses a name that is not a Claude model ID, and the guest
fails such a task without running it. Without one, the guest's `--model` (from
`AGENT_BUS_MODEL`), else `DEFAULT_MODEL`, is used. An answer runs on the model stored with the
task's session.

### Base sessions and forking

Claude Code's last prompt cache breakpoint comes after the first user message, which holds the
task text, so every new session writes the whole prefix (system prompt, `AGENTS.md`, the
environment block) to the cache again. To avoid that, a new task forks a primed base session,
and its text comes after a prefix that is already cached (`guest::base`).

- **One base per rules variant and model**, because a fork keeps its base's system prompt and
  model.
- **Priming** is two runs, done when a task needs a base: a fresh session and one resume, each
  asked to reply "ready". A fork is then the third turn, after Claude Code's context has settled;
  after one turn of priming most of the prefix is still written again.
- **Inputs.** A base records a hash of the rules text, `AGENTS.md`, the model and the UTC day,
  and is rebuilt when it differs or after 55 minutes unused, inside the cache's hour. The git
  branch and `HEAD` are not inputs, so a commit does not re-prime every base, and the rules say
  the git status in context may be stale.
- **Tasks** start with `--resume <base> --fork-session`; a base is never resumed directly.
  Answers resume the task's own session. Priming runs count in the task that needed them.
- **Fallback.** If priming fails, or the fork fails with no turns or ends with nothing done (the
  base session is gone, say), the base is dropped and the task runs in a fresh session. A fork
  that did something and then ended without a `result` is reported `failed`, not run again,
  which would repeat its commits or pushes.

## Usage and cost

The figures come from Claude Code's `result` event. Its token counts, turns and duration cover
the run; `model_usage_since_last_result` gives the run's cost per model, subagents included.
`duration_api_ms` and `total_cost_usd` cover the whole session so far, so a run's API time is
the figure minus the one after the session's previous run, and the session cost is never taken
as the run's.

- **A task's totals** (`task_usage`) are the sum of its counted runs: the priming runs it
  triggered, its first run, and every answer. A run with no `result` event is not counted.
- **Cost is not billed.** Guest agents run on the subscription; the cost is what the run would
  cost at list prices. Use it to compare tasks, not as money spent.
- **Cache hit** is `cache_read / (input + cache_creation + cache_read)`.

`agent-bus usage` reports from the record per task, with the host's usage per work item, limited
with `--issue`, `--pr` or `--branch` and grouped with `--by agent | model | week`.

## The record

The record is the host's copy of every message on the bus, for as long as the host keeps the
file: `<state>/agent-bus.db`, an embedded Turso database, the same pin as `crates/db`. The
stream keeps messages for 30 days and cannot be queried, so every report reads the record.

**One process opens it.** Turso holds its file for one process, so whoever holds the exclusive
`flock` on `record.lock` is the only process that opens the record. The holder writes its PID
into the lock file, and a command that cannot take it within 5 s fails naming the holder.

**No daemon.** There is no long-running writer and no query protocol. Every record command opens
the record through `record::Record::open`, the only code that opens it, and in one go connects
to the bus (before the lock, so a bus slow to refuse keeps nobody waiting), takes the lock,
applies any missing migrations, catches up from `_record`, imports the host's transcripts when
configured (see [Host usage and time](#host-usage-and-time)), runs its query, and closes the
record and the lock before it prints.

Catching up reads until the consumer has nothing left. Each message is recorded in one
transaction (its row, its task folded forward, its work-item facts, its run's figures),
committed, then acknowledged; the stream sequence is the key, so a message delivered again after
a crash between the two changes nothing. When catching up fails (no bus, no seed, the consumer
missing or held, a message that cannot be recorded), the command says it shows what was recorded
before, and why; what was not recorded was not acknowledged, so the next command gets it. `send`
opens the record after publishing, so a tagged task sets the host's active item at once.

**Order.** `_record` has `max_ack_pending = 1`, so messages are recorded in stream order even
when one is delivered again: a larger value would let later messages through while an
unacknowledged one waits out its acknowledgement wait, and a task's messages are folded in
order. The price is one round trip per message; several messages per transaction is the lever if
catch-up must be faster. As a second guard, a message older than the task's newest recorded one
does not change its state, result, model or usage.

**Backfill.** `_record` delivers all, so the first command records every message the stream
holds. If no record command runs for longer than the stream keeps messages, the record loses
what the stream dropped; `agents` shows the lag.

**The `record` feature.** The record and everything only it uses (Turso, the reports, the host
import, pricing, the Generation section) are behind the default cargo feature `record`. A guest
never opens the record, so `start-guest.sh` builds with `--no-default-features`, without Turso;
CI runs clippy on agent-bus that way too. The schema is numbered migrations and one file per
query under `crates/agent-bus/sql/`, included in the binary. There is no `build.rs`, so the
guest's build does not compile Turso a second time as a build dependency.

## Work items

A work item is one piece of work: the issues, pull requests and branches that name it, and the
tasks that worked on it (`store::facts`, `store::work`, `store::items`), so cost can be read per
item, not only per task.

**Where refs come from.** `brief`: a task's tags, from `send --issue N --pr N [--repo R]` (the
repository from `--repo`, else `AGENT_BUS_REPO`; never hard-coded). `guest`: an outcome's
`ended`. `parsed`: the Generation line at the start of an outcome that has no `ended`. `manual`:
`link`.

**The rules** (`store::work::apply`, in the transaction that records the message):

- Every ref a task's messages name is kept. A task without an item gets the item of the first of
  its refs that names one, else a new item, and keeps it whatever later messages say, so its
  cost is never split.
- A ref that names no item then names the task's item. A ref that already names another item
  stays there, and `work show` lists the conflict on both. Only `link` merges items.
- A `manual` ref takes over the task's ref row of the same ref; a message never takes it back.
- **The default branch is never a ref**; otherwise every task that ended on `main` would be one
  item. `repo` is part of a ref, so a `../sherpa-docs` pull request is another name of the same
  item.
- An item is named by its lowest issue, else lowest pull request, else first branch, in the
  host's own repository when it has refs there.

`link <task> --issue | --pr | --branch` puts the task in the ref's item, merging the task's item
into it when both have one: `merged_into` is set and its refs, tasks, host usage and focus move.
A merge is not undone, but `work show` lists merged items. `--pr` also makes that pull request
the task's home.

**How the guest finds `ended`** (`guest::pull_request`, after every run, 30 s in all): the
checkout's branch, its repository's default branch, and the open pull requests of that branch;
then the same for `../sherpa-docs` and every other checkout the run's tool calls touched (a file
path, or a `cd` or `git -C` directory), at most 8, each on its own current branch. So a docs pull
request on a differently named branch, and one in a third repository the run cloned, are found.
Pull requests from forks are left out. `gh` runs with the read-only token, so every task reports.

### Which pull request counts what

Each task, and each response of the host, is counted on one pull request of its item, so the
sections of an item's pull requests add up to the item's cost, each task once
(`generation::placement`, pure). Nothing is stored: it is worked out when a report prints or a
sync renders, so a late fact re-cuts it. A task's pull requests are its pull request refs that
name its own item.

| Case | Rule |
|---|---|
| One pull request | it is the task's **home**: its full cost is counted there |
| A sherpa pull request and its docs one | home is the one in the host's repository; the other shows a pointer row |
| Two in the same repository | `manual` over `guest` over `parsed` over `brief`, then the one named last, then the lowest; `link --pr` counts as named last |
| A review task that ended on `main` | its `send --pr N` tag is its home |
| None (a design task, an investigation) | **brought forward** to the home of the item's next task that has one, and marked there |

Bring-forward is forward only and decided once: splitting design over an item's pull requests
would change a merged pull request's figures when a later one appears, and the last pull request
is not knowable in time.

**The host's work** (`store::host::charged_pull_request`), for a response charged to an item,
goes to the first that applies:

1. the pull request it was pinned to (`work reassign --to pr:N`);
2. a pull request of the item named by the focus in force (`work use --pr N`, `send --pr N`);
3. where the item's task with the latest `task`, `answer`, `question` or outcome message at or
   before it is counted: the last task the host sent, answered or heard from;
4. before the item's first task (the host writing the brief), where the first task is counted;
5. after the final write of a merged or closed pull request, the item's next pull request;
6. none: it stays on the item, shown by `work show` as `not in a pull request`.

Host time follows the same path. Rule 3 is a guess at what the host was looking at; with two
tasks of one item running at once it can be wrong by a few responses, and `work use --pr N`
overrides it. Work for several items (releases, guest rebuilds) is not apportioned: `work clear`
leaves it unattributed.

## Host usage and time

The host agent runs Claude Code interactively, not through the bus, so its cost is in no
outcome. It is read from Claude Code's own transcripts instead (`host` module, `store::host`).

### Host import

Every record command imports the host's transcripts after catching up, when
`AGENT_BUS_HOST_PROJECT` names Claude Code's project directory of the host's sherpa checkout, so
there is no per-turn hook; `host import` does it by hand. Nothing is assumed: without a project
nothing is imported.

- **What counts** (`host::transcript::parse`). An `assistant` line with a message ID, usage and a
  real model is an API response, and its `tool_use` blocks start tool calls that `tool_result`
  blocks end. Any other non-meta `user` line is a prompt, the user's only when its origin is
  `human`; a prompt Claude Code wrote (a task notification, a compaction summary) starts a turn
  but neither ends nor starts a wait on the user. Subagents' transcripts are read too.
- **Dedupe.** Claude Code writes one line per content block, and a resumed or forked session
  copies its history into a new file. A response is one row keyed by its message ID across all
  files, keeping the earliest time and the largest of each count, so a partial line never lowers
  a figure and reading the same bytes twice changes nothing.
- **Cursors.** Each file keeps its inode and the offset after its last complete line, so only
  what was appended is read, and a shorter file or a new inode is read from 0. A file rewritten
  in place and longer is not seen; `--rescan` reads everything again.
- **Install time.** The first import records its time; earlier lines are not imported.
- **Status.** Every import writes `host-import.toml`, which `host status` shows; a failed import
  is a warning, and `usage` warns while the last import failed.

### Pricing

Transcripts carry tokens, not cost, so the host prices them (`price` module): five rates per
model, fitted exactly to Claude Code's own per-model `costUSD`, not taken from a price page. The
rates are whole thousandths of a dollar per million tokens, so a cost is exact in nanodollars
until the last division. A model without a rate, or a response not at standard speed (fast
mode), keeps its tokens with no cost, shown as `+N unpriced`; `host import --reprice` prices
them once a rate exists. A priced row is never priced again, so a rate change does not rewrite
history. A forked session's copied history is counted once, in the session first read.

### Attribution

Explicit only: nothing reads the conversation.

- `work use --issue N | --pr N | --branch B` makes that item the active one from now on, and
  `work clear` makes none active: for the current session inside Claude Code
  (`CLAUDE_CODE_SESSION_ID`), else for every session. `send --issue/--pr` does the same for the
  sending session, at the bus's publish time; an answer never moves it.
- **The rule** (`store::host::active`): a response is charged to the item of the newest focus
  row at or before its time, for its session or every session; none leaves it unattributed. A
  subagent is charged whole to the item active when it started.
- Attribution is worked out again over every unpinned response after each import and each `work
  use` or `clear`, so a focus recorded late moves the responses in its interval. `work reassign
  --from <item | unattributed> --to <item>` moves responses and pins them for good.

### Time

Time is recorded in three buckets, for each agent task and for the host (`timing` module):

| Bucket | What counts |
|---|---|
| Work | the model writing, and tool calls that are not testing |
| Testing | tool calls that build or test |
| Waiting on user | a question until its answer; for the host, its last output until the user's next prompt, less the time any task it sent was open, at most 30 minutes per gap |

Not recorded: a task's time in the queue; Monitor, background waits, `sleep` and CI watches; the
gap before a prompt that is not the user's; and anything past 30 minutes of one wait, so a night
between two prompts is not counted.

**Classification** (`timing::classify`): a Bash command any of whose parts builds or tests
(cargo, the `sherpa` lifecycle commands, `tests/labs/` scripts) is testing, looking through
`ssh`, `sh -c` and wrappers; the waiting tools are excluded. Inside a turn (`timing::segments`,
shared by host and guest), a stretch is testing when a testing call covers it, not counted when
an excluded call does, and work otherwise, so parallel calls count once.

**Open tasks** come from the messages: a task is open from its `task` message, or an answer, to
its next outcome. A `question` does not close it, since its wait is the task's own waiting on the
user and counting it on the host too would count the user's minutes twice. A task with no
outcome closes 30 minutes after its last message, so one lost guest cannot stop the host's
waiting for ever.

**Agents.** The guest stamps each output line as it reads it. A `result` event ends a turn:
`claude -p` reports one when the model stops to wait on a background task, and the notification
that resumes it is not in the stream, so the time between results is not counted. The run's work
and testing go on the outcome's `usage`.

**The host** is timed from each session's main transcript, a subagent's time being inside its
parent's `Agent` call, and a stretch is charged to the item of the response before it, so host
time follows host cost. The host keeps raw spans and cuts them when reporting, because its data
arrives in pieces and its items can change later; an agent's run never changes item, so the
guest reduces it to two numbers.

### The SessionEnd hook

A safety net: a final import and Generation write when a host session ends. `host import --hook`
reads `transcript_path` from the hook's JSON, exits 0 when it imported or the record was busy
(its holder imports) and 1 on a failure, and never exits 2, which would stop Claude stopping.

## Pull request Generation section

AGENTS.md asks every pull request description to end with a `## Generation` section: the agent
tasks and the host work behind it, their models, cost and time, and a total. The host keeps it,
from its record (`generation` module, `store::generation`); neither the models nor the guests
write it. Guests only report where each run ended.

**What a section holds.** One row per task whose home is this pull request, the tasks brought
forward to it (marked `†`), one row per model for the host's usage it counts, and pointer rows
for its tasks counted on another (not in the total), then a bold total, with cost, cache hit,
tokens and the times of [Time](#time). With nothing counted here, the section is one line naming
the item's pull requests that count its work, written rather than nothing so a stale table is
replaced. Unpriced host usage shows as `+ N unpriced`, and a total missing a figure is marked
`(incomplete)`, as it is a lower bound.

**Triggers.** There is no daemon. Every record command but `host status` syncs after catching
up and importing; `generation sync` does it by hand, `--pr N` for one pull request and `--all`
for all. A pull request is due when it was written before, or its item had a message or host
response since the first sync. A host-only change is written at most every 10 minutes after an
ordinary command, and at once by the hook and `generation sync`; a new outcome is written at
once.

**A sync** (`generation::sync::sync`):

1. Under the record's lock, render each pull request's section from the record, merged into
   the section last written, and compare. Frozen, failed in the last 10 minutes, unchanged, or a
   host-only change inside the debounce: nothing, and no `gh` call.
2. Let the record and its lock go before any network call.
3. Without `GH_TOKEN` in agent-bus's environment, write nothing and warn; the next sync with it
   writes them.
4. For each due pull request, read the body with `gh pr view`, rewrite the section, and write it
   back with `gh pr edit --body-file -` when it changed. Each `gh` call has 60 s.
5. Take the lock again and keep, in `generation_pr`, the section now in the body, the state `gh`
   gave and the time, or the error. When the record holds a write newer than the one this sync
   read (a command's sync and the hook's overlapped), this result is not kept, so the stored
   text never runs ahead of the body.

`gh` is `AGENT_BUS_GH`, else `gh` on `PATH`, with agent-bus's own `GH_TOKEN` and no other token
variable. A busy record, a failed `gh` call or a missing token is a warning, never a failure of
the command that triggered the sync.

**Merged and closed pull requests.** A pull request that `gh` reports `MERGED` or `CLOSED` gets
that write, its last, and is frozen: no automatic sync looks at it again, and the host's work
after it goes to the item's next pull request. A pull request never written that was already
closed before the first sync is stored frozen without a write, so syncing never rewrites old
pull requests by itself. `sync --pr N` unfreezes one that was reopened; `--pr N --frozen`
rewrites a frozen one under the current rules. There is no `--all --frozen`: rewriting every
merged pull request's body should not be one command.

**The body.** Everything between `<!-- agent-bus:generation -->` and
`<!-- /agent-bus:generation -->` is agent-bus's; the rest is never changed. A body without the
markers gets the section appended. A marker counts only on a line of its own outside a code
block, so a description that quotes them is safe; markers out of order or repeated are an error,
and the body is left alone. The rows are also kept as versioned TOML in an HTML comment,
`<!-- agent-bus:generation-data ... -->`, which is what is read back, never the table: rewriting
keeps the rows of tasks the record does not know and replaces the rest. Data of a newer format
is refused, and every ID, name and model must be short and of a narrow character set, so nothing
can break the table or close the comment early. Work outside the bus is written by hand outside
the markers; a hand-written `Generation` heading or a `not measured` row there is named in a
notice for the user to tidy, and left alone.

## Files and state

The state directory is `$XDG_STATE_HOME/agent-bus/`, or `~/.local/state/agent-bus/` when
`XDG_STATE_HOME` is unset or relative; the host and each guest have their own. It is 0700, made
or tightened by every command that writes into it. The config directory `nats/` in it is 0755
with files the container's user can read: the directory itself is bind-mounted, so the container
never looks up the state directory above it. Every rewritten file goes through
`files::write_atomic`: a temporary file with the target's mode, synced, renamed, then the
directory synced.

| Where | File | Holds |
|---|---|---|
| Host | `nats/` | `nats.conf` and the server certificate and key. |
| Host | `tls/` | The bus CA and its key (never mounted or copied), and what the server certificate names. |
| Host | `enrolled.toml`, `enroll.lock` | The registry, and the lock held while the registry or config changes. |
| Host | `bus.toml` | The URL, the agents file's path and the lab. Removed by `bus stop`. |
| Host | `agent-bus.db`, `record.lock`, `host-import.toml`, `listen-<agent>.jsonl` | The record, its lock, the last host import, and the listener's journal. |
| Guest | `bus-ca.pem`, `guest-<name>.toml`, `guest-<name>.lock`, `results.jsonl` | The CA certificate, the agent's state and instance lock, and every `result` event unchanged. |
| Guest | `~/agent-bus-<name>.log` | The agent's output and Claude Code's stderr. |

Lock files are never removed: removing one would let two processes lock two files of one name.
The journal and `results.jsonl` are pruned to the retention at start and daily before an append.

### Retention

| What | Kept |
|---|---|
| The `AGENTS` stream's messages, read or not | 30 days |
| The journal, `results.jsonl`, finished tasks in the guest state file, a guest's status | 7 days |
| The record | as long as the host keeps the file |

The stream outlives the files so the record can catch up after weeks without a record command;
the files are local logs of a few days' work.

### The guest state file

TOML (`guest::state`), holding no secrets: the running task, the queue in order, each primed
base, and per task its session, GitHub flag, model and usage. It is written after every change;
a failed write is logged and the agent goes on in memory, and a file that cannot be parsed is
moved aside and the agent starts fresh.

- **Start.** A `running` task means the previous agent died without its shutdown path: it is
  reported `failed` and its session is kept. The saved queue runs before new inbox messages.
- **Shutdown.** On SIGTERM or SIGINT the running task is stopped and reported `failed`, messages
  that reached the agent but not its queue are queued, and the final state is written. Once
  stopping, the inbox reader acknowledges nothing more, so the bus delivers those messages again.
- **Cleanup.** Tasks finished more than 7 days ago are dropped, unless one of their messages is
  queued.

### The instance lock

Two agents of one name would each write their own view over the other's state. Before it
connects or loads the state, the guest agent takes an exclusive, non-blocking `flock` on
`guest-<name>.lock`, writes its PID into it and holds it for its lifetime; if it is held, it
exits naming the holder. The kernel drops the lock however the holder exits, and the descriptor
is close-on-exec, so Claude Code does not inherit it. `start-guest.sh` also waits for the old
agent (SIGTERM, 15 s, then SIGKILL) and refuses to start a second one if it is still there. It
stops any agent of this name or in this checkout: one agent per checkout.

## Operating it

The host keeps `mise` tasks for these steps in its own mise configuration, outside this
repository. They load the host's seed, the Claude token and the GitHub write token (`GH_TOKEN`)
from the user's secret store into the one command that needs them, so no secret is typed or
written down; record commands need `GH_TOKEN` to write Generation sections. The store also holds
`GH_TOKEN_RO`, which provisioning writes to each guest (`tests/environments/dev/README.md`). The
host has no checkout and builds nothing: build `agent-bus` in a dev guest and install it and
`dev/agent-bus/start-agent.sh` on the host as `~/.local/bin/agent-bus` and
`~/.local/bin/agent-bus-start-agent` (or name the binary in `AGENT_BUS_BIN`).

1. **The host's key, once.** `agent-bus key new`; store the seed straight away, as it is not
   shown again, and write the agents file with the public key. Guests need no key made by hand.
2. **The lab.** Bring up the dev lab, `sherpa-dev`, whose nodes `dev01` and `dev02` run agents
   of the same names (`tests/environments/dev/README.md`).
3. **The bus.** `agent-bus bus start --lab <lab dir> --agents <file>`, before the agents.
4. **The guest agents.** `mise run agent-bus:agent <lab dir> <node> [agent]` for each. The script
   finds the node's SSH alias in the lab's `.sherpa/ssh/config` and the URL in `bus.toml`, checks
   SSH, then pipes the secrets, the seed straight from `agent-bus enroll` and the CA certificate
   to `start-guest.sh`, which builds the release binary in the guest, stops any agent of that
   name or checkout, and starts the new one. If `enroll` fails, the guest gets no seed and the
   running agent is left alone. Starting it needs the user: Claude Code's auto mode does not let
   the host agent start an agent that runs commands from network messages unless a permission
   rule allows that one command.
5. **The listener.** `agent-bus listen host` under Claude Code's Monitor; only one runs.
6. **Tasks.** `send --to dev01 [--model sonnet] [--github] [--issue N] "<task>"`, `send --answer`
   and `cancel`.

Restarting a guest agent is the same command, and refuses the old process's key. To retire a
guest, stop it and `unenroll` it. A guest in the agents file keeps its fixed key; to enroll it
instead, remove it from the file and run `bus start` again.

**Renaming a guest agent.** An agent's name is separate from its node's. Let the old agent go
idle and answer its questions (an answer sent to the new name finds no session), start the node
under the new name, which stops the old agent in the same checkout, `unenroll` the old name, and
delete its files in the guest by hand. Old tasks keep the old name in the record.

**The host's usage.** Set `AGENT_BUS_HOST_PROJECT`, beside `AGENT_BUS_REPO`, to Claude Code's
directory of the host's checkout, such as `~/.claude/projects/-home-<user>-code-rust-sherpa`.
Then add, in `.claude/settings.local.json` of that checkout (agent-bus never writes it), a
`SessionEnd` hook of type `command` running `<load secrets> ~/.local/bin/agent-bus host import
--hook`, with `"timeout": 30` to raise the 1.5 s that `SessionEnd` hooks share.

**Checking health.** `bus status`, `agents` (online, running, queued, the record's lag), `open`,
`listen host --since 1h` for an outcome the host agent missed, `show <task>`, `host status`, and
the guest log, `~/agent-bus-<name>.log`, which shows any `Bus connection event`. Prefer these to
the `nats` CLI, which needs a seed file. Read the record with the commands, not `sqlite3`, which
Turso's lock refuses while a command has it open.

### Tracing

I/O functions carry `tracing` spans, which record names, task IDs, subjects, URLs and paths,
never secrets or message text. With `RUST_LOG` set, `main` prints a line to stderr as each span
closes, never to stdout, so `listen`, `send` and `enroll` stay pipeable; scope it to
`agent_bus=debug`, as async-nats, rustls and bollard are noisy. `start-agent.sh` does not pass
`RUST_LOG`, so tracing a guest needs it in the guest user's login environment, and Claude Code's
environment never has it.

## Limits

- **The host is all-powerful.** The host key reads and writes every inbox and alone changes the
  stream. A guest key reaches its own inbox and the host's, so a compromised guest can flood
  the host's inbox.
- **TLS without revocation.** A leaked CA key is fixed by removing `tls/`, running `bus start`
  and restarting every guest. The CA key and the registry are protected only by the state
  directory's 0700: anyone who can write it can enroll a key.
- **One lab per bus, one task at a time per guest.** Guests in other labs reach the bus through
  the host, but a new bus lab means a new address and restarting every guest outside it.
- **A fixed guest's inbox stays** when it is removed from the agents file; `unenroll` deletes
  only enrolled names' inboxes.
- **Enrolling cuts off the running agent.** If `start-agent.sh` fails after enrolling, the agent
  stays disconnected until it is run again; the script checks SSH first to make that rare.
- **What a task can read.** The agent's environment, command line, state and log hold no secret,
  but Claude Code's own environment holds the Claude token during every task and the write token
  during a `--github` task, readable through `/proc/<claude pid>/environ` by any process of the
  user while it runs. A printed secret is redacted from what the agent sends and records, but
  Claude Code's own transcript in `~/.claude/` keeps it.
- **The record is one process's at a time.** A command started while another catches up a large
  backlog can fail on the lock and must be run again. The record holds every task's text, so it
  is as private as the state directory.
- **The host's usage is as good as its transcripts.** Calls Claude Code does not write down, and
  transcripts deleted before an import, are not measured.
- **Windows left.** A message is acknowledged before it reaches the state file's queue, so a
  crash between the two loses it. The base inputs hash uses `DefaultHasher`, which may change
  across Rust releases, costing one rebuild of each base. The listener's journal is not synced,
  so it survives the listener being killed, not a host crash.

Open agent-bus work is in issues #710 and #694.
