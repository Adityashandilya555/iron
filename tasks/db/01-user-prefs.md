# User Preferences — Data Model Analysis

**Status:** ANALYSIS
**Area:** preferences, onboarding
**Relates to:** tasks/db/02-user-identity.md (phone storage overlaps)

## Current State

**Where it lives today:**

- `src/workspace/seeds/USER.md` — Seed template with empty fields. Deployed to each new workspace. Contains: name, telegram_id, phone, city, residence, residence_type, diet, cuisines[], budget_min, budget_max, friends[], squads[], onboarding_completed, last_order_platform, last_order_restaurant, order_count.
- `memory_documents` table (PG/libSQL) — USER.md stored as row with `path = "USER.md"`, `user_id = <tenant_id>`, `content = <raw markdown>`.
- `memory_chunks` table — USER.md is chunked and indexed for FTS + vector search.
- `src/workspace/document.rs:21` — `paths::USER = "USER.md"` constant.
- `skills/onboarding/SKILL.md` + `src/workspace/seeds/BOOTSTRAP.md` — Define Q1-Q7 flow and USER.md schema (duplicated in both files).

**Lifecycle:**
- **Write:** LLM writes via `memory_write` tool during onboarding. Fields written incrementally per question. Final full write in BOOTSTRAP.md Step 3.
- **Read:** `workspace.system_prompt()` calls `read_primary("USER.md")` on every LLM turn. Also read by food-ordering, grocery, table-booking, social-engine skills.
- **Mutate:** User says "update my preferences" — skill reads current, modifies field, rewrites entire file.
- **Encryption:** None. Plaintext in `memory_documents.content`.

**What works:**
1. Onboarding resume across sessions — skill reads USER.md, resumes from first missing required field.
2. Tenant isolation — `memory_documents.user_id` scoping + `read_primary()` for identity files.
3. System prompt injection — USER.md automatically included in every LLM turn.
4. Seed template — new users get well-structured empty USER.md.

## Gaps and Risks

- **No typed parsing.** No `UserProfile` struct in Rust. The LLM parses markdown ad-hoc each turn. If LLM writes malformed USER.md (drops a field, changes format), downstream reads break silently.
- **No validation on write.** The `memory_write` tool accepts arbitrary content. Nothing prevents the LLM from writing `diet: maybe` or `budget_max: lots`. Validation is a prompt instruction, not code.
- **Partial onboarding writes may lose data.** Each `memory_write` REPLACES the entire document. If the LLM writes only the new field (not all previous fields), prior answers are lost. BOOTSTRAP.md Step 3 does a full rewrite, but partial writes during Q1-Q6 are not guaranteed to be full.
- **ORDER_PATTERNS.md never created.** agents-manifest says Phase 4 writes it. AGENTS.md references it. No seed file and no code that writes it. Violates PERSONIFI_SPEC Phase 3 proactive nudge requirements.
- **Phone number in plaintext in USER.md.** See 02-user-identity.md.
- **Friends as Telegram usernames, not resolvable IDs.** See 03-social-graph.md.

## Proposed Schema

**Storage engine:** Hybrid — keep USER.md as workspace file for LLM consumption; add a structured `user_preferences` table for code consumption. Both PG and libSQL.

### `user_preferences`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| user_id | TEXT | NO | PK, FK users(id) | Stable IronClaw user ID |
| name | TEXT | YES | | Display name |
| city | TEXT | NO | DEFAULT 'Bangalore' | Pre-set for MVP |
| residence | TEXT | YES | | Hostel/PG name |
| residence_type | TEXT | YES | CHECK IN ('hostel','pg','apartment','home') | |
| diet | TEXT | YES | CHECK IN ('veg','non-veg','eggetarian') | |
| cuisines | TEXT | NO | DEFAULT '[]' | JSON array, lowercase |
| budget_min | INTEGER | NO | DEFAULT 0 | INR |
| budget_max | INTEGER | NO | DEFAULT 0 | 0 = not set |
| onboarding_step | INTEGER | NO | DEFAULT 0 | 0-7 in progress, 8 = complete |
| onboarding_completed_at | TEXT | YES | | ISO-8601 |
| last_order_platform | TEXT | YES | | swiggy / zomato |
| last_order_restaurant | TEXT | YES | | |
| order_count | INTEGER | NO | DEFAULT 0 | |
| created_at | TEXT | NO | DEFAULT now | |
| updated_at | TEXT | NO | DEFAULT now | |

**Indexes:** `idx_user_prefs_residence(residence)` for Phase 7 hostel grouping. `idx_user_prefs_onboarding(onboarding_step) WHERE onboarding_step < 8`.

**SOUL.md compliance:** No phone stored here (moved to separate encrypted storage). Diet/budget not sensitive. Tenant isolation via user_id as PK.

## Migration Strategy

1. Create `user_preferences` table (V15 PG migration + libsql_migrations.rs).
2. Backfill: parse all `memory_documents WHERE path = 'USER.md'` and INSERT into `user_preferences`.
3. Keep USER.md for LLM consumption. Add write-through: table updates regenerate USER.md; USER.md writes (via memory_write) sync to table.
4. Create `ORDER_PATTERNS.md` seed file.

**Files that will need code changes:**
- `src/db/mod.rs` — new `UserPreferencesStore` sub-trait
- `src/db/postgres.rs`, `src/db/libsql/` — implement it
- `migrations/V15__user_preferences.sql` + `src/db/libsql_migrations.rs`
- `src/workspace/` — write-through sync
- `src/workspace/seeds/` — add ORDER_PATTERNS.md seed

## SOUL.md / Spec Compliance Notes

- Phone is explicitly excluded from this table (see 02-user-identity.md).
- `onboarding_step` enables partial onboarding recovery without parsing markdown.
- PERSONIFI_SPEC Phase 2 resume behavior is now enforceable in code: `SELECT onboarding_step FROM user_preferences WHERE user_id = ?`.

## Coordination Notes

- None — this concern is self-contained.

## Open Questions

1. Should `memory_write` to USER.md be intercepted to parse and sync to `user_preferences`? Or should the skill call a separate `set_preference` tool?
2. Is `onboarding_step` (integer counter) sufficient, or do we need per-field presence tracking?
