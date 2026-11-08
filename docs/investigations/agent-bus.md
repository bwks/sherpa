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

## What was proven (2026-11-08)

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
| Bus | NATS 2.15 with JetStream, in the container `sherpa-agent-bus` on the host (`agent-bus bus start --listen <address>`), on the host's network. It listens on one address: the dev lab's management bridge, which is the guest's default gateway. Stored messages live in the volume `sherpa-agent-bus-data` |
| Inboxes | One stream, `AGENTS`, on `agents.>`. Each agent reads `agents.<name>` through a durable consumer, so a message waits until its agent reads it, and a restarted reader goes on where it stopped |
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

1. Start the bus on the dev lab's management bridge, `ipv4_gateway` in the lab's
   `lab-info.toml`: `agent-bus bus start --listen <address>`.
2. Start the guest agent. `dev/agent-bus/start-guest.sh` runs in the guest, reads the token from
   stdin, builds the binary and starts the agent in the background; its log is
   `~/agent-bus-dev.log`. From the host, pipe the token to it over the lab's SSH config.
3. Listen for replies with `agent-bus listen --server nats://<address>:4222 host`, and send tasks
   with `agent-bus send`.

`nats --server nats://<address>:4222 server check connection` checks the bus, and
`nats stream view AGENTS` shows the stored messages.

## Limits of the proof of concept

- **No authentication.** Anything that reaches the bus address can read and write every inbox.
  Binding it to one lab's bridge keeps that to that lab and the host.
- **One task at a time,** and no way to cancel one.
- **Sessions live in the guest agent's memory.** Restarting it loses them, so an answer to an
  earlier question fails, and the failure is reported to the host.
- **Starting the guest agent needs the user.** Claude Code's auto mode refuses to let the host
  agent start an agent that edits files and runs commands from network messages, unless a
  permission rule allows that one command.

## Next steps

1. Authentication: a NATS user per agent, allowed only its own inbox and the host's.
2. Cancelling a task, and more than one guest.
3. Starting the guest agent with the guest, for example from its provisioning.
4. Whether A2A belongs on top of the bus or replaces it, for agents that are not Claude Code
   (#439).
