# PRD: Persistent Job State for HTML Progress Operations

## Status

Draft backlog item.

## Problem

The browser UI uses `AppState.pending_jobs`, an in-memory `DashMap`, to hand off create/destroy operations from an HTML form POST to a job progress page and SSE stream. This design is simple, but jobs do not survive process restart and can be consumed only once. If a user reloads, loses connection, or opens the stream from multiple tabs, the job can become unavailable even though the user still expects progress or a final result.

The current design also means the operation does not actually start until the SSE stream handler consumes the pending job.

## Findings

- `AppState.pending_jobs` is `Arc<DashMap<String, Job>>` in `daemon/state.rs`.
- `Job` currently contains only `lab_name` and `JobType`.
- `JobType` currently supports create and destroy requests.
- `lab_create_post_handler` inserts a pending create job and redirects to `/jobs/{job_id}`.
- `lab_destroy_post_handler` inserts a pending destroy job and redirects to `/jobs/{job_id}`.
- `job_page_handler` only peeks at the in-memory pending job to render display metadata.
- `job_stream_handler` removes the job from `pending_jobs`; after removal the job is considered consumed.
- If the server restarts before stream consumption, the job is gone.
- If the stream endpoint is opened after the job has already been consumed, the user receives a "Job not found or already consumed" style error.
- Progress events are transient channel messages and are not stored.

## Goals

- Make UI job state durable enough to survive server restart.
- Allow reconnecting to an in-progress or completed job page.
- Store final result and enough recent progress/status to explain what happened.
- Avoid accidentally running the same destructive job twice.
- Preserve the simple browser UX: submit form, redirect to progress page, stream updates.

## Non-goals

- Building a distributed task queue.
- Supporting multiple sherpad instances coordinating jobs in the first iteration.
- Persisting every low-level progress event forever.
- Changing REST/WebSocket streaming semantics for non-HTML clients.

## Proposed solution

Replace `pending_jobs` with persistent job records in SurrealDB and a server-side job runner abstraction.

Recommended job model:

| Field | Purpose |
|---|---|
| `job_id` | Stable UUID returned to UI. |
| `kind` | Create, destroy, redeploy, image import, etc. Start with create/destroy. |
| `status` | Pending, running, succeeded, failed, cancelled, abandoned. |
| `owner_username` | User who created the job. |
| `lab_id` | Target lab when applicable. |
| `lab_name` | Display label. |
| `request_payload` | Serialized request needed to execute or audit the job. |
| `created_at`, `started_at`, `finished_at` | Lifecycle timestamps. |
| `last_heartbeat_at` | Helps detect abandoned running jobs after process crash. |
| `progress_tail` | Bounded recent progress messages for reconnect. |
| `result_payload` | Serialized final result or error summary. |

Execution model:

1. Form POST creates a durable pending job record.
2. Job runner starts the operation exactly once.
3. SSE clients subscribe to progress by `job_id`.
4. Progress is both streamed live and appended to a bounded persisted tail.
5. Final result is persisted.
6. Reconnects replay job metadata, recent progress, and final result if complete.

## Functional requirements

- Creating a UI job must persist a job record before redirecting to `/jobs/{job_id}`.
- A job must have an owner and authorization checks on page/stream access.
- Refreshing `/jobs/{job_id}` must work after the SSE stream has started.
- Reconnecting to `/jobs/{job_id}/stream` must not start a duplicate operation.
- Completed jobs must show their final result after stream completion.
- Failed jobs must show a useful persisted error summary.
- Server restart must not lose pending/completed job records.
- Running jobs interrupted by process restart must be marked abandoned or recoverable, not silently pending forever.

## Non-functional requirements

- Job persistence must not store secrets unnecessarily.
- Progress history must be bounded to avoid unbounded database growth.
- Destructive operations must be idempotent or protected by a single-run lease.
- Job records should have a retention/cleanup policy.
- SSE behavior should remain responsive and not depend on a database write for every emitted line if that becomes too slow.

## Action plan

### Phase 1: Data model and schema

- Add a SurrealDB job table/schema.
- Define job status enum in shared/server data models as appropriate.
- Store serialized request/result payloads as TOML-compatible data models where config-like persistence is involved; database serialization can remain structured records.
- Add DB CRUD helpers for create, claim/start, append progress, finish, fail, and list jobs by owner.

### Phase 2: Job runner abstraction

- Create a server-side job runner that claims a pending job and executes it.
- Use an atomic claim/update so two streams cannot run the same job twice.
- Decouple operation start from SSE stream connection where practical.
- Add a cancellation/abandonment strategy for server shutdown.

### Phase 3: SSE reconnect behavior

- Update job page handler to load job metadata from DB.
- Update stream handler to subscribe to live progress if running.
- Replay persisted progress tail before live progress.
- If job is complete, return final completion event immediately.

### Phase 4: Retention and operator visibility

- Add retention policy for completed jobs.
- Add admin visibility into failed/abandoned jobs if useful.
- Add cleanup task or maintenance action for old job records.

## Testing plan

- Unit tests for job state transitions.
- DB tests for job create/claim/finish/fail semantics.
- Integration tests for page refresh after stream start.
- Integration tests for duplicate stream connections not duplicating execution.
- Restart simulation test: pending/running jobs are recovered or marked abandoned according to policy.
- Authorization tests: users cannot view/stream another user's job unless admin.

## Acceptance criteria

- `AppState.pending_jobs` is removed or no longer used as the source of truth.
- HTML create/destroy jobs survive process restart as DB records.
- Reconnecting to a job page works after stream consumption.
- Duplicate stream connections do not duplicate operations.
- Final job result is persisted and viewable after completion.
- Abandoned running jobs are clearly marked after server restart.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Durable jobs cause duplicate destructive operations | Use atomic job claim and idempotency guards. |
| Progress writes overload SurrealDB | Persist a bounded tail and throttle writes if needed. |
| Request payloads contain sensitive data | Audit payloads and redact secrets before persistence. |
| Restart recovery semantics are ambiguous | Explicitly mark interrupted running jobs as abandoned in the first iteration. |
