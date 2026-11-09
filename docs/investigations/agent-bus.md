# Agent bus: a message channel between the host agent and guest agents

> How the agent bus works now (components, messages, flows, permissions, guest execution, usage,
> files and how to run it) is in [`../architecture/AGENT_BUS.md`](../architecture/AGENT_BUS.md).
> This file keeps what was tried and measured on the way.
>
> Status: proof of concept, working. Tracked in #494. Follows from #439 (A2A), which stays open
> for the protocol question. The code is the `agent-bus` crate (`crates/agent-bus`), a development
> tool that is not released.

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

## What Claude Code's result figures mean

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
session's $0.0582, which is how the per-run and per-session fields were told apart. The
per-model entries had `costBasis` `list`. How the guest turns these into `usage` and
`task_usage` is in the architecture document.

Not measured: whether a fork's `duration_api_ms` starts from its base's total. The run's API time
is taken as reported, since a fork is a different session.

## Prompt cache

Measured (Claude Code 2.1.319): a new session's first request is byte-identical across sessions
except `metadata`. Claude Code's last cache breakpoint is at the end of the `# Environment`
block, which comes **after** the task text in the first user message. So every new task text is a
new prefix, and about 9.2k tokens are written again. Proof: two tasks with exactly the same text.
The first wrote 9,148 tokens and read 13,213 ($0.0380); the second wrote 0 and read 22,361
($0.0023).

That led to forking each new task from a primed base session.

- **One priming turn was not enough.** Measured live with a base primed by one turn: a fork wrote
  about 3,770 tokens and read 18,775 ($0.0170, against $0.038 for a fresh session). That matches
  the second turn of any session: the first answer of task 562a9215 wrote 3,777, and its third
  turn wrote only 37. Claude Code seems to rearrange its context blocks after the first turn and
  settle after the second. So the base got a second turn, and a fork is a third.
- **Two priming turns.** Measured afterwards (commit 776ae6f7): a fork writes about 35 tokens, and
  a small task fell from about $0.038 to about $0.0025 on Sonnet (API-equivalent).

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

Fix: `max_waiting = 1` on the consumers, and a second reader is refused (see the architecture
document, "One reader per inbox").

Upgrading an existing bus then failed: NATS cannot change `max_waiting` on an existing consumer
(`bus start` failed with "max waiting can not be updated", error code 10012). `bus start` now
recreates such a consumer from its ack floor. Limits found while designing it: a message
acknowledged out of order above the ack floor would be delivered again (an inbox has one reader
that acknowledges in order, so this does not occur), and a reader acknowledging between the info
read and the delete is replayed, not lost. The ignored integration test makes an inbox the old
way, acknowledges two messages, leaves one delivered and two unread, runs `bus start`, and checks
that the three are delivered once and the two do not return.

## Restarts: two agents on one state file

Found in a live test: `start-guest.sh` sent `pkill` and started the new agent at once, so the new
one loaded `running` from the state file before the old one, still stopping, had cleared it. The
host got two failures for one task ("restarted while this task ran" from the new agent, then "the
task was interrupted" from the old). This led to the instance lock and to `start-guest.sh`
waiting for the old agent to exit.

A later live check found the lock was taken only after connecting: a second agent pointed at a
bad server failed with "Connection refused" instead of the lock error. The lock now comes before
the connection, so a duplicate fails at once and never touches the bus.
