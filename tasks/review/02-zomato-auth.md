# Zomato Auth State Consumed Before Token Exchange Completes

## Hypothesis

`complete_auth` returns HTTP 500 "Token exchange failed". Retries fail with "OTP must be 4-8 digits" or "No pending Zomato auth found". Subsequent `start_auth` returns HTTP 500. Swiggy auth works.

## Evidence (files + line numbers)

- `src/tools/builtin/zomato_auth.rs:418` — `let pending = self.pending.lock().await.remove(user_id)` **consumes** the `PendingAuth` state at the start of `complete_auth`, before the token exchange at line 504.
- `src/tools/builtin/zomato_auth.rs:504-524` — Token exchange (`POST /token`) happens AFTER the pending state is removed. If this call fails (HTTP 500), the `PendingAuth` (containing `login_challenge`, `csrf_cookie`, `pkce_verifier`) is already gone.
- `src/tools/builtin/zomato_auth.rs:409-416` — OTP validation (`otp.len() < 4 || otp.len() > 8`) runs on the raw digit-filtered OTP. The validation itself is correct.
- `src/tools/builtin/zomato_auth.rs:213-214` — `sensitive_params()` returns `&["phone", "otp"]`. These are redacted in logs/SSE events via `redact_params()` but the original values ARE passed to `execute()`. Redaction is display-only and does NOT run before validation.
- `src/agent/dispatcher.rs:559` — `redact_params()` called for log display only. Does NOT affect the params passed to `tool.execute()`.
- `src/tools/builtin/zomato_auth.rs:318-335` — `login_challenge` extraction from `/authorize` redirect handles both absolute and relative (`./consent`) URLs (recent fix in commit `ecff34ac`).
- `.claude/skills/food-ordering/references/zomato-mcp.md:360` — Documents that `/token` requires `application/x-www-form-urlencoded`. Code at line 506-510 correctly uses `.form()`.
- `src/tools/builtin/zomato_auth.rs:424-429` — Session timeout check (`OTP_SESSION_TIMEOUT_SECS` = 600s). Already removed by line 418, so expiry cleanup is a no-op after the pending entry is consumed.

## Root Cause

**Primary bug:** `src/tools/builtin/zomato_auth.rs:418` — `self.pending.lock().await.remove(user_id)` is a destructive operation that removes the `PendingAuth` entry. If the subsequent `/verify-otp` (line 446) succeeds but `/token` (line 504) fails with HTTP 500, the `login_challenge`, `csrf_cookie`, and `pkce_verifier` are permanently lost. The user cannot retry `complete_auth` because the pending state is gone. They must restart the entire flow with `start_auth`.

**Secondary issue:** On retry, calling `start_auth` again generates a NEW PKCE challenge and calls `/authorize` again. If the Zomato server-side session from the first `/login` call is still active, the new `/authorize` may conflict with it, causing the HTTP 500 on the retry `start_auth`. This explains the cascading failure.

**Tertiary issue:** The SOUL.md guardrail "If auth fails 3 times, stop and tell the user to try the app directly" has no enforcement in code. There is no retry counter in `ZomatoAuthTool`.

**On the reported "OTP must be 4-8 digits" symptom on retry:** This is likely the LLM re-calling `complete_auth` after the pending state was consumed, receiving "No pending Zomato auth found" (line 419-422). The OTP length error would only occur if the LLM sends an OTP with non-digit characters that get filtered out, leaving fewer than 4 digits. Actual log review is needed to confirm.

## Required Changes (specific files and what to change)

### `src/tools/builtin/zomato_auth.rs` (line 418)
- **Current:** `self.pending.lock().await.remove(user_id)` consumes state at the start of `complete_auth`.
- **Required:** Clone/peek the state instead of removing it. Only call `.remove()` AFTER the token exchange at line 542 succeeds. On any failure in `/verify-otp` or `/token` steps, leave the pending state intact so the user can retry with a new OTP.
- **Change type:** refactor
- **Ripple:** `PendingAuth` already derives `Clone` — no struct changes needed.

### `src/tools/builtin/zomato_auth.rs` (new: retry counter)
- **Current:** No retry tracking.
- **Required:** Add a `retry_count: u8` field to `PendingAuth`. Increment on each failed `complete_auth`. After 3 failures, remove the pending state and return a message directing the user to the Zomato app. This enforces SOUL.md "If auth fails 3 times, stop."
- **Change type:** additive

### `src/tools/builtin/swiggy_auth.rs`
- **Required:** Audit for the same `.remove()` before token exchange pattern. Apply the same fix if present.
- **Change type:** audit + potential refactor

## Spec Impact (does this force a PERSONIFI_SPEC.md change?)

No. The spec (Phase 4, Step 4.0) describes the auth flow as "Phone + OTP in-context". The fix preserves this flow while making it retry-safe.

## Coordination Notes

- No database schema changes needed — `PendingAuth` is memory-only (in-process `HashMap`).
- SOUL.md "3 retries then stop" guardrail needs code enforcement, not just documentation.
- database-specialist task `tasks/db/05-auth-tokens.md` covers token storage once auth succeeds — this task only covers the OTP/CSRF flow.
