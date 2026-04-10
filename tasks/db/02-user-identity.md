# User Identity and Phone Storage — Data Model Analysis

**Status:** ANALYSIS
**Area:** identity, auth, privacy
**Relates to:** tasks/db/01-user-prefs.md (preferences table), tasks/db/05-auth-tokens.md (phone used for re-auth)

## Current State

**Where it lives today:**

- `migrations/V14__users.sql` — `users` table: `id TEXT PRIMARY KEY`, `email`, `display_name`, `status`, `role`, `metadata JSONB`. Also in `src/db/libsql_migrations.rs`. Designed for IronClaw multi-user web deployments, **not** for Telegram-to-IronClaw mapping.
- `src/db/mod.rs:316` — `UserRecord` struct: `id: String`, `email: Option<String>`, `display_name: String`, `status: String`, `role: String`. No phone field. No telegram_id field.
- `src/channels/channel.rs:75-79` — `IncomingMessage` has `user_id: String`, `owner_id: String`, `sender_id: String`. For Telegram, the WASM channel sets these from the Telegram chat/user ID (raw numeric string).
- `src/workspace/seeds/USER.md` — Has `telegram_id:` and `phone:` fields. Both stored as **plaintext markdown** in `memory_documents.content`.
- `skills/onboarding/SKILL.md` — Q2 captures phone, writes to USER.md as `phone: {digits}`.
- `src/tools/builtin/swiggy_auth.rs:8-9` — `start_auth(phone, country_code)` reads phone from LLM context (which got it from USER.md in the system prompt). Phone passed as a tool parameter.
- `src/tools/builtin/zomato_auth.rs` — Same pattern.

**Lifecycle:**
- **User ID creation:** Telegram WASM channel extracts Telegram user ID from the update, sets it as `IncomingMessage.user_id`. This becomes the `user_id` in `memory_documents`, `conversations`, `secrets`, etc.
- **Phone capture:** During onboarding Q2, LLM writes phone to USER.md. Readable in every subsequent system prompt.
- **Phone re-use for auth:** food-ordering skill instructs the LLM to "read phone from USER.md" and call `swiggy_auth(action="start_auth", phone=<number>)`. Phone travels: USER.md → LLM context → tool call parameter → Swiggy API.
- **The V14 `users` table is not used by Personifi/Aria.** Telegram users do not get a row in `users`.

**What works:**
1. Workspace-based identity (USER.md scoped by user_id) provides tenant isolation.
2. Phone re-auth path works end-to-end for Swiggy (read from USER.md → send OTP → complete auth).
3. Telegram chat ID as user_id is a stable identifier.

## Gaps and Risks

- **CRITICAL: Phone stored in plaintext in USER.md.** USER.md is in `memory_documents.content` (unencrypted TEXT column). Also chunked into `memory_chunks.content` and indexed for FTS. The phone is searchable, queryable, and exposed in the LLM system prompt on every turn. SOUL.md says "never displayed back in full" — enforced only by prompt instruction, not code.
- **No formal Telegram-to-IronClaw user mapping table.** System uses raw Telegram user ID as user_id throughout. No `users` table row for Telegram users. If the same person uses two Telegram accounts, there is no way to link them.
- **V14 `users` table is disconnected from Telegram identity.** No `telegram_id` column, no `phone` column.
- **Phone passes through the LLM.** During re-auth, the LLM reads phone from USER.md (system prompt), then passes it as a tool call parameter. The phone appears in: (1) LLM input tokens, (2) `job_actions.input` log, (3) possibly LLM output if it echoes it. SOUL.md "never displayed back in full" is architecturally unenforceable with this design.
- **No write-once enforcement for phone.** The LLM could overwrite the phone via `memory_write` at any time.

## Proposed Schema

### `user_identities`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | Internal stable ID, e.g. `usr_` + UUID |
| created_at | TEXT | NO | DEFAULT now | |
| updated_at | TEXT | NO | DEFAULT now | |
| status | TEXT | NO | DEFAULT 'active' | active / suspended / deactivated |

### `user_external_identities`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| user_id | TEXT | NO | FK user_identities(id) | |
| provider | TEXT | NO | | `telegram`, `web`, etc. |
| external_id | TEXT | NO | | Telegram numeric user ID |
| display_name | TEXT | YES | | Telegram display name |
| created_at | TEXT | NO | DEFAULT now | |
| UNIQUE(provider, external_id) | | | | One mapping per external account |

**Purpose:** Maps Telegram user 123456789 → IronClaw user `usr_abc123`. Allows future multi-channel identity.

### Phone storage

Phone should **NOT** be in USER.md or any regular database column. It should be in SecretsStore under key `user_phone` scoped to user_id.

- **Write:** During onboarding Q2, after validation, store via `secrets.create(user_id, CreateSecretParams::new("user_phone", phone_digits))`.
- **Read for re-auth:** `swiggy_auth` and `zomato_auth` tools read directly from SecretsStore: `secrets.get_decrypted(user_id, "user_phone")`. This **removes the phone from LLM context entirely**.
- **USER.md:** Replace `phone: 9289289123` with `phone: (captured)`. The LLM does not need the actual digits.

**SOUL.md compliance:** Phone is AES-256-GCM encrypted at rest. Never appears in LLM context. Auth tools read it from SecretsStore directly, bypassing the LLM. "Never displayed back in full" is now enforced architecturally.

## Migration Strategy

1. Create `user_identities` and `user_external_identities` tables.
2. Backfill: for each unique `user_id` in `memory_documents`, create a `user_identities` row and a `user_external_identities` row with `provider = 'telegram'`, `external_id = <current_user_id>`.
3. Migrate phone from USER.md to SecretsStore: parse USER.md for each user, extract phone, call `secrets.create(user_id, "user_phone", phone)`.
4. Rewrite USER.md for each user to replace phone field with `phone: (captured)`.
5. Update `swiggy_auth` and `zomato_auth` tools to read phone from SecretsStore instead of expecting it as a tool parameter.
6. **Breaking change** for the auth flow. Coordinate with tasks/db/05-auth-tokens.md.

**Files that will need code changes:**
- `src/db/mod.rs` — new `UserIdentityStore` sub-trait
- `src/db/postgres.rs`, `src/db/libsql/` — implement it
- `migrations/V15__user_identities.sql` + `src/db/libsql_migrations.rs`
- `src/tools/builtin/swiggy_auth.rs` — read phone from SecretsStore, not from tool params
- `src/tools/builtin/zomato_auth.rs` — same
- `src/workspace/seeds/USER.md` — remove phone field or replace with sentinel
- `skills/onboarding/SKILL.md` — Q2 writes to SecretsStore not USER.md
- `src/workspace/seeds/BOOTSTRAP.md` — same update

## SOUL.md / Spec Compliance Notes

- PERSONIFI_SPEC Phase 0: "Create `UserRecord` in database (keyed by `telegram:{user_id}`)." — `user_external_identities` with `provider = 'telegram'` fulfills this.
- PERSONIFI_SPEC Resolved Decision 5: "Never ask for phone number again after onboarding." — SecretsStore enables this without exposing phone to the LLM.
- SOUL.md "asked once, never displayed back" — architecturally enforced once phone moves to SecretsStore.

## Coordination Notes

- The `user_external_identities` table is a prerequisite for `tasks/db/03-social-graph.md` — the social graph needs stable internal user IDs to support the Telegram WebApp (tasks/review/04-telegram-webapp.md).

## Open Questions

1. Should internal user_id transition happen now or at MVP? At minimum, move the phone to SecretsStore.
2. Phone hashing for lookup: if two Telegram accounts provide the same phone, should they be linked? Requires a phone hash index. Current design does not support this.
3. The `users` table from V14 — repurpose or keep separate? Recommend keeping it for web UI admin and creating separate `user_identities` for Telegram users.
