# Bootstrap Greeting vs Aria Identity — MODIFIED (Reopened)

**Status:** REOPENED — user reports Aria greeting still not appearing at runtime
**Changes from original:** Original marked "Status: Fixed" based on code analysis. User says default IronClaw greeting is still being sent. Reopened with deeper investigation areas.

---

## What the Original Task Found (Still Valid)

- `GREETING.md` contains the correct Aria greeting text (verified)
- `BOOTSTRAP_GREETING` is `include_str!("../workspace/seeds/GREETING.md")` — compiled in
- Two bootstrap paths exist: owner-bootstrap (line 438-463) and per-user bootstrap (line 1317-1345)
- `seed_if_empty()` sets `bootstrap_pending` flag when BOOTSTRAP.md is seeded

## What the Original Task Missed

The task concluded "issue is fixed" from static code analysis. But the user explicitly reports the Aria greeting is NOT being used at runtime. Possible causes not investigated:

### Hypothesis 1: Stale Binary

If the binary was compiled before `GREETING.md` was updated to the Aria greeting, `include_str!` baked in the old text. The source file shows the right content, but the running binary has the old version.

**Verification:** `strings <binary_path> | grep "Yo!"` — if no match, the binary is stale.

**Fix:** Recompile. `cargo build --release`.

### Hypothesis 2: `seed_if_empty()` Not Called for Telegram Users

`seed_if_empty()` (workspace/mod.rs:1624) only runs when `is_fresh_workspace` is true. The condition (line 1639-1650):
```rust
let is_fresh_workspace = if self.read_primary(paths::BOOTSTRAP).await.is_ok() {
    false  // BOOTSTRAP already exists — NOT fresh
} else {
    // All three must be missing
    agents_res.is_err() && soul_res.is_err() && user_res.is_err()
};
```

If any of AGENTS.md, SOUL.md, or USER.md already exist (from a prior deployment or partial seed), `is_fresh_workspace` = false, and BOOTSTRAP.md is never created, so `bootstrap_pending` is never set, and the greeting is never sent.

**Scenario:** User's workspace was partially seeded in a prior run (AGENTS.md exists from an earlier IronClaw version). New run sees AGENTS.md exists → `is_fresh_workspace = false` → no greeting.

**Verification:** Check `memory_documents` table for the user: `SELECT path FROM memory_documents WHERE user_id = '<telegram_user_id>'`. If AGENTS.md exists but GREETING.md / BOOTSTRAP.md don't, this is the cause.

### Hypothesis 3: Owner Bootstrap Path Wins, Per-User Path Skipped

`Agent::run()` (line 438-463) checks `ws.take_bootstrap_pending()` on the OWNER workspace. If the owner workspace is fresh, this consumes the flag. Then when a Telegram user sends a message, `tenant_ctx()` creates their per-user workspace and calls `seed_if_empty()` — but if the owner workspace's `take_bootstrap_pending()` already consumed the global flag, the per-user path at line 1322 might not fire.

**Key:** `take_bootstrap_pending()` uses `AtomicBool::swap(false, Ordering::AcqRel)` (workspace/mod.rs:468). The per-user workspace has its OWN `bootstrap_pending` flag (it's a field on the `Workspace` struct, not global). So this should NOT be the issue — but verify that `tenant_ctx()` creates a genuinely separate `Workspace` instance with its own flag.

**Verification:** Add `tracing::debug!` in `seed_if_empty()` to log `is_fresh_workspace` and `bootstrap_pending` for the Telegram user's workspace.

### Hypothesis 4: Per-User Workspace Not Scoped Correctly

`tenant_ctx()` (agent_loop.rs:335-376) calls `ws.scoped_to_user(&message.user_id)`. If `scoped_to_user` returns a workspace that shares state with the owner workspace (including the `bootstrap_pending` flag), the owner bootstrap at line 441 consumes it before the per-user path fires.

**Verification:** Read `src/workspace/mod.rs` — `scoped_to_user()` implementation. Check if it creates a new `Workspace` with a fresh `bootstrap_pending = AtomicBool::new(false)` or inherits from the parent.

## Required Changes

### Immediate: Investigate at Runtime

1. Add debug logging in `seed_if_empty()` to trace `is_fresh_workspace` decision
2. Add debug logging in the per-user bootstrap path (line 1322-1345)
3. Run with `RUST_LOG=ironclaw::workspace=debug,ironclaw::agent=debug`
4. Send `/start` from a fresh Telegram account
5. Check logs for: "Fresh user workspace — persisting bootstrap greeting" (line 1327)

### If Hypothesis 2 (partial seed): Fix `seed_if_empty()` condition

The `is_fresh_workspace` check is too strict. If AGENTS.md exists but USER.md doesn't, the user hasn't completed onboarding — they should still get the greeting.

**Proposed fix:** Change `is_fresh_workspace` to check USER.md only (not all three):
```rust
let needs_greeting = self.read_primary(paths::BOOTSTRAP).await.is_err()
    && self.read_primary(paths::USER).await
        .map(|content| content.contains("name:") && !content.contains("name: \n"))
        .unwrap_or(false) == false;
```

Logic: Send greeting if BOOTSTRAP.md doesn't exist AND USER.md either doesn't exist or has no name filled in.

### If Hypothesis 4 (shared flag): Fix `scoped_to_user()`

Ensure `scoped_to_user()` creates a workspace with its own `bootstrap_pending` flag. The scoped workspace should call `seed_if_empty()` independently.

### Owner Bootstrap Cleanup (from original task, still valid)

Remove the owner-bootstrap path (lines 438-463) or gate it behind a check for single-user mode. For Telegram-first deployments, the per-user path (lines 1317-1345) is the correct one. The owner path sends to `"gateway"` channel which is not Telegram.

## Files That Need Changes

| File | Change | Type |
|---|---|---|
| `src/workspace/mod.rs:1624-1697` | Debug logging in `seed_if_empty()` + potentially relax `is_fresh_workspace` condition | investigate + potential fix |
| `src/workspace/mod.rs` | Check `scoped_to_user()` for flag isolation | investigate |
| `src/agent/agent_loop.rs:438-463` | Consider removing owner-bootstrap path | optional cleanup |
| Binary | Recompile to ensure `GREETING.md` changes are baked in | deploy step |

## Coordination Notes

- This is a runtime investigation task. Cannot be resolved by code reading alone.
- Requires a fresh Telegram test account (or clearing the user's `memory_documents` rows).
- The onboarding skill (`skills/onboarding/SKILL.md`) expects the greeting to have already been sent and the first message to be the user's name. If greeting isn't sent, onboarding breaks.
