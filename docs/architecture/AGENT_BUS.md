# Agent Bus

The agent bus is a message channel between the AI coding agent the user works with on the host
(the host agent) and agents running in lab guests (guest agents). The host agent hands a guest
agent a whole task, and gets back its progress, its questions for the user, and its outcome.

The code is the `agent-bus` crate, `crates/agent-bus`. The research behind it (what was tried and
measured) is in [`../investigations/agent-bus.md`](../investigations/agent-bus.md). Open work is
tracked in issue #494.

## Purpose and scope

- **A development tool, not released.** `agent-bus` has `publish = false`, is not installed by
  the install scripts, and is not part of `sherpad` or the `sherpa` CLI. It has no user-facing
  documentation in `../sherpa-docs`.
- **Two roles.** The host agent sends tasks, answers and cancels, and reads the replies. A guest
  agent (`agent-bus guest`), running in a lab guest such as the dev server
  (`tests/environments/dev`), takes each task to Claude Code in its checkout and replies.
- **What it replaces.** Before the bus, the host agent did the work itself over SSH, one command
  at a time, and every build log and test run landed in its context. With the bus the guest
  agent does the work, and the host agent sees only the progress lines and the outcome.
- **What it is not.** It is not A2A (#439). The message kinds are named after A2A's task states,
  but the protocol is plain JSON on NATS subjects.

## Components

| Part | What it is | Code |
|---|---|---|
| Bus container | `sherpa-agent-bus`, image `nats:2.15-alpine` pinned by digest (nats-server 2.15.2), run with JetStream on the host's network. It listens on one address, port 4222: with `--lab`, the lab's management gateway (`ipv4_gateway` in `.sherpa/lab-info.toml`), which is also each guest's default gateway. Restart policy `unless-stopped`. | `src/bus.rs` |
| Store | The Docker volume `sherpa-agent-bus-data`, mounted at `/data` (`--store_dir`). It outlives the container. | `src/bus.rs` |
| Server config | `nats.conf`, rendered from the agents file and the registry of enrolled guests. Its directory is bind-mounted read-only at `/etc/agent-bus/` (`-c /etc/agent-bus/nats.conf`), and SIGHUP reloads it. | `src/config.rs` |
| TLS | The bus's own private CA (not the Sherpa lab CA), made once by `bus start`, and the server certificate it signs for the listen address, in the same mounted directory as the config. The server takes TLS only, handshake first. See [TLS](#tls). | `src/tls.rs` |
| Registry | `enrolled.toml`: the guests `enroll` added, with their public keys. Host-managed, never edited by hand. | `src/registry.rs` |
| Stream | One JetStream stream, `AGENTS`, on `agents.>`, keeping messages for 7 days (`max_age`). | `src/channel.rs` |
| Status bucket | A JetStream key-value bucket, `AGENT_STATUS` (stream `KV_AGENT_STATUS`), one key per guest, history 1, kept 7 days. Each guest agent's heartbeat. See [Agent status](#agent-status). | `src/channel.rs`, `src/status.rs` |
| Inboxes | One durable pull consumer per agent, named after it, filtered on `agents.<name>.>` (one subject per sender), with `max_waiting = 1`. A message waits until its agent reads and acknowledges it, and a restarted reader goes on where it stopped. | `src/channel.rs` |
| Host commands | `key new`, `bus start/stop/status`, `enroll`, `unenroll`, `send`, `cancel`, `listen`, `usage`, `show`, `agents`. | `src/main.rs`, `src/bus.rs`, `src/listen.rs`, `src/table.rs` |
| Starting a guest | `dev/agent-bus/start-agent.sh` on the host enrolls a node's agent and pipes its seed and the bus's CA certificate over SSH to `dev/agent-bus/start-guest.sh` in the guest. | `dev/agent-bus/` |
| Guest agent | `agent-bus guest`: reads its inbox, queues tasks, runs them one at a time through Claude Code, and replies. | `src/guest/` |
| Claude Code | `claude -p` with `stream-json` output, one process (and process group) per run, in the guest's checkout. | `src/guest/claude.rs` |

The flowchart shows the parts and how they connect. The only path between the host and a lab
guest is NATS over TLS on the lab's management bridge; nothing goes over SSH once the guest agent
runs.

```mermaid
flowchart LR
    subgraph host["Host"]
        hostagent["Host agent<br/>Claude Code"]
        cmds["agent-bus send, cancel,<br/>listen, usage, show"]
        start["agent-bus bus start"]
        enroll["agent-bus enroll, unenroll"]
        registry["enrolled.toml<br/>enrolled guests' public keys"]
        conf["nats.conf<br/>public keys, permissions"]
        subgraph ctr["container sherpa-agent-bus, host network"]
            nats["nats-server + JetStream<br/>stream AGENTS<br/>on lab gateway:4222"]
        end
        vol[("volume<br/>sherpa-agent-bus-data")]
    end
    subgraph guest["Lab guest, e.g. dev server"]
        gagent["agent-bus guest"]
        claude["claude -p<br/>stream-json"]
        checkout["checkout"]
    end
    hostagent --> cmds
    cmds <-->|"NATS over TLS, NKey auth"| nats
    start -->|"Docker API"| ctr
    start --> conf
    enroll --> registry
    registry --> conf
    enroll --> conf
    enroll -->|"SIGHUP, Docker API"| ctr
    conf -.->|"read-only bind mount<br/>of its directory"| nats
    nats --- vol
    gagent <-->|"NATS over TLS, NKey auth<br/>to default gateway"| nats
    gagent -->|"prompt on stdin"| claude
    claude -->|"events on stdout"| gagent
    claude --> checkout
```

### Starting the bus

`agent-bus bus start --lab <lab dir> --agents <file>`, with the host's seed in `AGENT_BUS_SEED`.
Exactly one of `--lab` and `--listen <ipv4>` is required (`src/main.rs`, `src/state.rs`).

1. With `--lab`, read `.sherpa/lab-info.toml` in the lab directory (`shared::util::LabFolder`);
   its `ipv4_gateway` is the listen address, and its `id` and `name` are recorded. A missing
   file, or one without a gateway, is an error naming the path. A lab folder in the layout from
   before `.sherpa/` (a loose `lab-info.toml` and no `.sherpa/`) is refused, with a message to
   run any sherpa 0.5.15 command there, which moves its files; there is no fallback to the old
   names (see `LAB_FOLDER.md`).
2. Load and validate the agents file, and check that the seed's public key is the host's key.
   Under the registry lock, load the registry and merge it with the agents file (see
   [Enrollment](#enrollment)).
3. Write `nats/nats.conf` (see [Files and state](#files-and-state)). Then make the bus CA if
   there is none, and issue the server certificate for the listen address if it needs it (see
   [TLS](#tls)). A listen address of `0.0.0.0` is refused: the certificate names the one address
   clients connect to.
4. Pull the pinned image if Docker does not have it, then replace the container: stop and remove
   any existing `sherpa-agent-bus`, create it, start it. An existing container is never just
   restarted, because the config may have changed. The messages are in the volume, so replacing
   the container loses nothing.
   A `nats.conf` left in the state directory by a version that mounted the file itself is
   removed.
5. Connect as the host over TLS (20 attempts, 500 ms apart), create or check the `AGENTS` stream, and
   bring each agent's inbox, the host's and every enrolled guest's included, to the wanted
   consumer config.
6. Record the URL, the agents file's absolute path (and the lab) in `bus.toml`, and print one
   `Inbox ready: <agent> (<change>)` line per agent.

Each inbox is reported as `created`, `updated`, `recreated from sequence N` or `unchanged`
(`Channel::ensure_inbox`). A missing consumer is created. A different filter subject is updated
in place. NATS cannot change `max_waiting` on an existing consumer, so a consumer that differs in
it is deleted and created again with `DeliverPolicy::ByStartSequence` at the old consumer's ack
floor stream sequence + 1, read before the delete: acknowledged messages stay gone, delivered but
unacknowledged and unread messages are delivered again. If the create fails after the delete,
the error names the sequence to create it from.

An inbox made before messages carried their sender in the subject is filtered on
`agents.<name>`; `bus start` updates it to `agents.<name>.>` in place. Messages stored under the
old subject are not delivered after that, and `usage` and `show` no longer see them, so let the
inboxes drain (and run `listen` once) before upgrading. Guests must run the same version as the
host: an older guest publishes to `agents.<host>`, which it may no longer do.

`bus stop` stops and removes the container and removes `bus.toml`; the volume and its messages
stay. A container that is stopped already or gone is no error (`bus::remove`), so `bus stop`
run again, or after the container was removed by hand, still removes `bus.toml`. `bus status` prints the container's state, the recorded URL and the lab.

Adding a guest means starting it: `start-agent.sh` enrolls it on the running bus (see
[Enrollment](#enrollment)). `bus start` is needed again only when the agents file changes.

### Finding the bus

`enroll` and `unenroll` use the whole record: the URL, and the agents file `bus start` was
given. A record without the agents file, written before enrollment, makes them fail and say to
run `bus start` again. With no record at all (the bus was stopped), `unenroll` still works: see
[Enrollment](#enrollment).

`send`, `cancel`, `listen`, `usage` and `show` take the first of: `--server <url>`, the
environment variable `AGENT_BUS_URL`, the URL in `bus.toml` (`state::find_server`). With none,
they fail and say to run `bus start` or set one. The guest agent runs in the lab guest, where
there is no `bus.toml`: it takes `--server` (or `AGENT_BUS_URL`), which `dev/agent-bus/start-guest.sh`
builds from the guest's default gateway for a guest in the bus's lab. A guest in any other lab
is given `AGENT_BUS_URL`, the bus's URL, since its own default gateway has no bus on it.

Every command that connects trusts one CA certificate: the file named by `AGENT_BUS_CA`, else
`tls/ca.pem` in the host's state directory, which `bus start` made (`tls::client_ca`). The
commands inside `bus start` and `enroll` always use their own state directory's. A missing file
fails before connecting, naming the path and `AGENT_BUS_CA`. The guest agent is given
`AGENT_BUS_CA` by `start-guest.sh` (see [TLS](#tls)).

### One reader per inbox

Readers of a durable consumer share its messages, so a second reader on one inbox would silently
take some of them. Inboxes are created with `max_waiting = 1`, and `Channel::inbox` refuses to
open an inbox whose consumer already has a pull waiting (`num_waiting > 0`), naming the likely
cause (an earlier `agent-bus listen <agent>` still running). A killed reader leaves no claim
behind.

## Messages

Every message is one JSON object, published to `agents.<to>.<from>` and stored by JetStream
before `send` returns (`src/message.rs`, `Channel::send`). A guest may publish only under its own
name (see [The rendered config](#the-rendered-config)), and a reader drops, logs and
acknowledges a message whose `to` and `from` are not its subject's (`channel::sent_by`;
`usage` and `show` skip it too), so a guest cannot pose as another agent.

| Field | Type | Meaning |
|---|---|---|
| `task` | string | The task ID: 8 hex digits, made by `send` for a new task. Every message of a task carries it. |
| `from`, `to` | string | Agent names: 1 to 32 lowercase letters, digits and hyphens, so a name is one subject token. |
| `kind` | string | One of the kinds below, in snake case. |
| `text` | string | The task, the answer, the progress line or the outcome text. Empty for `cancel`. |
| `usage` | object, optional | What this run of Claude Code used. Only on outcome messages that had a `result` event. |
| `task_usage` | object, optional | What the task has used so far, this run included. Sent with `usage`. |
| `github` | bool, optional | Whether the task may use GitHub. Only `task` sets it. Left out of the JSON when false; a missing field reads as false. |
| `model` | string, optional | On a `task`: the model to run on, an alias (`opus`, `sonnet`, `haiku`) or a full ID such as `claude-sonnet-5-5`. Without it the guest's own default. Ignored on an `answer`. On the guest's replies for a run: the full ID it ran on. Left out of the JSON when unset. |

| Kind | Direction | Meaning |
|---|---|---|
| `task` | host to guest | Work to do. |
| `answer` | host to guest | The answer to a question; resumes the task's Claude session. |
| `cancel` | host to guest | Stop the task named in `task`, running or queued. |
| `progress` | guest to host | A text Claude said, or a tool call (`<tool>: <command, file_path, pattern or path>`), cut to 300 characters. |
| `question` | guest to host | The run's result has a line starting `QUESTION:`. |
| `completed` | guest to host | The run ended without an error and without a question. |
| `failed` | guest to host | The run ended with an error, Claude Code could not run, the guest agent stopped or restarted during the task, or a cancel named a task that is neither running nor queued. |
| `canceled` | guest to host | The task was canceled; the text holds the last progress. |

`usage` has `input`, `output`, `cache_creation`, `cache_read`, `num_turns`, `duration_ms`,
`duration_api_ms`, `cost_usd`, `session_cost_usd`, and `models`: one entry per model under Claude
Code's own field names (`model`, `inputTokens`, `outputTokens`, `cacheCreationInputTokens`,
`cacheReadInputTokens`, `costUSD`, `canonicalModel`, `costBasis`), left out when empty.
`task_usage` has `runs`, `cost_usd`, the four token counts, `num_turns`, `duration_ms` and
`duration_api_ms`. Their meaning is in [Usage and cost](#usage-and-cost).

An outcome message from a guest:

```json
{"task":"1a2b3c4d","from":"dev","to":"host","kind":"completed","text":"Clippy is clean; 9 of 9 tests pass.",
 "usage":{"input":4,"output":120,"cache_creation":37,"cache_read":22538,"num_turns":3,"duration_ms":9100,
          "duration_api_ms":7400,"cost_usd":0.0031,"session_cost_usd":0.0031},
 "task_usage":{"runs":1,"cost_usd":0.0031,"input":4,"output":120,"cache_creation":37,"cache_read":22538,
               "num_turns":3,"duration_ms":9100,"duration_api_ms":7400}}
```

### A task's lifecycle

The state diagram shows a task as the host sees it. The guest agent keeps no "waiting for an
answer" state: an `answer` is accepted for any task whose Claude session it knows, so an answer
after `completed`, `failed` or `canceled` also resumes the session. An answer for a task without
a session fails with "No session for task ...: an answer must follow a question". A cancel for a
task that is neither running nor queued gets a `failed` reply and changes nothing.

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

"Queued" covers a message in the inbox and one in the guest agent's local queue. "Canceled while
queued" replies "Canceled before it started", with no usage.

## Flows

### A task with progress and a result

The host sends a task, the guest agent runs it through Claude Code, and each line of Claude
Code's output that means something to the host becomes a message. Claude's last text is also
its result, so each text is held until the next update shows whether it was the last; the final
text is not sent twice (`stream::repeats_result`).

```mermaid
sequenceDiagram
    participant H as Host agent
    participant B as Bus
    participant G as Guest agent
    participant C as Claude Code
    H->>B: send --to dev "task" (kind task)
    Note over H: prints the task ID
    B->>G: delivered from inbox dev
    G->>B: ack
    G->>C: claude -p --resume base --fork-session, task on stdin
    C-->>G: system event: session_id
    Note over G: session saved in the state file
    C-->>G: assistant: text, tool_use
    G->>B: progress
    B->>H: listen host prints one JSON line
    C-->>G: result event
    G->>B: completed, with usage and task_usage
    B->>H: listen host prints one JSON line
```

`agent-bus listen host`, run under Claude Code's Monitor on the host, appends each message to
its journal, then prints it as one JSON line, then acknowledges it, so each becomes a
notification for the host agent (`src/listen.rs`). A message the bus has delivered and the
listener acknowledged is never delivered again, so if the Monitor was gone when it was printed,
the journal is where it is left: `agent-bus listen host --since 30m` prints the journal's
messages from the last 30 minutes (units `s`, `m`, `h`, `d`), then listens live. A message that
cannot be journaled is neither printed nor acknowledged, and the listener exits; the bus delivers
it to the next listener. The guest
agent acknowledges each inbox message as soon as it reads it, before running it: a task runs far
longer than the bus waits for an acknowledgement, and must not be delivered twice.

A reader (the guest agent, and `listen`) outlives a short bus outage, such as `bus start` run
again. async-nats reconnects by itself, and its pull stream ends only after a deleted consumer,
a push consumer, or a pull over the consumer's limits. Those end the reader with an error. Any
other stream error (a missed heartbeat, a pull that could not be sent, no responders) is logged
as `Reading the inbox: ...; still reading`, and the reader goes on and pulls again once
reconnected (`channel::recoverable`).

### A question and its answer

A result with a line starting `QUESTION:` makes the outcome a `question`. The host agent asks the
user and sends the answer; the guest agent resumes the task's own session with it, with the
GitHub flag stored for that session.

```mermaid
sequenceDiagram
    participant H as Host agent
    participant B as Bus
    participant G as Guest agent
    participant C as Claude Code
    C-->>G: result: "... QUESTION: which crate first?"
    G->>B: question, with usage and task_usage
    B->>H: question
    Note over H: asks the user
    H->>B: send --to dev --answer task "agent-bus"
    B->>G: answer
    Note over G: session and GitHub flag of the task, from its state
    G->>C: claude -p --resume session, answer on stdin
    C-->>G: result event
    G->>B: completed, task_usage counts both runs
```

### Cancelling a running task

While a task runs, the guest agent reads its inbox alongside Claude Code's output. Tasks and
answers that arrive wait in the local queue; a cancel for the running task is acted on at once.
Claude Code runs in its own process group, so the signals reach every command it started (cargo,
bash, sleep).

```mermaid
sequenceDiagram
    participant H as Host agent
    participant B as Bus
    participant G as Guest agent
    participant C as Claude Code process group
    H->>B: cancel --to dev task
    B->>G: cancel
    G->>C: SIGTERM to the group
    alt the group leader exits within 2 s
        C-->>G: exited
    else KILL_GRACE runs out
        Note over G: "Claude Code ignored SIGTERM"
    end
    G->>C: SIGKILL to the group, for anything left
    G->>B: canceled: last progress, usage if a result event arrived
    Note over G: the session is kept, the queue goes on
```

`canceled` carries `usage` and `task_usage` only if Claude Code's `result` event arrived before
the cancel; otherwise the text says the run's cost is unknown and the run is not counted. A
cancel for a queued task removes it from the queue and replies `canceled`. `Group` in
`src/guest/claude.rs` also sends SIGKILL to the group when it is dropped while still armed, so
no way out of a run leaves Claude Code behind. When Claude Code exits on its own, whatever it
left in its group (a command started with `&` or `nohup`) gets SIGTERM, then SIGKILL if it is
still there after the same 2 s (`claude::finish`), before the outcome is sent. The guest agent is
not in the group.

### A guest restart

The guest agent is restarted on every deploy (`dev/agent-bus/start-guest.sh`). Its state file
and lock make the restart lose nothing and report an interrupted task exactly once: the old agent
reports it on a clean stop, and the new agent reports it only if the old one died without its
shutdown path.

```mermaid
sequenceDiagram
    participant S as start-guest.sh
    participant O as Old guest agent
    participant F as guest-dev.toml and .lock
    participant N as New guest agent
    participant B as Bus
    S->>O: SIGTERM (pkill)
    alt clean stop
        O->>O: stop Claude Code's group
        O->>B: failed "the guest agent stopped"
        O->>F: final state, running empty, queue kept
        Note over O,B: inbox messages after the stop are not acked
    else no exit within 15 s
        S->>O: SIGKILL
        Note over O,F: running may still be set in the file
    end
    O-->>F: exit releases the flock
    S->>N: start
    N->>F: flock guest-dev.lock, write PID
    N->>B: connect, open inbox dev
    N->>F: load state
    opt running was set
        N->>B: failed "restarted while this task ran"
    end
    N->>N: run the saved queue in order, then the inbox
    B->>N: unacknowledged messages delivered again
```

The interrupted task's session is kept, so an answer still resumes it.

A stop that comes before the task's own run started (while a base session is being primed, or
between priming and the run) is not an interruption: the priming run is stopped, nothing is
reported, no fresh run is started, and the task goes back to the front of the queue in the final
state, to run after the restart (`Finish::Postponed`).

## Authentication and permissions

### NKeys

Every agent has an NKey user key pair (`src/keys.rs`). On connect, the server sends a nonce, the
client signs it with its seed, and the server checks the signature against the public keys in its
config. No password or token goes over the wire. NKeys authenticate; they do not encrypt: TLS
does (see [TLS](#tls)).

| Thing | Where it lives | Secret |
|---|---|---|
| Public key (`U...`) | The agents file (the host, and any fixed guests), the registry (enrolled guests), and the rendered `nats.conf` | No |
| Host seed (`SU...`) | The user's secret store, then the environment of one host command as `AGENT_BUS_SEED` | Yes |
| Enrolled guest seed (`SU...`) | Nowhere at rest: `enroll` prints it into a pipe, which ends in the guest agent's environment as `AGENT_BUS_SEED` | Yes |

Every agent has its own key: no two entries of the agents file and the registry together may
share one, because the bus authorizes keys, not names. Seeds are printed in two places only:
`agent-bus key new`, for the host's key (or a fixed guest's), and `agent-bus enroll`. A seed is
read only from `AGENT_BUS_SEED` (trimmed, and checked to be a user seed), never from a
command-line flag or a file. It is never written to a file or a log, and no error repeats it:
key errors say what is wrong without quoting the value. The guest agent removes
`AGENT_BUS_SEED` from Claude Code's environment, because Claude Code runs commands from network
messages with root.

On the host, the host's seed comes from the secret store into the environment of one command.
A guest's seed is made by `enroll` each time the guest starts and goes straight down a pipe:
`start-agent.sh` pipes it over SSH stdin, never as an argument, and in the guest
`start-guest.sh` reads it from stdin, exports it to the agent process only, and unsets its own
copy. The user stores only the host's seed.

### TLS

The bus has its own private CA, made by `bus start` with rcgen (`src/tls.rs`). It is not the
Sherpa lab CA: the bus is a development tool, and no other certificate should be able to pose as
it. Every connection is TLS from its first byte, and every client trusts only that CA.

- **The CA.** `tls/ca.pem` and `tls/ca-key.pem` in the host's state directory, ECDSA P-256,
  subject `CN=agent-bus CA, O=Sherpa agent-bus`, path length 0, valid about ten years. Made once,
  when neither file exists, and reused by every later start. One file without the other is an
  error that says to remove it to make a new CA. A new CA means every guest agent must be started
  again to get its certificate. The CA key never leaves `tls/`, which is not mounted. A
  certificate is signed with the CA rebuilt from its key and its fixed subject, so the CA
  certificate is never parsed back.
- **The server certificate.** `nats/server.pem` and `nats/server-key.pem`, next to `nats.conf`, so
  the container sees them through the same read-only directory mount. ECDSA P-256, signed by the
  CA, server authentication only, with one IP SAN: the listen address (the lab's gateway with
  `--lab`), the one address the host and every guest connect to. Valid a year, from an hour
  before it was issued so a guest whose clock lags the host's still accepts it.
  `tls/server.toml` records the addresses it names and when it expires, so `bus start` decides
  without parsing it. `bus start` issues it again when there is none, its key or record is
  missing, the CA is new, the listen address changed, or it expires within 30 days, and prints
  which. `enroll` and `unenroll` never touch it.
- **The server.** `nats.conf` has a `tls` block with `cert_file`, `key_file` and
  `handshake_first: true`: the server waits for a TLS ClientHello and sends nothing in
  plaintext, not even its INFO, so a plaintext client gets no answer and is dropped. There are no
  client certificates (no mTLS): NKeys authenticate clients over the TLS connection.
- **The clients.** `Channel::connect` (`channel::tls_options`) connects with `tls_first` and TLS
  required, and adds only the CA certificate as a root, so async-nats loads no system roots. The
  server's certificate must chain to the CA and name the URL's address. A client trusting another
  bus's CA is refused (every bus CA has the same subject, so rustls reports a bad signature). The
  process's rustls provider is aws-lc-rs, as in the rest of the workspace: `main` installs it once
  at startup (async-nats's `aws-lc-rs` feature, through its rustls re-export), and the Docker
  tests install it the same way.
- **The guests.** `start-agent.sh` sends `tls/ca.pem` down the same SSH stdin pipe as the token
  and the seed, after them, to the end of the stream. `start-guest.sh` checks it is a PEM
  certificate, writes it to `bus-ca.pem` in the guest's state directory (0644, replaced
  atomically) and starts the agent with `AGENT_BUS_CA` pointing at it. No file is copied with
  `scp`, and nothing goes on a command line.

**Upgrading.** The host and the guests must be upgraded together, as with the change to
per-sender subjects: an older client speaks plaintext, which the new bus refuses, and a new
client refuses an older plaintext bus. Install the new `agent-bus` and
`agent-bus-start-agent` on the host, run `bus start` again (it makes the CA and the server
certificate, and replaces the container), then start every guest agent again with
`agent-bus:agent`, which builds the new guest binary and gives it the CA. A `start-guest.sh`
from this version refuses to start without the CA on stdin. Run `nats` CLI commands with
`--tlsfirst --tlsca <state>/tls/ca.pem`.

### The agents file

TOML, holding no secrets, written by the user and passed with `bus start --agents <path>`
(`src/agents.rs`). It holds the host and, optionally, guests with fixed keys made with
`key new`; enrolled guests are not in it. One `[[agent]]` per agent:

```toml
[[agent]]
name = "host"
key = "UA..."
role = "host"

[[agent]]
name = "dev"
key = "UB..."
role = "guest"
```

Names follow the agent name rule and are unique, keys must be user public keys used by one
agent each, there is exactly one `host`, and unknown fields are refused. A TOML parse error gives
the line
number and the parser's message with every word of 40 characters or more replaced by
`<hidden>`, so a seed pasted into the file is not echoed.

### Enrollment

`agent-bus enroll <name>` gives a guest a fresh key on the running bus, with the host's seed in
`AGENT_BUS_SEED` (`bus::enroll`). It reads `bus.toml` for the URL and the agents file, and:

1. checks the name, loads the agents file and checks the host's seed, and refuses a name the
   agents file lists;
2. checks that the bus container runs and mounts this state directory's `nats/` directory; a
   container started by an older version, which mounted the file itself, would never see the new
   config, so `enroll` says to run `bus start` again;
3. takes the registry lock (`<state>/enroll.lock`, an exclusive `flock`, waited for);
4. connects as the host and creates the guest's inbox, so a failure there changes no key;
5. generates a user key pair, and records the name, role `guest` and public key in the registry,
   replacing the guest's previous key;
6. merges the agents file and the registry, checks the result, saves the registry (0600,
   atomically) and writes `nats.conf` (atomically);
7. sends the container SIGHUP (Docker's kill with signal `HUP`), and connects with the new seed
   until the bus accepts it (20 attempts, 500 ms apart), which shows the reload took;
8. prints the seed, and only the seed, on stdout. Progress goes to stderr.

`enroll` refuses to run when stdout is a terminal, so the seed lands in a pipe, not on a screen
or in a scrollback. If step 6 or 7 fails, the seed is discarded, so `enroll` saves the previous
registry and config again and reloads the bus: the previous key, if any, works again, and no key
whose seed is lost stays enrolled. The error says so. If putting them back fails too, the error
says to enroll the guest again (which replaces the key) or run `bus start`, which renders and
loads the config from the registry.

`agent-bus unenroll <name>` removes the guest's key from the registry, renders the config and
reloads a running bus (`bus::unenroll`). It needs no seed. A name the agents file lists is
refused, and a name that is not enrolled changes nothing. If the bus is recorded but not
running, only the files change, and the next `bus start` leaves the guest out. With no
`bus.toml` (after `bus stop`), the agents file is not known, so only the registry changes
(`bus::unenroll_standard`): `bus start` renders the config from it, so the key is refused from the next start.
A container that runs without a record is refused, with a message to run `bus start` first. The
guest's inbox and messages stay in the stream.

**Which wins.** The agents file wins a name clash. `enroll` and `unenroll` refuse its names,
including the host's. A registry entry whose name the agents file lists later is left out of the
config, with a warning from every command that renders it. An enrolled key that the agents file
also uses is an error.

**Reloading.** nats-server 2.15 reloads its config on SIGHUP. This was checked on a throwaway
bus in the pinned image, with the config's directory bind-mounted: with users `a` and `b`
connected, the config was rewritten with `a` and a new user `c` and the container sent HUP. The
server logged `Reloaded: authorization nkey users`; `a`'s connection stayed up, `b` was
disconnected and refused on reconnect, and `c` connected. Rewriting a bind-mounted single file by
renaming over it leaves the container reading the old file, so the config's directory is mounted
instead. A guest's permissions depend only on its own name and the host's, so a reload never
changes those of a guest that stays: only the guest whose key changed is disconnected.
`enrolled_guests_get_their_own_keys_and_old_keys_are_refused` (`src/bus.rs`, ignored, needs
Docker) checks the same with the real code: two enrolled guests connect; re-enrolling one refuses
its old key and drops its connection while the other stays connected and keeps reading; once
unenrolled, its key is refused; a `bus start` after that still has the enrolled guest.

### The rendered config

`bus start`, `enroll` and `unenroll` render the nats-server config from the merged agents as
JSON, which nats-server accepts, from serde structs (`src/config.rs`). It holds one
`authorization.users` entry per agent, the host first, then the agents file's guests, then the
enrolled guests: the public key and its publish and subscribe allow lists. There is no field a
seed could go in.
The bus has one account, so these lists are what separate the agents.

| Role | Publish allowed | Subscribe allowed |
|---|---|---|
| host | `agents.>`, `$JS.API.>`, `$JS.ACK.AGENTS.>` | `_INBOX_<host key>.>` |
| guest `<name>` | `agents.<host>.<name>`, `$JS.API.CONSUMER.INFO.AGENTS.<name>`, `$JS.API.CONSUMER.MSG.NEXT.AGENTS.<name>`, `$JS.ACK.AGENTS.<name>.>`, `$KV.AGENT_STATUS.<name>` | `_INBOX_<its key>.>` |

The diagram shows the same lists for a host and one guest, `dev`. Replies to requests (JetStream
API calls, publish acknowledgements, key-value reads) and pulled messages come back on the
connection's inbox subjects. Each connection sets its inbox prefix to `_INBOX_<public key>`
(`custom_inbox_prefix`, `channel::inbox_prefix`); async-nats makes every reply subject from it,
the pull consumer's and the request multiplexer's included. Each agent may subscribe to its own
prefix and nothing else, so no agent reads the replies another pulled. The public key is used
rather than the name because it is what the connection authenticated with, and the client
derives it from its seed.

```mermaid
flowchart LR
    host(["host"])
    dev(["guest dev"])
    allinboxes["agents.> (every inbox)"]
    hostinbox["agents.host.dev"]
    jsapi["$JS.API.> (streams, consumers)"]
    devapi["$JS.API.CONSUMER.INFO and<br/>MSG.NEXT on AGENTS.dev"]
    allacks["$JS.ACK.AGENTS.>"]
    devack["$JS.ACK.AGENTS.dev.>"]
    devstatus["$KV.AGENT_STATUS.dev<br/>(its own status)"]
    hostreplies["_INBOX_(host key).>"]
    devreplies["_INBOX_(dev key).>"]
    host -->|publish| allinboxes
    host -->|publish| jsapi
    host -->|publish| allacks
    dev -->|publish| hostinbox
    dev -->|publish| devapi
    dev -->|publish| devack
    dev -->|publish| devstatus
    host -.->|subscribe| hostreplies
    dev -.->|subscribe| devreplies
```

So a guest can read and acknowledge its own inbox, send to the host as itself and write its own
status, and nothing else: it cannot send to another guest, send as another agent, read another
inbox or another agent's replies, write another agent's
status, read any status (that takes the JetStream API), or create or change a stream or
consumer. Only
the host creates the stream and the consumers. A denied publish does not fail the call:
nats-server drops it and sends an asynchronous `Permissions Violation` error, and the call times
out. Every client prints the connection's events (`Channel::connect`), so a denial shows up as
`Bus connection event: server error: nats: Permissions Violation for Publish to ...`.

`usage` and `show` read the host's inbox history through an ephemeral pull consumer, which the
host's `$JS.API.>` already allows. An ordered push consumer would need `$JS.FC` publish rights for
flow control, so a pull consumer is used and nothing is widened.

## Agent status

`agent-bus agents [--json]`, run as the host, shows every agent of the recorded bus: the host
from the agents file, then every guest (the agents file's, then the enrolled ones, as `bus start`
merges them, `bus::agents`). For each it shows whether it is online, its running task and how
long it has run, how many tasks wait for it, and when it was last active, as a `tabled` modern
table (`src/table.rs`), or with `--json` as a JSON array of the same rows with the parts of each
figure apart (`status::Row`). Nothing is consumed: the host reads each inbox consumer's info and
each guest's status.

### Why a heartbeat

The guest agent acknowledges each message as soon as it reads it and keeps tasks in its own
queue (see [A task's lifecycle](#a-tasks-lifecycle)), so the bus never knows which task runs or
how many wait in the guest: only the guest can say. JetStream offers two signals of its own, and
neither says whether a guest agent is working:

- **The inbox consumer's waiting pull requests** (`num_waiting`). A reader holds one pull
  request (`max_waiting = 1`). It shows a reader is attached, not that the agent is alive: a
  guest agent that hangs, or whose engine has stopped while its connection stays up, keeps the
  pull request until it expires, and the request comes back after every expiry for as long as the
  client runs. Any other reader of the inbox looks the same.
- **The consumer's last delivery** (`delivered.last_active`). It moves only when a message is
  delivered, so an idle guest that is fine looks as inactive as one that is gone.

So each guest agent writes its status to the key-value bucket `AGENT_STATUS`, under its name:

| Field | Meaning |
|---|---|
| `online` | true; false only in the last status, written as the agent stops |
| `at_ms` | when the guest wrote it, by the guest's clock |
| `running` | the running task's ID and when it started (`since_ms`, guest clock), or none |
| `queued` | tasks and answers in the guest's local queue |

The engine updates the status on every change of its state (in `save`, beside the state file)
and a heartbeat task (`guest::run::heartbeat`) writes it on every change and every 10 s
(`status::HEARTBEAT`). When the engine ends (stopped, or the inbox closed) the heartbeat writes it
once more with `online = false`, so a clean stop shows at once. The beat is a parameter of `heartbeat`
and the stale threshold one of `status::row`, so the real-bus test
(`bus::tests::offline_detection`) runs both short: a clean stop is offline at once, and a killed
heartbeat, which writes no last status, is offline once its status is older than the threshold. A write that fails is reported
once until one succeeds again; the agent goes on, since tasks do not depend on it. The write is
a plain JetStream publish to `$KV.AGENT_STATUS.<name>`, so the guest needs that one subject and
no JetStream API call. The host creates the bucket with the stream (`Channel::create_inboxes`,
run by `bus start` and `enroll`).

### What each column means

| Column | Guest | Host |
|---|---|---|
| Online | `yes` while its last status says online and the bus stored it at most 35 s ago (`status::STALE`, three missed heartbeats and slack); `no` otherwise. `unreported` when there is no status but a reader holds its inbox: a guest agent from before the heartbeat, or another reader. | `yes` while a listener holds its inbox (`num_waiting`), `no` otherwise. The host has no agent process to report. |
| Running task, running for | the task in its status, only while online; a stale status's task is not running now, and a restarted guest reports it failed | `-` |
| Queued | the inbox's unread and unacknowledged messages (`num_pending + num_ack_pending`) plus the guest's local queue from its last status | `-`: what waits in the host's inbox is guests' progress and outcomes for a listener, not tasks. Its count is in the JSON's `inbox_unread`. |
| Last active | when the bus stored its last status | when a message was last delivered from its inbox |

The ages are taken against the host's clock from times the bus stamps: the bus runs on the
host, so they agree. A running task's age is the guest's own `at_ms - since_ms` plus the time
since the bus stored the status, so a guest clock set wrong does not skew it. Every name in the
JSON is in `status::Row`: `queued` is `inbox_unread` plus `local_queue` for a guest and `null`
for the host, and `reader_attached`
is the inbox's pull request, kept for diagnosis (a guest with a reader but a stale status is
hung or cut off). A bus prepared by an older `agent-bus` has no bucket: `agents` says so and
shows every guest without a status until `bus start` runs again.

## Guest execution

### Starting the guest agent

`agent-bus guest --server <url> --name <name> --workdir <checkout> [--model <id>] [--reply-to host]`
(`src/guest/run.rs`). In order, it:

1. checks the names, and takes the instance lock (see [Files and state](#files-and-state)); a
   second agent of the same name fails here with the holder's PID, before it touches the bus;
2. requires `CLAUDE_CODE_OAUTH_TOKEN` (from `claude setup-token`): guest agents use the Claude
   subscription, never API billing;
3. connects over TLS, trusting the CA in `AGENT_BUS_CA` (see [TLS](#tls)), and opens its inbox;
4. loads its state file, reports an interrupted task, and saves;
5. runs three loops together until the inbox closes or SIGTERM or SIGINT arrives: the inbox
   reader, the sender of replies, and the engine that runs tasks.

If no state directory can be found (neither `XDG_STATE_HOME` nor `HOME` is absolute), the agent
runs with its state in memory only and without the lock, and says so.

### How Claude Code is started

Each run is one Claude Code process (`claude::claude_command`):

```text
claude -p --output-format stream-json --verbose
       --model <model>
       --dangerously-skip-permissions
       --disallowedTools=<deny list>
       --append-system-prompt <rules>
       [--resume <base> --fork-session | --resume <session>]
```

- **Prompt.** The task or answer text goes in on stdin, which is then closed, so the tool lists
  cannot swallow it as an argument.
- **Model.** The task's `model`, or the guest's default: `claude-opus-5-5` unless `--model` is
  given (`start-guest.sh` passes `AGENT_BUS_MODEL` when it is set). See
  [Model per task](#model-per-task).
- **Process.** It runs in the checkout, in its own process group (`process_group(0)`), with
  `kill_on_drop`. Stdout is read line by line; stderr goes to the guest agent's log.
- **Environment.** `ANTHROPIC_API_KEY` and `AGENT_BUS_SEED` are always removed. Without GitHub
  access `GH_TOKEN`, `GITHUB_TOKEN` and `GH_ENTERPRISE_TOKEN` are removed too, so `curl` with the
  token cannot get round the deny list.
- **Permissions.** Permission checks are skipped, because the guest is a disposable lab VM and
  the agent has root in it. The deny list still refuses some commands.
- **Rules.** The appended system prompt says it is a guest agent, to follow `AGENTS.md`, not to
  add dependencies, to end with a `QUESTION:` line when it needs a decision and otherwise with a
  short summary, that the git status in its context may be stale, and never to use `git stash`,
  `git restore`, `git clean` or `git checkout -- <path>`. It also says to give each subagent
  the cheapest model that fits (haiku for searches, reading files and lookups; sonnet for
  routine edits, builds, tests and docs; opus only for design, hard debugging and review) and to
  do small things itself rather than spawn one. A GitHub sentence follows (below).

The outcome is read from the `result` event: `is_error` true (or missing) is `failed`, a line
starting `QUESTION:` (after leading spaces) is `question`, anything else is `completed`. A run
that exits without a `result` event is reported `failed` with its exit status.

### GitHub access per task

`agent-bus send --github` sets `github` on the task. An answer cannot set it (`--github`
conflicts with `--answer`, and the guest ignores `github` on an answer): it uses the flag stored
with the task's session (`engine::task_github`).

| | Without `github` | With `github` |
|---|---|---|
| Tokens | `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` removed | Kept |
| Denied | `git commit`, `git push`, `git reset`, `gh`, and the discarding commands below | `gh pr merge`, `gh api`, `gh repo delete`, `gh release`, `git reset`, `git push` with `--force`, `-f`, `--delete`, `-d`, a `+` refspec or `:branch`, `git branch -D`, and the discarding commands below |
| Rules add | The task has no GitHub access: no commit, push, merge, pull request or `gh`. | Do only what the task asks; commit messages end with the Co-Authored-By line from its own attribution instructions, never one copied from the task; no session links or "Generated with" footers; never merge or delete. |

With the token present the deny list is a guardrail, not a security boundary: it matches command
text, so a command written another way (`curl` with the token, a script) is not caught. The
token's own scopes are the boundary.

Both lists also refuse the commands that throw away uncommitted work, because the guest's working
tree is the only copy of a branch's uncommitted changes: `git stash`, `git restore`, `git clean`,
and `git checkout` with `--`, `.`, `./…`, `-f` or `--force`. The rules say the same for both
kinds of task, and point at `git show HEAD:<path>` and `git diff` for seeing how things were
before. The patterns match command text and cannot tell `git checkout <branch>` from
`git checkout <path>`, so a bare `git checkout <path>` is not caught; `git checkout <branch>`
keeps working.

### Model per task

`agent-bus send --model <alias or id>` sets `model` on a task. `send` and the guest both turn it
into a full ID with `model::model_id`, the one place the aliases map to the current IDs:
`opus` is `claude-opus-5-5`, `sonnet` is `claude-sonnet-5-5`, `haiku` is `claude-haiku-5-5`. A
full ID is `claude-` followed by lowercase letters, digits, hyphens and dots; anything else is
refused by `send`, and a task that names one anyway is reported `failed` without running. The
guest's `--model` takes the same values.

Like the GitHub flag, an answer cannot change it: `--model` conflicts with `--answer`, and the
guest runs an answer on the model stored with the task's session (`engine::task_model`). A
session saved before tasks chose their model runs on the guest's default. The guest's replies
for a run carry the full ID, the guest log's usage line starts `Task <id> <kind> on <model>`, and
`agent-bus usage` has a Model column.

### Base sessions and forking

Claude Code's last prompt cache breakpoint comes after the first user message, which holds the
task text. So every new session writes the whole prefix (system prompt, `AGENTS.md`, the
environment block) to the cache again. To avoid that, a new task forks a primed base session, and
its text comes after a prefix that is already cached (`src/guest/base.rs`, `Engine::fork_of_base`).

- **One base per rules variant and model**: GitHub or no-GitHub, and each model a task runs
  on, because a fork keeps its base's system prompt and model. The bases are kept by
  `(github, model)`, so a task on another model primes its own base and leaves the others in
  place.
- **Priming** is two runs, done lazily when a task needs a base: a fresh session with "You are a
  guest agent. Tasks will follow, one per session. Reply with exactly: ready.", then a resume of
  it with "Confirm once more: reply with exactly: ready." The base is the session the second run
  ends in. A fork is then the third turn, after Claude Code's context has settled.
- **Inputs.** A base records a hash of its inputs: the rules text, `AGENTS.md`, the model and
  the UTC day. It is rebuilt when the hash differs, or when it has not been used for 55 minutes,
  inside the cache's 1 hour lifetime. Each fork counts as a use. The git branch and `HEAD` are
  not inputs, so a commit does not re-prime every base; the git status in a base's context can
  be out of date, and the rules say so.
- **Tasks** start with `--resume <base> --fork-session`. A base is never resumed directly by a
  task. Answers resume the task's own forked session.
- **Accounting.** Both priming runs are counted in the `task_usage` of the task that needed them
  (`runs` includes them), the guest log has a "priming run" line for each, and `results.jsonl`
  records them as run 0 of that task.
- **Fallback.** If priming fails, or the fork produces a failed `result` with no turns, or ends
  with no `result` and nothing done before (no session named, no text, no tool call; for example,
  the base session is gone), the guest agent logs it, drops the base, and runs the task in a
  fresh session. A fork that did something and then ended without a `result` is not run again,
  which would repeat its commits or pushes: the task is reported `failed`. A stop while priming
  starts no fresh session (see [A guest restart](#a-guest-restart)).

The flowchart shows how a new task gets its session.

```mermaid
flowchart TD
    task["new task, rules variant, model"] --> check{"base for the variant and model,<br/>same inputs hash,<br/>used within 55 min?"}
    check -->|yes| fork
    check -->|no| p1["priming run 1<br/>fresh session"]
    p1 -->|ok| p2["priming run 2<br/>--resume run 1's session"]
    p2 -->|ok| save["base saved in the state file"]
    save --> fork["task run<br/>--resume base --fork-session"]
    p1 -->|fails| fresh["task run in a fresh session"]
    p2 -->|fails| fresh
    fork -->|"no result and nothing done,<br/>or failed with 0 turns"| drop["drop the base"]
    fork -->|"no result after doing something"| failed["task failed"]
    drop --> fresh
    fork -->|result| own["the task's own session<br/>answers resume it"]
    fresh --> own
```

Measured API-equivalent figures (see the investigation for the runs). The Sonnet rows are from
before guests moved to Opus 5.5:

| Start of a new task | Model | Cache written | Cache read | Cost of a small task |
|---|---|---|---|---|
| Fresh session | Sonnet | about 9,150 tokens | | about $0.038 |
| Fork of a base primed with one turn | Sonnet | about 3,770 tokens | | about $0.017 |
| Fork of a base primed with two turns | Sonnet | about 35 tokens | | about $0.0025 |
| Fork of a base primed with two turns | Opus 5.5 | 32 to 39 tokens | 22,691 tokens | about $0.005 |

Priming costs about one fresh session and one resume, once per base per inputs change or idle
hour. Priming a new Opus 5.5 base cost about $0.13, once (task c9488976). The git status in a
base's context is from when it was primed, which is why the rules tell Claude to run
`git status` itself when it matters.

## Usage and cost

The figures come from Claude Code's `result` event (`stream::interpret`). Some of its fields
cover the run, and some the whole session so far, across resumes:

| Event field | Scope | Used as |
|---|---|---|
| `usage.input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens` | this run | `input`, `output`, `cache_creation`, `cache_read` |
| `num_turns`, `duration_ms` | this run | `num_turns`, `duration_ms` |
| `model_usage_since_last_result.<model>` | this run, per model | `models`; their `costUSD` summed is `cost_usd` |
| `duration_api_ms` | session so far | minus the session's previous run's: `duration_api_ms` |
| `total_cost_usd` | session so far | `session_cost_usd`, not the run's cost |
| `modelUsage` | session so far | not used |

- **A run's API time** is the event's `duration_api_ms` minus the figure after the session's
  previous run (`message::api_time_of_run`). The first run of a session, and a figure that went
  down, are taken as reported. The guest keeps the last figure per task and session (`api` in the
  state file); a fork is a new session, so its first run is taken as reported.
- **A task's totals** (`task_usage`) are the sum of its counted runs (`TaskUsage::add`, saturating):
  the priming runs it triggered, its first run, and every answer. A run with no `result` event is
  not counted.
- **Cost is not billed.** Guest agents run on the subscription; `cost_usd` is what the run would
  cost at list prices (`costBasis` `list`). Use it to compare tasks, not as money spent.
- **Cache hit** is `cache_read / (input + cache_creation + cache_read)`, in tenths of a percent
  rounded down, shown as `89.9%`, or `n/a` with no input (`message::cache_hit_permille`).

The guest log has one line per finished run with the run's figures and the task's so far, and
one line per priming run. Every `result` event line is also appended unchanged to
`results.jsonl` (see [Files and state](#files-and-state)), to check what the figures mean.

`agent-bus usage [--agent <guest>] [--host <name>] [--all]` reads every message the stream still
holds for the host's inbox (7 days) through an ephemeral pull consumer with no acknowledgement
(`Channel::history`: deliver all, 256 per fetch, removed after use or after 30 s idle), so the
host's inbox is neither consumed nor changed. It prints one row per task that had an outcome
(`question`, `completed`, `failed`, `canceled`): task, agent, model (from its outcomes'
`model`, `n/a` before guests sent it), runs, cost, cache hit, tokens (the four kinds added) and
the kind of its last outcome, then a total row, as a `tabled` modern table
(`src/report.rs`). A task's figures are the `task_usage` of its last outcome that carried one.
Tasks with no usage are left out unless `--all` is given. `agent-bus show <task>` prints every
kept message of one task, oldest first, one JSON line each, the same way, from the bus rather than
the listener's journal.

## Files and state

The state directory is `$XDG_STATE_HOME/agent-bus/`, or `~/.local/state/agent-bus/` when
`XDG_STATE_HOME` is unset or relative. The host and the guest each have their own. On both it is
0700: every command that writes into it creates it, or tightens an existing one, with
`files::private_dir` (`src/files.rs`). The server config's directory `nats/` in it is 0755 and
`nats.conf` 0644: the `nats/` directory itself is bind-mounted, so the container opens it by its
mount path and never looks up the state directory; their own modes are what the container's
user needs (the pinned image runs as root, but a non-root user would read them too). The
real-bus tests start the bus with its state in a 0700 directory.

| Where | File | Mode | Contents | Written by |
|---|---|---|---|---|
| Host | `<state>/nats/nats.conf` | 0644, `nats/` 0755, state directory 0700 | The rendered server config as JSON: public keys and allow lists. Rewritten, with `files::write_atomic` (below), on every `bus start`, `enroll` and `unenroll`. Readable by the container's user. | `config::write` |
| Host | `<state>/tls/ca.pem` | 0644, `tls/` 0700 | The bus CA's certificate. Made once by `bus start`; the CA every client trusts. Sent to each guest by `start-agent.sh`. | `tls::ensure` |
| Host | `<state>/tls/ca-key.pem` | 0600, `tls/` 0700 | The bus CA's private key. Never mounted or copied. | `tls::ensure` |
| Host | `<state>/tls/server.toml` | 0600, `tls/` 0700 | The addresses the server certificate names and its expiry (`not_after`). Removed before a new certificate is written, and written after it. | `tls::ensure` |
| Host | `<state>/nats/server.pem`, `<state>/nats/server-key.pem` | 0644 and 0600, `nats/` 0755 | The server certificate and key, read by nats-server through the mounted `nats/` directory (the pinned image runs as root, so it reads the 0600 key). | `tls::ensure` |
| Host | `<state>/enrolled.toml` | 0600, directory 0700 | The registry of enrolled guests, in the agents file's format, guests only, with a header saying it is managed by agent-bus. Written with `files::write_atomic`. A missing file is an empty registry. | `registry::Registry::save` |
| Host | `<state>/enroll.lock` | 0600, directory 0700 | Empty; the exclusive `flock` that `bus start`, `enroll` and `unenroll` hold while they change the registry or render the config. Never removed. | `registry::lock` |
| Host | `<state>/bus.toml` | 0600, directory 0700 | `url`, `agents` (the agents file's absolute path), and `[lab]` with `id` and `name` when started with `--lab`. Written with `files::write_atomic`. Removed by `bus stop`. | `state::write` |
| Host | `<state>/listen-<agent>.jsonl` | 0600, directory 0700 | The listener's journal: one line per message received, `{"at_ms":..,"message":{..}}`, the message as `listen` prints it. Appended and in the file before the message is printed and acknowledged. Pruned to 7 days (below). | `listen::Journal` |
| Host | the agents file | the user's | The host's and any fixed guests' names, public keys and roles. Only read. | the user |
| Host | Docker volume `sherpa-agent-bus-data` | Docker's | JetStream's store: the `AGENTS` stream and the consumers. | nats-server |
| Guest | `<state>/bus-ca.pem` | 0644, directory 0700 | The bus CA's certificate, from the SSH stdin pipe; the agent's `AGENT_BUS_CA`. Replaced on every start. | `start-guest.sh` |
| Guest | `<state>/guest-<name>.toml` | 0600, directory 0700 | The guest agent's state (below). Written with `files::write_atomic` (below). A file that cannot be read or parsed is moved to `guest-<name>.toml.corrupt-<unix seconds>` and the agent starts fresh. | `guest::state` |
| Guest | `<state>/guest-<name>.lock` | 0600, directory 0700 | The PID of the agent holding the lock. Never removed. | `guest::lock` |
| Guest | `<state>/results.jsonl` | 0600, directory 0700 | One JSON line per `result` event: `task`, `session`, `run` (0 for priming, then 1, 2, ...), `at_ms`, and `raw`, the event line unchanged. Appended; pruned to 7 days (below). | `guest::record` |
| Guest | `~/agent-bus-<name>.log` | default (umask) | The guest agent's stdout and stderr, Claude Code's stderr included. Truncated on each start. | `start-guest.sh` |

Claude Code keeps its sessions in its own files; the guest agent only records which session is
whose.

`results.jsonl` and the listener's journal are pruned to the stream's retention, 7 days
(`files::JsonLines`): lines whose `at_ms` is older are dropped, and a line without one (cut short
by a crash) is kept. Each is pruned when its program starts, and again before an append once a
day has passed since the last prune; a file that is not appended to does not grow. The file is
read whole and rewritten only when a line is dropped, with `files::write_atomic`, which every
rewritten file goes through: to `<file>.<pid>.tmp`, with the file's mode (0600 for state files,
0644 for `nats.conf`) whatever the umask, synced, renamed over the file, then the directory
synced. Pruning and appending happen in the same task (the
guest's engine, the listener's loop), so no append is lost to the rename. A failed prune is
logged and the append goes on.

### The guest state file

TOML. It holds no secrets; the only message text is that of the queued tasks and answers, which
they need to run. Times are milliseconds since the Unix epoch. The inputs hash is a `u64`, which
a TOML integer (signed) cannot always hold, so it is kept as 16 hex digits.

| Key | Contents |
|---|---|
| `running` | The task Claude Code was working on, if any. |
| `[[queue]]` | The waiting tasks and answers, in order: `task`, `from`, `kind`, `github`, `model` (when set), `text`. |
| `[[bases]]` | Each primed base: `github`, `model`, `session`, `inputs` (hash), `primed_at_ms`, `last_used_ms`. A base saved without `model` matches no task's model and is replaced. |
| `[tasks.<id>]` | Each task: `runs` (for `results.jsonl`), `finished_at_ms`, `session` (`id`, `github`, `model`), `api` (`session`, `api_ms`), and `usage`, its `task_usage` totals. |

It is written after every change: a task queued, started, finished or canceled, a session
recorded, a base primed, used or dropped, a run counted. A failed write is logged and the agent
goes on with its state in memory.

- **Start.** A missing file is a fresh start. A `running` task means the previous agent died
  without its shutdown path (SIGKILL, a crash, power loss): it is reported `failed` with "The
  guest agent restarted while this task ran", and its session is kept. The saved queue runs
  first, in order, before new inbox messages. Bases come back and are checked at the next fork as
  usual.
- **Shutdown.** On SIGTERM or SIGINT the running group is stopped and the task reported `failed`
  ("The guest agent stopped: the task was interrupted"); messages that reached the agent but not
  its queue are queued; the final state, with `running` empty, is written. Once stopping, the
  inbox reader acknowledges nothing more, so the bus delivers those messages again after the
  restart.
- **Cleanup.** Tasks whose last run ended more than 7 days ago (the stream's retention) are
  dropped on start and after each task, unless one of their messages is queued.

### The instance lock

Two agents of one name would each write their own view over the other's state. Before it
connects to the bus, subscribes to its inbox or loads the state, the guest agent takes an
exclusive, non-blocking `flock` on `guest-<name>.lock`, writes its PID into it and holds it for
its lifetime. If the lock is held, it exits with "Another guest agent of this name is running
(PID n holds ...)". The kernel drops the lock however the holder exits, so a file left by a dead
agent does not block. The file is never removed, as removing it would let two agents lock two
files of one name. The descriptor is close-on-exec, so Claude Code does not inherit it.

`start-guest.sh` also waits for the old agent: SIGTERM, up to 15 s (the agent's own kill grace is
2 s), then SIGKILL and 5 s more, and it refuses to start a second agent if one is still there.

## Operating it

The host keeps `mise` tasks for these steps in its own mise configuration, outside this
repository; they load the host's seed and the Claude token from the user's secret store (a
`pass-cli` env file) into the one command that needs them, so no secret is typed or written down.
The only bus secret stored is the host's seed: guests get a fresh key each time they start.

| Task | What it does |
|---|---|
| `agent-bus:bus` | `agent-bus bus start --lab` with the dev lab and the agents file |
| `agent-bus:agent <lab dir> <node>` | `~/.local/bin/agent-bus-start-agent <lab dir> <node>`: enrolls the node's guest agent and starts it |
| `agent-bus:send` | `agent-bus send`: a task, or an answer with `--answer` |
| `agent-bus:listen` | `agent-bus listen host` |
| `agent-bus:cancel` | `agent-bus cancel` |
| `agent-bus:usage` | `agent-bus usage` |
| `agent-bus:show` | `agent-bus show <task>` |

`agent-bus:agent` replaces the per-agent tasks that piped a stored seed for each guest. It holds
no secret itself; `<load secrets>` stands for however the other tasks load the host's
`AGENT_BUS_SEED` and `CLAUDE_CODE_OAUTH_TOKEN` from the secret store. The host has no checkout and builds nothing: build `agent-bus` in the dev guest and
install the binary and the script on the host as `~/.local/bin/agent-bus` and
`~/.local/bin/agent-bus-start-agent` (from `dev/agent-bus/start-agent.sh`). The script uses
`agent-bus` on PATH, or the binary named by `AGENT_BUS_BIN`, and fails if it is missing or not
executable:

```toml
[tasks."agent-bus:agent"]
description = "Enroll a lab node's guest agent on the bus and start it"
usage = '''
arg "<lab_dir>" help="The lab directory, holding .sherpa/lab-info.toml and .sherpa/ssh/config"
arg "<node>" help="The node, which is also the agent's name"
'''
run = '<load secrets> ~/.local/bin/agent-bus-start-agent "$usage_lab_dir" "$usage_node"'
```

Run it as `mise run agent-bus:agent <lab dir> <node>`, for example
`mise run agent-bus:agent ~/labs/sherpa-agent-2 agent-2`.

In order:

1. **The host's key, once.** `agent-bus key new`; store the seed in the secret store straight
   away, as it is not shown again. Write the agents file with the host's public key. Guests need
   no key made by hand.
2. **The lab.** Bring up the dev lab (`tests/environments/dev/README.md`).
3. **The bus.** `agent-bus bus start --lab <lab dir> --agents <file>` with the host's seed. It
   comes before the guest agents and the listener.
4. **The guest agents.** `mise run agent-bus:agent <lab dir> <node>` for each, on the host
   (`dev/agent-bus/start-agent.sh`). The script finds the node's SSH alias, `<node>` or
   `<node>.<lab id>`, in the lab's `.sherpa/ssh/config` (with `id` from `.sherpa/lab-info.toml`),
   and the bus's URL in `bus.toml`; like `bus start`, it refuses a lab folder in the old layout. It uses the installed `agent-bus` binary, checks the guest answers over
   SSH, then pipes over SSH stdin to `dev/agent-bus/start-guest.sh <node>` in the guest, with
   `AGENT_BUS_URL` set to the bus's URL: the Claude token, then the seed straight from
   `agent-bus enroll <node>`, each on a line, then the bus's CA certificate, `tls/ca.pem`. The seed never sits in a variable, a command line, a file or a
   log, and `ssh` runs without the secrets in its environment. If `enroll` fails, the guest
   script gets no seed and exits before it touches the running agent. In the guest,
   `start-guest.sh` builds the release binary, stops any agent of that name, and starts
   `agent-bus guest` in the background. Starting it needs the user: Claude Code's auto mode does
   not let the host agent start an agent that runs commands from network messages unless a
   permission rule allows that one command.
5. **The listener.** `agent-bus listen host` with the host's seed, under Claude Code's Monitor.
   Run only one: a second is refused. After a restart, `agent-bus listen host --since <duration>`
   first prints what the journal got in that time, covering anything printed while no Monitor
   was reading.
6. **Tasks.** `agent-bus send --to dev [--model sonnet] "<task>"`,
   `send --to dev --answer <task> "<answer>"`,
   `agent-bus cancel --to dev <task>`.

Restarting a guest agent is the same command: it enrolls the agent again, so the key the old
process had is refused from then on. To retire a guest, stop its agent and run
`agent-bus unenroll <name>`.

**Moving from stored guest keys.** Guests listed in the agents file keep their fixed keys and
cannot be enrolled. To move one to enrollment, remove it from the agents file, run `bus start`
again, then start it with `agent-bus:agent`. Guests may no longer share a key: an agents file
that lists one key twice is refused.

More guests run in worker labs, one VM per guest agent, each its own lab:
`tests/environments/agent` (lab `sherpa-agent-2`, node and agent `agent-2`), brought up from a
lab directory outside the repository, with dev's provisioning and no Sherpa server. The bus
stays on the host, started with `--lab` on dev's lab, so it listens only on dev's gateway. A
worker's guest agent is started the same way, `mise run agent-bus:agent <worker lab dir>
<node>`: `start-agent.sh` always passes the URL from `bus.toml` as `AGENT_BUS_URL`, which
`start-guest.sh` uses in place of the guest's own default gateway. A worker on its own bridge
reaches dev's gateway with no firewall change; the check and its result are in
`tests/environments/agent/README.md`.

Checking health:

- `agent-bus bus status`: the container's state, the recorded URL, the lab and the CA certificate.
- The `bus start` output: one `Inbox ready` line per agent.
- `<state>/enrolled.toml`: the enrolled guests and their public keys.
- `agent-bus usage`: what each recent task cost, and its last outcome.
- `agent-bus listen host --since 1h`, or `<state>/listen-host.jsonl` on the host: what the
  listener received, for an outcome the host agent missed.
- `agent-bus show <task>`: every message of one task the bus still keeps.
- The guest log, `~/agent-bus-<name>.log` in the guest: the lock and state paths, each task and
  answer taken, one usage line per run, priming lines, and any `Bus connection event`, such as a
  permission violation.

The `nats` CLI (in the dev server's mise tools) can inspect the bus with `--nkey <seed file>
--tlsfirst --tlsca <state>/tls/ca.pem`, but that puts a seed on disk; prefer `agent-bus listen`
and `show`.

## Limits and next steps

- **The host is all-powerful.** The host key can read and write every inbox and is the only key
  that can change the stream. A guest key reaches that guest's inbox and the host's, so a
  compromised guest can flood the host's inbox.
- **TLS without revocation.** The bus CA has no revocation list: a leaked CA key is fixed by
  removing `tls/`, running `bus start` and starting every guest again. The CA key sits in the
  host's state directory, protected by its 0700 mode only.
- **One lab per bus.** One container, one address, one state file. Guests in other labs reach the
  bus at the bus lab's gateway through the host, with no firewall change (checked), so one bus
  serves them all. The bus depends on that lab, though: a new dev lab means a new address, and
  every guest outside it must be restarted with the new `AGENT_BUS_URL` (`bus start` issues the
  server certificate for the new address from the same CA).
- **One task at a time per guest.** The rest wait in its queue.
- **Removing an agent**: `unenroll` for an enrolled guest, or removing a fixed one from the
  agents file and running `bus start`, drops its access; its consumer and messages stay in the
  stream.
- **Enrolling cuts off the running agent.** `enroll` refuses the previous key at once, so an
  agent whose `start-agent.sh` fails after enrolling (the guest unreachable mid-way, say) stays
  disconnected until the script is run again. The script checks SSH first to make that rare.
- **The registry is trusted as written.** Anyone who can write the host's state directory can
  enroll a key; it is 0700 for that reason.
- **Windows left.** A message is acknowledged before it reaches the state file's queue, so a
  crash between the two loses it. The inputs hash uses Rust's `DefaultHasher`, which is not
  promised to be stable across Rust releases; a change costs one rebuild of each base. The
  listener's journal is written without `fsync`, so it survives the listener being killed, not a
  host crash.

Open work, including more than one task at a time and more guests, starting the guest agent from
provisioning, several labs, and whether A2A belongs on top of the bus (#439), is tracked in issue
#494.
