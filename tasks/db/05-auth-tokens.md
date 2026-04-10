# Auth Tokens: Swiggy (Working) vs Zomato (Broken) — Data Model Analysis

**Status:** ANALYSIS
**Area:** auth, secrets
**Relates to:** tasks/review/02-zomato-auth.md (product-review agent owns OTP/CSRF root cause)

## Current State

**Where it lives today:**

- `secrets` table (PG: `migrations/V2__wasm_secure_api.sql`, libSQL: `src/db/libsql_migrations.rs:289`) — `id, user_id, name, encrypted_value (AES-256-GCM), key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at`. Unique constraint on `(user_id, name)`.
- `SecretsStore` trait (`src/secrets/store.rs:25`) — CRUD + expiry checking + usage tracking + access control. Implementations: `PostgresSecretsStore`, `LibSqlSecretsStore`, `InMemorySecretsStore`.
- `SecretsCrypto` (`src/secrets/crypto.rs`) — AES-256-GCM encryption with per-secret HKDF key derivation. Master key from OS keychain.
- `SwiggyAuthTool` (`src/tools/builtin/swiggy_auth.rs`) — Stores tokens under: `mcp_swiggy-food_access_token`, `mcp_swiggy-instamart_access_token`, `mcp_swiggy-dineout_access_token`. One Swiggy auth covers all three. Pending auth state (PKCE verifier, session) held in-memory HashMap, keyed by user_id. Ephemeral — lost on restart.
- `ZomatoAuthTool` (`src/tools/builtin/zomato_auth.rs`) — Stores token under `mcp_zomato-mcp-server_access_token`. Same in-memory pending state pattern. HTTP 500 failure is in the auth flow itself (CSRF/cookie handling), **not in token storage**.
- `McpServerConfig::token_secret_name()` (`src/tools/mcp/config.rs:248`) — Generates key name as `mcp_{server_name}_access_token`. Also `refresh_token_secret_name()` = `{token_secret_name}_refresh_token`.
- MCP client token injection — Extension manager resolves `token_secret_name` from SecretsStore and injects it as `Authorization: Bearer <token>` in MCP HTTP calls.

**Lifecycle:**
- **Write:** `swiggy_auth` calls `secrets.create(user_id, CreateSecretParams::new("mcp_swiggy-food_access_token", token_value))` after successful OTP verification. UPSERT semantics (ON CONFLICT DO UPDATE).
- **Read:** MCP client reads `secrets.get_decrypted(user_id, "mcp_swiggy-food_access_token")` before each MCP call.
- **Expiry:** `secrets.expires_at` column exists but **neither `swiggy_auth` nor `zomato_auth` sets it**. Tokens are stored with `expires_at = None` (never expires). The system discovers token expiry only when an MCP call fails.
- **Refresh:** No refresh token is stored by either auth tool. `refresh_token_secret_name()` exists in MCP config but is used only by the generic OAuth flow in `ExtensionManager`, not by the custom Swiggy/Zomato auth tools. Re-auth requires a full OTP flow.
- **Deletion:** No automatic cleanup. Tokens persist indefinitely.

**What works:**
1. Encryption at rest — AES-256-GCM with per-secret salt. Master key in OS keychain. Meets SOUL.md requirement.
2. User isolation — secrets keyed by `(user_id, name)`. User A cannot access user B's tokens.
3. UPSERT semantics — re-auth overwrites the old token cleanly.
4. Usage tracking — `last_used_at` and `usage_count` updated on each use.
5. Swiggy auth flow works end-to-end.

## Gaps and Risks

- **No expiry set on stored tokens.** Tokens stored with `expires_at = None`. System discovers expiry only on MCP call failure. No proactive expiry check.
- **No refresh token stored.** Every re-auth requires a full OTP flow requiring user interaction. Acceptable per spec ("No background refresh") but token expiry is disruptive when it happens.
- **Auth failure count not tracked.** SOUL.md: "If auth fails 3 times, stop." No counter in database for auth failures. Currently tracked in session memory (ephemeral), lost on restart.
- **Pending auth state is in-memory only.** If server restarts between `start_auth` (OTP sent) and `complete_auth` (user enters OTP), PKCE state is lost. User must restart. Acceptable but should be documented.
- **Zomato auth failure is not a storage issue.** The HTTP 500 is in the `/authorize` → `/login` → `/verify-otp` → `/token` HTTP flow. product-review agent owns this (tasks/review/02-zomato-auth.md). However: if a partial/corrupted Zomato token was ever stored from a previous attempt, it could cause MCP calls to fail with auth errors, prompting re-auth, which then fails again. A `last_verified_at` field would help distinguish "token stored but never verified" from "token confirmed working."
- **No unified view of auth status across platforms.** To show "Swiggy: connected, Zomato: needs setup," the system must check 4 separate secret keys.

## Proposed Schema

The existing `secrets` table is sufficient for token storage. **No new tables for token values.** Propose conventions and one lightweight tracking table.

### Convention: Token naming (existing, do not change)

| Platform | Secret name | Notes |
|---|---|---|
| Swiggy Food | `mcp_swiggy-food_access_token` | Already in use |
| Swiggy Instamart | `mcp_swiggy-instamart_access_token` | Already in use |
| Swiggy Dineout | `mcp_swiggy-dineout_access_token` | Already in use |
| Zomato | `mcp_zomato-mcp-server_access_token` | Already in use |

Generated by `McpServerConfig::token_secret_name()`. Do not change.

### Enhancement: Use existing `secrets` columns

- **`provider`:** Set to `"swiggy"` or `"zomato"` when storing tokens. Currently NULL.
- **`expires_at`:** Set based on platform token TTL. If the platform does not return an expiry, set a conservative default (e.g., 7 days) and trigger re-auth on expiry. Swiggy tokens appear to be long-lived (weeks/months) — empirical measurement needed.

### New: `auth_status` tracking table

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| user_id | TEXT | NO | | |
| platform | TEXT | NO | | `swiggy` / `zomato` |
| status | TEXT | NO | | `connected` / `expired` / `failed` / `never_connected` |
| last_verified_at | TEXT | YES | | Last time a lightweight API call confirmed token works |
| consecutive_failures | INTEGER | NO | DEFAULT 0 | For the 3-failure SOUL.md rule |
| last_failure_reason | TEXT | YES | | |
| updated_at | TEXT | NO | DEFAULT now | |
| PRIMARY KEY(user_id, platform) | | | | |

**SOUL.md compliance:** "If auth fails 3 times, stop." → `consecutive_failures >= 3` check before attempting auth. Tokens remain encrypted in `secrets` table; this table holds only metadata.

**Sample rows:**
```
("tg:123456789", "swiggy", "connected", "2026-04-09T08:00:00Z", 0, null, ...)
("tg:123456789", "zomato", "failed", null, 3, "HTTP 500 during /authorize", ...)
```

## Migration Strategy

1. Create `auth_status` table (both PG and libSQL).
2. Backfill: for each user, check if `mcp_swiggy-food_access_token` exists in `secrets` → set `auth_status(user_id, 'swiggy', 'connected')`. Same for Zomato.
3. Update `swiggy_auth` and `zomato_auth` tools to:
   - Set `provider` field when storing tokens.
   - Set `expires_at` to a conservative TTL (or actual TTL if returned by platform).
   - Update `auth_status` on success (reset `consecutive_failures`, set `status = 'connected'`).
   - Update `auth_status` on failure (increment `consecutive_failures`, set `last_failure_reason`).
   - Check `consecutive_failures >= 3` before attempting auth; return "try the app directly" if exceeded.
4. Do NOT change the `secrets` table schema. It already supports everything needed.

**Files that will need code changes:**
- `src/db/mod.rs` — new `AuthStatusStore` sub-trait
- `src/db/postgres.rs`, `src/db/libsql/` — implement
- `migrations/` + `src/db/libsql_migrations.rs` — `auth_status` table
- `src/tools/builtin/swiggy_auth.rs` — set `provider`, `expires_at` on token create; update `auth_status`
- `src/tools/builtin/zomato_auth.rs` — same (after tasks/review/02-zomato-auth.md root cause is fixed)

## SOUL.md / Spec Compliance Notes

- PERSONIFI_SPEC Phase 4, Step 4.0: "Check `mcp_swiggy-food_access_token` in SecretsStore. If valid: skip auth." — `auth_status.status = 'connected'` enables this check without decrypting the token.
- PERSONIFI_SPEC Resolved Decision 5: "No background refresh." — `auth_status` enables expiry detection without background jobs; re-auth is triggered on user action.
- SOUL.md: "If auth fails 3 times, stop." — `consecutive_failures` field enforces this in code, not just as a prompt instruction.

## Coordination Notes

- **product-review agent owns the Zomato OTP/CSRF root cause** — see tasks/review/02-zomato-auth.md. This task covers token storage once auth is fixed. The `auth_status` table is useful regardless of whether Zomato auth is broken or working.
- The phone-in-SecretsStore migration from tasks/db/02-user-identity.md should be done **before** updating the auth tools to read phone from SecretsStore. The two changes must be coordinated.

## Open Questions

1. **What is the actual TTL of Swiggy tokens?** Empirical measurement needed with a real account. Until known, use 7-day conservative default.
2. **Should `auth_status` be in the `secrets` table as metadata, or a separate table?** The `secrets` table has no metadata column. A separate table is cleaner and does not couple the general-purpose secrets schema to Personifi-specific auth state.
3. **Pending auth state persistence:** Currently in-memory, lost on restart. Should it be persisted in a `pending_auth_sessions` table? The OTP window is 10 minutes, so the restart-loss window is small. Not worth the complexity for MVP — but note it as a known gap.
