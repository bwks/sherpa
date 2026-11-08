# Agent bus: a message channel between the host agent and guest agents

> Status: proof of concept, working. Tracked in #494. Follows from #439 (A2A), which stays open
> for the protocol question. The code is the `agent-bus` crate (`crates/agent-bus`), a development tool that is
> not released.

## Goal

The user works with one AI coding agent on the host. Agents in lab guests, starting with the
dev server (`tests/environments/dev`), do the work. Until now the host agent did that work
itself, one SSH command at a time, with every build log landing in its context. The bus lets
the host agent hand a guest agent a whole task and get back its progress, its questions for the
user, and its result.

## What was proven (2026-11-08, before authentication)

1. Task `8aaadbbc`, "run clippy and the tests for `agent-bus`, change nothing": the guest agent
   ran them on the Claude subscription and sent back each tool call as progress and a
   `completed` result (clippy clean, 9 of 9 tests passed).
2. Task `9c97040b`, "ask me which crate to test first": the guest agent stopped with a
   `question`. The user answered through the host agent (`agent-bus`), the answer resumed the
   same Claude session, and the task completed.

Each message reached the host agent as a notification, with no SSH round trip and no build
output in its context.

## How it works

| Part | What it is |
|---|---|
| Bus | NATS 2.15 with JetStream, in the container `sherpa-agent-bus` on the host (`agent-bus bus start --lab <lab dir> --agents <file>`), on the host's network. It listens on one address: the lab's management bridge, which is the guest's default gateway. Stored messages live in the volume `sherpa-agent-bus-data` |
| Inboxes | One stream, `AGENTS`, on `agents.>`. Each agent reads `agents.<name>` through a durable consumer, so a message waits until its agent reads it, and a restarted reader goes on where it stopped. The host creates the stream and the consumers when it starts the bus |
| Messages | JSON: task ID, sender, receiver, kind and text. The kinds are `task`, `answer`, `progress`, `question`, `completed` and `failed`, named after A2A's task states |
| Host agent | `agent-bus send --to dev "<task>"` prints the task ID; `--answer <task>` answers a question. `agent-bus listen host`, run under Claude Code's Monitor, turns each message into a notification |
| Guest agent | `agent-bus guest` runs each task through `claude -p` with `stream-json` output in the checkout, one at a time (see Cancel and the queue). Tool calls and text become progress; a reply line starting `QUESTION:` makes the outcome a question; an answer resumes the task's Claude session |

### Guest rules

- The guest agent uses the Claude subscription: `CLAUDE_CODE_OAUTH_TOKEN` from
  `claude setup-token`, given on stdin when it starts. `ANTHROPIC_API_KEY` is removed from
  Claude Code's environment, so it can never fall back to API billing.
- Claude Code runs with `--dangerously-skip-permissions`: permission checks are skipped
  because the guest is a disposable lab VM where the agent has root. Its instructions say to
  follow `AGENTS.md` and to ask instead of adding dependencies.
- GitHub access is given per task. `agent-bus send --github` sets `github` on the task message
  (default false, left out of the JSON when false).
  - Without it, `GH_TOKEN`, `GITHUB_TOKEN` and `GH_ENTERPRISE_TOKEN` are removed from Claude
    Code's environment, with `ANTHROPIC_API_KEY` and `AGENT_BUS_SEED`, so `curl` with the token
    cannot get round the deny list. `git commit`, `git push`, `git reset` and `gh` are refused
    (`--disallowedTools`), and the rules say the task has no GitHub access.
  - With it, the token stays and the deny list is narrower: `gh pr merge`, `gh api`,
    `gh repo delete`, `gh release`, `git reset`, `git push` with `--force`, `-f`,
    `--force-with-lease`, `--delete`, `-d`, a `+` refspec or `:branch`, and `git branch -D`.
    Merging and destructive operations stay with the user. With the token present this list is a
    guardrail, not a security boundary: it matches command text, and the token's own scopes are
    the boundary. The rules say to do only what the task asks, to end commit messages with the
    Co-Authored-By line the task gives, and to leave out session links and "Generated with"
    footers.
  - An answer resumes the session with the flag of its original task, which is stored with the
    session. An answer can never turn GitHub on (`--github` cannot be combined with `--answer`,
    and a `github` field on an answer is ignored).

## Authentication

Every agent has an NKey user key pair and connects with it: the server sends a nonce, the client
signs it with its seed, and the server checks the signature against the public keys in its config.
There are no passwords or tokens on the wire.

| Thing | Where it lives | Secret |
|---|---|---|
| Public key (`U...`) | The agents file, and so the rendered server config | No |
| Seed (`SU...`) | The user's secret store, then the one agent's environment as `AGENT_BUS_SEED` | **Yes** |

A seed is only ever read from the environment (or, for `dev/agent-bus/start-guest.sh`, from
stdin). It is never a command-line value (there is no flag for it), never
written to a file, never logged, and not repeated in error messages (a seed pasted into the agents
file is refused without being echoed). `agent-bus key new` is the only place that prints one. The
guest agent removes `AGENT_BUS_SEED` from Claude Code's environment, as Claude Code runs commands
from network messages with root.

### The agents file

TOML, holds no secrets, passed with `bus start --agents <path>`. One `[[agent]]` per agent:

```toml
[[agent]]
name = "host"
key = "UA..."      # public key from `agent-bus key new`
role = "host"

[[agent]]
name = "dev"
key = "UB..."
role = "guest"
```

Names follow the agent name rules (lowercase letters, digits and hyphens), keys must be user
public keys, names and keys are unique, and there must be exactly one `host`. Unknown fields are
refused.

### Permissions

`bus start` renders the nats-server config as JSON from the agents file (nats-server accepts JSON;
it is built from serde structs). The bus has one account, so the permissions do the separating.
Replies to requests come back on `_INBOX.>`, so every agent may subscribe to that and nothing else.

| Role | Publish allowed |
|---|---|
| guest `<name>` | `agents.<host name>`, `$JS.API.CONSUMER.INFO.AGENTS.<name>`, `$JS.API.CONSUMER.MSG.NEXT.AGENTS.<name>`, `$JS.ACK.AGENTS.<name>.>` |
| host | `agents.>`, `$JS.API.>`, `$JS.ACK.AGENTS.>` |

So a guest can read and acknowledge its own inbox and send to the host, and nothing else: it
cannot send to another guest, open another inbox, or create or change a stream or consumer. A
denied call does not fail: nats-server drops it and sends an asynchronous `Permissions Violation`
error, and the call times out. Every client prints the connection's error events, so a denial
shows up as `Bus connection event: server error: nats: Permissions Violation for Publish to ...`.

### Starting the bus

`agent-bus bus start --lab <lab dir> --agents <file>`, with the host's seed in
`AGENT_BUS_SEED`. Exactly one of `--lab` and `--listen <address>` is required: `--lab` reads
`lab-info.toml` in the lab directory and listens on its `ipv4_gateway`, and `--listen` gives an
address directly. A missing file, or one without a gateway, is an error that names the path.

1. Read the lab's gateway, if `--lab` was given.
2. Read and validate the agents file, and check the seed belongs to the host's key.
3. Write the config to `$XDG_STATE_HOME/agent-bus/nats.conf` (default
   `~/.local/state/agent-bus/`), outside the repository, mode 0644 in a 0755 directory so the
   container's user can read it. It is rewritten on every start.
4. Pull the pinned image if needed, then **replace** the container: stop and remove the old one
   if present, and create it with the config bind-mounted read-only (`-c`). The config can change,
   so an existing container is never just started. Messages live in the volume, so replacing
   is safe.
5. Wait for the server, connect as the host, and create the `AGENTS` stream and a durable consumer
   for every agent in the file, the host included (it reads its replies through one). Only the
   host may do this. The other agents bind to their consumer and fail with a clear error if it is
   missing, for example when the agent is not in the agents file the bus was started with.
6. Record where the bus is in `$XDG_STATE_HOME/agent-bus/bus.toml`, next to `nats.conf`, written
   atomically: the URL, and the lab's ID and name when `--lab` was used. `bus stop` removes it,
   and `bus status` prints it.

Adding an agent: generate a key, add it to the agents file, and run `bus start` again.

## Usage

Each finished run records what Claude Code reported in its `result` event, so the host can see
how efficient a task was. The outcome messages (`completed`, `question`, `failed`) carry it as
`usage` (this run) and `task_usage` (the task so far, its first run and its answers). A
`canceled` message carries them when the `result` event arrived before the cancel; otherwise the
run is not counted and the canceled text says its cost is unknown. Progress messages and failures
that never reached a result (Claude Code did not start, or exited without one) carry none. The
guest log gets one line per finished run, with the run and the task total.

### What Claude Code's figures mean

Measured with task 562a9215: one session, three runs (the first and two resumes). The raw
`result` events are kept in `results.jsonl` in the guest's state directory.

| Event field | Scope | Run 1 | Run 2 | Run 3 |
|---|---|---|---|---|
| `usage.*` | this run | 2 / 16 / 9281 / 13213 | 4 / 15 / 3777 / 18761 | 4 / 4 / 37 / 22538 |
| `model_usage_since_last_result.<model>.costUSD` | this run | $0.0386 | $0.0171 | $0.0024 |
| `total_cost_usd` | **session total so far** | $0.0386 | $0.0558 | $0.0582 |
| `modelUsage` | **session total so far** | cumulative | cumulative | cumulative |
| `duration_api_ms` | **session total so far** | 1556 | 2937 | 4218 |
| `duration_ms`, `num_turns` | this run | 1994, 1 | 1748, 1 | 1695, 1 |

(`usage.*` is input / output / cache creation / cache read.) The per-run costs add up to the
session's $0.0582. So:

- A run's cost is the sum of the `costUSD` of the entries in `model_usage_since_last_result`,
  which is per run and per model. `total_cost_usd` is not a run's cost; it is kept as
  `session_cost_usd`. `modelUsage` is not used.
- A run's API time is `duration_api_ms` minus the previous run's in the same session (the first
  run takes it as is). The arithmetic is checked: a figure that went down means the count started
  again, and the figure is then taken as is.
- The per-model entries keep the names Claude Code writes (`inputTokens`, `outputTokens`,
  `cacheReadInputTokens`, `cacheCreationInputTokens`, `costUSD`, `canonicalModel`, `costBasis`;
  `costBasis` was `list`, list prices).

| Field | Claude Code field | Meaning |
|---|---|---|
| `input` | `usage.input_tokens` | Input tokens not read from or written to the cache |
| `output` | `usage.output_tokens` | Output tokens |
| `cache_creation` | `usage.cache_creation_input_tokens` | Input tokens written to the prompt cache (a miss) |
| `cache_read` | `usage.cache_read_input_tokens` | Input tokens read from the prompt cache (a hit) |
| `num_turns` | `num_turns` | Model turns in the run |
| `duration_ms` | `duration_ms` | Wall-clock time of the run |
| `duration_api_ms` | `duration_api_ms` minus the previous run's | Time this run spent in API calls |
| `cost_usd` | sum of the models' `costUSD` | API-equivalent cost of the run, **not billed** |
| `session_cost_usd` | `total_cost_usd` | The session total so far, not the run's |

`task_usage` holds `runs`, `cost_usd`, the four token kinds, `num_turns`, `duration_ms` and
`duration_api_ms`, summed over the task's counted runs.

Guest agents run on the subscription, so the cost is what the run would cost at list prices.
Use it to compare tasks, not as money spent.

The cache hit ratio is `cache_read / (input + cache_creation + cache_read)`, computed in integer
tenths of a percent and shown as, for example, `89.9%` (rounded down), or `n/a` with no input
tokens. Close to 100% means most of the context was served from the cache. A high
`cache_creation` with a low ratio means the prompt prefix was rebuilt, as on the first run of a
session.

### `agent-bus usage`

`agent-bus usage [--agent <guest>] [--host <name>]` prints one row per task: task, agent, runs,
cost, cache hit, tokens (all four kinds added) and final state, then a total row. It reads the
outcome messages sent to the host that the stream still keeps (7 days), with an ephemeral pull
consumer that needs no acknowledgement, so the host's own inbox is neither consumed nor
changed. The host's permissions already allow it (`$JS.API.>` creates the consumer and its pulls
go to the same API), so nothing was widened; a guest cannot do it. An ordered (push) consumer
would have needed `$JS.FC` publish rights for flow control, which is why a pull consumer is used.
The task's figures are the `task_usage` of its last message that carried one.

## Prompt cache

Measured (Claude Code 2.1.319): a new session's first request is byte-identical across sessions
except `metadata`. Claude Code's last cache breakpoint is at the end of the `# Environment`
block, which comes **after** the task text in the first user message. So every new task text is a
new prefix, and about 9.2k tokens are written again. Proof: two tasks with exactly the same text.
The first wrote 9,148 tokens and read 13,213 ($0.0380); the second wrote 0 and read 22,361
($0.0023).

Design: each new task is forked from a primed base session, so the task text comes after a
prefix that is already cached.

- **Base sessions.** The guest primes one with two turns, run the way tasks run: a fixed prompt
  ("You are a guest agent. Tasks will follow, one per session. Reply with exactly: ready."), then
  a resume of that session with "Confirm once more: reply with exactly: ready." There is one base
  per rules variant, GitHub and no-GitHub, because a fork keeps its base's system prompt
  (`--system-prompt-snapshot`). A base is primed lazily, on the first task that needs it.
- **Why two turns.** Measured live with one priming turn: a fork wrote about 3,770 tokens and
  read 18,775 ($0.0170, against $0.038 for a fresh session). That matches the second turn of any
  session: the first answer of task 562a9215 wrote 3,777, and its third turn wrote only 37.
  Claude Code seems to rearrange its context blocks after the first turn and settle after the
  second. So the base gets a second turn, and a fork is then a third. The third-turn saving
  (a fork writing about 37 tokens) is still to be confirmed.
- **Tasks** start with `--resume <base> --fork-session` and the task text as the new message.
  Answers resume the task's own forked session, as before. A base is never resumed directly.
- **Rebuilds.** A base is built again when its inputs change: the rules text, the hash of
  `AGENTS.md`, the git branch, `HEAD`, and the date (UTC day); or when it has been unused for 55
  minutes, so the 1 hour cache TTL has not run out. Each fork uses the cached prefix and so
  counts as a use. A stale base that slips through costs one cache write.
- **Stale context.** The base's git status is from when it was primed, so the rules say: "The git
  status in your context may be out of date; run git status yourself when it matters."
- **Accounting.** Both priming runs' usage is added to the `task_usage` of the task that triggered
  it (so `runs` counts them), and the guest log marks each with a "priming run" line. The record
  of result events keeps them as run 0 of that task. Whether a fork's `duration_api_ms` starts from
  its base's total is not measured; the run's API time is taken as reported, since a fork is a
  different session.
- **Fallback.** If the fork fails (no result, or a failed result with no turns, for example a
  missing base session) or the priming fails, the guest logs it, drops the base and runs the task
  in a fresh session, as before.

The tests are unit tests for the rebuild rules, the command lines and the priming accounting,
and ignored tests with a fake `claude` (`cargo test -p agent-bus -- --ignored`) for the fork,
the answer and the fallback. The saving itself needs a real Claude Code run to confirm.

## Running it

On the host, with the dev lab up:

1. Make a key for the host and one for each guest: `agent-bus key new`. It prints the public key
   and the seed. Store each seed in your secret store straight away; it is not shown again.
2. Write the agents file with the names, public keys and roles.
3. Start the bus for the dev lab, with the host's seed in `AGENT_BUS_SEED`, supplied from the
   secret store. `--lab` takes the lab directory and listens on the gateway in its
   `lab-info.toml`, so no address is written down. With the pass-cli env file
   `~/.config/mise/sherpa.env` (it holds `AGENT_BUS_HOST_SEED` and `AGENT_BUS_DEV_SEED`), the
   mise task runs:
   `pass-cli run --env-file ~/.config/mise/sherpa.env -- sh -c 'AGENT_BUS_SEED=$AGENT_BUS_HOST_SEED agent-bus bus start --lab <lab dir> --agents <file>'`.
4. Start the guest agent. `dev/agent-bus/start-guest.sh` runs in the guest, reads two lines from
   stdin, the Claude token and then the guest's seed (sent over SSH stdin, never as an argument), builds the binary and starts the agent in
   the background; its log is `~/agent-bus-dev.log`. From the host, pipe both to it over the
   lab's SSH config.

   Guests run Claude Code on `claude-opus-5-5`, the default of `agent-bus guest --model <id>`.
   To use another model, set `AGENT_BUS_MODEL=<id>` for `start-guest.sh`, or pass `--model`. The
   model is one of a base session's inputs, so a different model gets its own base and never
   forks one primed on another model. Guests sign commits with the Co-Authored-By line from
   their own attribution instructions, never one copied from the task text.
5. Listen for replies with `agent-bus listen host`, and send tasks with `agent-bus send`, both
   with `AGENT_BUS_SEED` set to the host's seed, as above. No address is needed: `send`,
   `listen` and `cancel` find the bus in this order, and stop at the first that is set:
   1. `--server <url>`
   2. the environment variable `AGENT_BUS_URL`
   3. the state file `bus start` wrote, `$XDG_STATE_HOME/agent-bus/bus.toml`

   If none is set they fail and say to run `bus start` or set one. The state file is the host's;
   the guest agent runs in a lab guest and still takes `--server`, which
   `dev/agent-bus/start-guest.sh` builds from the guest's default gateway.

The `nats` CLI can inspect the bus with `--nkey <seed file>`; that needs a seed on disk, so
prefer `agent-bus listen`, or a seed in a process substitution.

## Cancel and the queue

`agent-bus cancel --to <agent> <task>` sends a `cancel` message (`task` names the task). The guest
agent answers with `canceled` (after A2A's canceled state), or with `failed` when the task is
neither running nor queued.

While a task runs, the guest agent keeps reading its inbox, alongside Claude Code's output. Tasks
and answers that arrive meanwhile wait in a local queue and run afterwards, in arrival order. A
cancel is acted on at once:

- **Running task:** Claude Code is started in its own process group, and the group is signalled:
  SIGTERM, then SIGKILL after a grace period (`KILL_GRACE`, 2 s), and a final SIGKILL to the group
  for any command that ignored SIGTERM after Claude Code left. The commands it started (cargo,
  sleep, bash) die with it. `canceled` carries the last progress. The Claude session is kept, so a
  later answer to the task resumes it. The queue then goes on.
- **Queued task:** dropped from the queue; `canceled` is sent.
- **Unknown task:** `failed` with "neither running nor queued". This includes a task that already
  finished or is waiting for an answer.

The guest agent's own shutdown (SIGTERM or SIGINT) stops the running group the same way, reports
the task `failed` ("the guest agent stopped"), and exits, so restarting it does not leave Claude
Code running. Dropping the group on any other path kills it too. The signals come from the `nix`
crate. The tests are unit tests for the queue and cancel decisions, and ignored tests with a fake
`claude` (`cargo test -p agent-bus guest -- --ignored`).

## Limits of the proof of concept

- **The host can read and write every inbox,** and holds the only key that can change the
  stream. Compromising the host seed compromises the bus. A guest seed gives access to that
  guest's inbox and to sending to the host, so a compromised guest can flood the host's inbox.
- **No transport encryption.** NKeys authenticate, they do not encrypt. The bus is bound to one
  lab's bridge.
- **Removing an agent** means removing it from the agents file and running `bus start`, which
  drops its connection. Its consumer and messages stay in the stream.
- **One task at a time.** Others wait in the guest agent's memory queue, which a restart loses.
- **Sessions live in the guest agent's memory.** Restarting it loses them, so an answer to an
  earlier question fails, and the failure is reported to the host.
- **Starting the guest agent needs the user.** Claude Code's auto mode refuses to let the host
  agent start an agent that edits files and runs commands from network messages, unless a
  permission rule allows that one command.

## Next steps

1. More than one task at a time, and more than one guest.
2. Starting the guest agent with the guest, for example from its provisioning.
3. Several labs at once. The bus is one container on one address, so serving more labs needs a
   bus per lab (a container name, volume, port and state file for each), or a bus listening on
   more than one address.
4. Whether A2A belongs on top of the bus or replaces it, for agents that are not Claude Code
   (#439).

## Lost replies: two readers on one inbox

Symptom: `agent-bus listen host` got only some of a guest's messages (one progress, no
completed), or none, and a restarted listener got nothing redelivered. The guest logged
Completed and the bus logged no violations.

Cause: an inbox is one durable pull consumer, and every reader of a durable consumer shares its
messages. A second `listen host` that was never stopped (a leftover background listener) took
some messages, printed them where nobody looked and acknowledged them, so they were never
redelivered. Reproduced with two listeners on a throwaway bus: progress went to one, completed to
the other. Publishing, permissions, filter subject, ack policy and idle time were all ruled out:
a single reader got every message, also across a listener killed with `kill -9` and after 100 s
idle.

Fix: the consumers are created with `max_waiting = 1` (an older bus has them with the default; see below), and `Channel::inbox` refuses to open an inbox that already has a reader
waiting. A killed reader does not leave a stale claim. The ignored integration test checks that
every reply reaches the host listener, also after it restarts, and that a second reader is refused.

NATS cannot change `max_waiting` on an existing consumer (`bus start` failed with "max waiting can
not be updated", error code 10012). `bus start` now compares each existing inbox with the wanted
one: no difference is left alone, an updatable difference (the filter subject) is updated, and a
difference that cannot be updated deletes the consumer and creates it again with
`DeliverPolicy::ByStartSequence` at the old consumer's ack floor stream sequence + 1, read before
the delete. Acknowledged messages stay gone; delivered-but-unacknowledged and unread messages are
delivered. Each inbox is reported as created, updated, recreated from sequence N, or unchanged.
Limits: a message acknowledged out of order above the ack floor would be delivered again (an inbox
has one reader that acknowledges in order, so this does not occur), and a reader acknowledging
between the info read and the delete is replayed, not lost. If the create fails after the delete,
the error gives the sequence to create it from. The ignored integration test makes an inbox the
old way, acknowledges two messages, leaves one delivered and two unread, runs `bus start`, and
checks that the three are delivered once and the two do not return.
