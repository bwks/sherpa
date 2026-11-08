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
| Bus | NATS 2.15 with JetStream, in the container `sherpa-agent-bus` on the host (`agent-bus bus start --listen <address> --agents <file>`), on the host's network. It listens on one address: the dev lab's management bridge, which is the guest's default gateway. Stored messages live in the volume `sherpa-agent-bus-data` |
| Inboxes | One stream, `AGENTS`, on `agents.>`. Each agent reads `agents.<name>` through a durable consumer, so a message waits until its agent reads it, and a restarted reader goes on where it stopped. The host creates the stream and the consumers when it starts the bus |
| Messages | JSON: task ID, sender, receiver, kind and text. The kinds are `task`, `answer`, `progress`, `question`, `completed` and `failed`, named after A2A's task states |
| Host agent | `agent-bus send --to dev "<task>"` prints the task ID; `--answer <task>` answers a question. `agent-bus listen host`, run under Claude Code's Monitor, turns each message into a notification |
| Guest agent | `agent-bus guest` runs each task through `claude -p` with `stream-json` output in the checkout, one at a time. Tool calls and text become progress; a reply line starting `QUESTION:` makes the outcome a question; an answer resumes the task's Claude session |

### Guest rules

- The guest agent uses the Claude subscription: `CLAUDE_CODE_OAUTH_TOKEN` from
  `claude setup-token`, given on stdin when it starts. `ANTHROPIC_API_KEY` is removed from
  Claude Code's environment, so it can never fall back to API billing.
- Claude Code runs with `--dangerously-skip-permissions`: permission checks are skipped
  because the guest is a disposable lab VM where the agent has root. `git commit`, `git push`,
  `git reset` and `gh` are still refused (`--disallowedTools`), so committing, pushing and
  pull requests stay with the host agent. Its instructions say to follow `AGENTS.md` and to ask
  instead of adding dependencies.

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

`agent-bus bus start --listen <address> --agents <file>`, with the host's seed in
`AGENT_BUS_SEED`:

1. Read and validate the agents file, and check the seed belongs to the host's key.
2. Write the config to `$XDG_STATE_HOME/agent-bus/nats.conf` (default
   `~/.local/state/agent-bus/`), outside the repository, mode 0644 in a 0755 directory so the
   container's user can read it. It is rewritten on every start.
3. Pull the pinned image if needed, then **replace** the container: stop and remove the old one
   if present, and create it with the config bind-mounted read-only (`-c`). The config can change,
   so an existing container is never just started. Messages live in the volume, so replacing
   is safe.
4. Wait for the server, connect as the host, and create the `AGENTS` stream and a durable consumer
   for every agent in the file, the host included (it reads its replies through one). Only the
   host may do this. The other agents bind to their consumer and fail with a clear error if it is
   missing, for example when the agent is not in the agents file the bus was started with.

Adding an agent: generate a key, add it to the agents file, and run `bus start` again.

## Usage

Each finished run records what Claude Code reported in its `result` event, so the host can see
how efficient a task was. The outcome messages (`completed`, `question`, `failed`) carry it as
`usage`; progress messages and failures that never reached a result (Claude Code did not start,
or exited without one) carry none. The guest log also gets one line per finished run.

| Field | Claude Code field | Meaning |
|---|---|---|
| `input` | `usage.input_tokens` | Input tokens not read from or written to the cache |
| `output` | `usage.output_tokens` | Output tokens |
| `cache_creation` | `usage.cache_creation_input_tokens` | Input tokens written to the prompt cache (a miss) |
| `cache_read` | `usage.cache_read_input_tokens` | Input tokens read from the prompt cache (a hit) |
| `num_turns` | `num_turns` | Model turns in the run |
| `duration_ms` | `duration_ms` | Wall-clock time |
| `duration_api_ms` | `duration_api_ms` | Time in API calls |
| `total_cost_usd` | `total_cost_usd` | API-equivalent cost, **not billed** |

Guest agents run on the subscription, so `total_cost_usd` is what the run would cost at API
prices. Use it to compare tasks, not as money spent.

The cache hit ratio is `cache_read / (input + cache_creation + cache_read)`, computed in integer
tenths of a percent (`cache_hit_permille`) and logged as, for example, `89.9%` (rounded down). Close to 100% means
most of the context was served from the cache. A high `cache_creation` with a low ratio means the
prompt prefix was rebuilt, as on the first run of a session. A run that resumes a session after
an answer should hit the cache for the earlier context unless the cache expired. The log line
shows `n/a` when a run had no input tokens.

The field names were checked against Claude Code 2.1.319, but only from a run that was not
logged in, so its counts were all zero. `modelUsage` (per model) was empty there, so it is not
recorded. The parser tests use that captured event and a hand-written one with cache counts.

## Running it

On the host, with the dev lab up:

1. Make a key for the host and one for each guest: `agent-bus key new`. It prints the public key
   and the seed. Store each seed in your secret store straight away; it is not shown again.
2. Write the agents file with the names, public keys and roles.
3. Start the bus on the dev lab's management bridge, `ipv4_gateway` in the lab's
   `lab-info.toml`, with the host's seed in `AGENT_BUS_SEED`, supplied from the secret store. With the pass-cli env
   file `~/.config/mise/sherpa.env` (it holds `AGENT_BUS_HOST_SEED` and `AGENT_BUS_DEV_SEED`):
   `pass-cli run --env-file ~/.config/mise/sherpa.env -- sh -c 'AGENT_BUS_SEED=$AGENT_BUS_HOST_SEED agent-bus bus start --listen <address> --agents <file>'`.
4. Start the guest agent. `dev/agent-bus/start-guest.sh` runs in the guest, reads two lines from
   stdin, the Claude token and then the guest's seed (sent over SSH stdin, never as an argument), builds the binary and starts the agent in
   the background; its log is `~/agent-bus-dev.log`. From the host, pipe both to it over the
   lab's SSH config.
5. Listen for replies with `agent-bus listen --server nats://<address>:4222 host`, and send tasks
   with `agent-bus send`, both with `AGENT_BUS_SEED` set to the host's seed, as above.

The `nats` CLI can inspect the bus with `--nkey <seed file>`; that needs a seed on disk, so
prefer `agent-bus listen`, or a seed in a process substitution.

## Limits of the proof of concept

- **The host can read and write every inbox,** and holds the only key that can change the
  stream. Compromising the host seed compromises the bus. A guest seed gives access to that
  guest's inbox and to sending to the host, so a compromised guest can flood the host's inbox.
- **No transport encryption.** NKeys authenticate, they do not encrypt. The bus is bound to one
  lab's bridge.
- **Removing an agent** means removing it from the agents file and running `bus start`, which
  drops its connection. Its consumer and messages stay in the stream.
- **One task at a time,** and no way to cancel one.
- **Sessions live in the guest agent's memory.** Restarting it loses them, so an answer to an
  earlier question fails, and the failure is reported to the host.
- **Starting the guest agent needs the user.** Claude Code's auto mode refuses to let the host
  agent start an agent that edits files and runs commands from network messages, unless a
  permission rule allows that one command.

## Next steps

1. Cancelling a task, and more than one guest.
2. Starting the guest agent with the guest, for example from its provisioning.
3. Whether A2A belongs on top of the bus or replaces it, for agents that are not Claude Code
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
