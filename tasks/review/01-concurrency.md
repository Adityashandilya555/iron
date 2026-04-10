# Sequential Message Processing Blocks Multi-Tenant Concurrency

## Hypothesis

When user A is mid-session and user B sends a message (including a pairing accept at the server), B gets suspended until A finishes due to a sequential await in the main loop.

## Evidence (files + line numbers)

- `src/agent/agent_loop.rs:862-894` — Main loop: `tokio::select!` pulls one message, then `self.handle_message(&message).await` blocks until the response is fully generated and sent. No `tokio::spawn` around message handling.
- `src/agent/agent_loop.rs:1088-1167` — `handle_message()` runs submission parsing, hook evaluation, thread hydration, and delegates to `process_user_input` which runs the full agentic loop including LLM calls.
- `src/agent/agent_loop.rs:1350-1360` — `process_user_input` call within `handle_message` is the hot path running LLM and tool calls.
- `src/agent/agent_loop.rs:1378-1379` — After initial response, a drain loop processes queued messages from the same user, further extending the time the main loop is blocked.
- `src/channels/manager.rs:99-133` — `start_all()` merges all channel streams via `stream::select_all`. Messages from all users arrive in a single merged stream.
- `src/agent/session_manager.rs:29` — Sessions are `Arc<Mutex<Session>>` per user. The per-user lock exists but is irrelevant because the main loop never runs two messages concurrently.
- `src/agent/agent_loop.rs:335-376` — `tenant_ctx()` creates per-user `TenantCtx` with scoped DB, workspace, rate limiter. Multi-tenant infrastructure exists but is serialized by the main loop.
- `src/agent/CLAUDE.md:124` — "The agent loop is single-threaded per thread; parallel execution happens at the job/scheduler level." Confirms this is by design for single-user, but incompatible with multi-user Telegram.

## Root Cause

`src/agent/agent_loop.rs:894`: `match self.handle_message(&message).await` blocks the main `loop {}` (line 862) until the entire message-handling pipeline completes. The pipeline includes LLM API calls (3–15 seconds per turn). During that time, no other user's message is dequeued from the merged stream. The `ChannelManager` correctly merges streams and `SessionManager` correctly isolates per-user state behind `Arc<Mutex<Session>>`, but the serialization bottleneck is solely in the main loop's sequential await pattern.

**The user's hypothesis is exactly correct.**

## Required Changes (specific files and what to change)

### `src/agent/agent_loop.rs` (lines 862–950)
- **Current:** Main loop awaits `handle_message` synchronously, then sends response, then loops.
- **Required:** Spawn `handle_message` (and response sending) as a `tokio::spawn` task per message. The spawned task acquires the per-user session lock, ensuring same-user messages serialize while different users run concurrently. Add a `tokio::sync::Semaphore` (capacity ~50) to bound concurrent tasks. Extract the handler into a standalone async fn taking `Arc<AgentDeps>` + `Arc<ChannelManager>` + `Arc<SessionManager>` + `IncomingMessage`.
- **Change type:** refactor
- **Ripple:** `Agent` struct fields used by spawned tasks must be `Arc`-wrapped and `Send + Sync`. The shutdown signal (`Ok(None)` at line 946) needs a `CancellationToken` instead of breaking the main loop from inside a spawned task.

### `src/agent/agent_loop.rs` (lines 1378–1400, drain loop)
- **Current:** After processing user input, drains queued messages in a while loop, extending blocking time.
- **Required:** Move the drain loop into the spawned per-user task. It already runs under the session lock, so it naturally serializes for the same user.
- **Change type:** refactor (move, not rewrite)

### `src/agent/session_manager.rs`
- **No change needed.** The existing per-user `Arc<Mutex<Session>>` is the correct primitive for serializing same-user messages.

## Spec Impact (does this force a PERSONIFI_SPEC.md change?)

No. The spec already assumes multi-tenant operation (Phase 0 creates `TenantScope`). This change implements what the spec describes.

## Coordination Notes

- The database layer already supports per-user scoping via `TenantScope`. No DB changes needed.
- Test with two simulated Telegram users sending messages simultaneously after the change lands.
- The `/quit` shutdown mechanism needs rethinking — currently returns `Ok(None)` to break the main loop, but spawned tasks cannot break the parent loop. Use a shared `CancellationToken`.
