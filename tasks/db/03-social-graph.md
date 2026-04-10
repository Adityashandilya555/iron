# Social Relationships Graph — Data Model Analysis

**Status:** ANALYSIS
**Area:** social, community
**Relates to:** tasks/review/04-telegram-webapp.md (product-review agent owns WebApp code surface)

## Current State

**Where it lives today:**

- `USER.md friends: []` — A flat list of Telegram usernames (e.g., `[@rahul_k, @priya_m]`). Written during onboarding Q7 or later via social-engine skill.
- `USER.md squads: []` — Empty array. Never populated by any existing code or skill.
- `skills/social-engine/SKILL.md` — Reads `USER.md -> friends[]`. Friend management: add/remove via `memory_write` to USER.md. Notes: "cross-user data is not available (tenant isolation). Hostel grouping will be effective once aggregate analytics are implemented."
- `skills/group-order/SKILL.md` — References `preferences/social.md` (does not exist). Mentions `group_order()` tool (does not exist). Assumes friends can be invited by phone number.
- **No database tables** for friendships, squads, or group order sessions exist.

**What works:**
1. The concept is well-specified in PERSONIFI_SPEC.md Phase 7.
2. Tenant isolation via workspace prevents direct cross-user data access.

## Gaps and Risks

- **Friends stored as @usernames, not resolvable.** `@rahul_k` cannot be resolved to a Telegram user ID or IronClaw user_id. The Telegram Bot API does not support username-to-ID resolution.
- **No mutual friendship confirmation.** User A adding `@rahul_k` does not mean Rahul consented. SOUL.md privacy rules imply friendship should be bidirectional.
- **`preferences/social.md` referenced in group-order skill does not exist.**
- **No `group_order` tool exists.** The group-order SKILL.md references a `group_order(action=...)` tool that has not been implemented.
- **Hostel grouping requires cross-tenant queries.** To find "users in the same hostel," the system needs to query `user_preferences.residence` across multiple users. Current tenant isolation prevents this. An aggregate/anonymized query layer is needed.
- **No squad detection or storage.** PERSONIFI_SPEC describes auto-detected squads from shared ordering patterns. No infrastructure exists.

## Proposed Schema

### `friendships`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| requester_id | TEXT | NO | FK user_identities(id) | Who sent the friend request |
| addressee_id | TEXT | NO | FK user_identities(id) | Who received it |
| status | TEXT | NO | DEFAULT 'pending' | pending / accepted / declined / blocked |
| created_at | TEXT | NO | DEFAULT now | |
| accepted_at | TEXT | YES | | When status changed to accepted |
| UNIQUE(requester_id, addressee_id) | | | | One request per direction |

**Design decision: Bidirectional (mutual).** Friendship requires both sides to confirm. Requester sends invite via Telegram deep link. Addressee accepts via Aria. Only `status = 'accepted'` friendships are used for social ranking.

**SOUL.md compliance:** Social signals are aggregated — "3 people ordered here", never "Rahul ordered biryani at 8 PM." The query layer must return counts, not individual details.

**Indexes:** `idx_friendships_addressee(addressee_id, status)`, `idx_friendships_requester(requester_id, status)`

### `squads`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| name | TEXT | NO | | "HB-1 Crew", "Friday Biryani Gang" |
| created_by | TEXT | NO | FK user_identities(id) | |
| squad_type | TEXT | NO | | `manual` / `auto_detected` |
| created_at | TEXT | NO | DEFAULT now | |

### `squad_members`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| squad_id | TEXT | NO | FK squads(id) | |
| user_id | TEXT | NO | FK user_identities(id) | |
| joined_at | TEXT | NO | DEFAULT now | |
| PRIMARY KEY(squad_id, user_id) | | | | |

### `group_order_sessions`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| initiator_id | TEXT | NO | FK user_identities(id) | |
| restaurant_name | TEXT | NO | | |
| platform | TEXT | NO | | `swiggy-food` / `zomato` |
| status | TEXT | NO | DEFAULT 'collecting' | collecting / finalized / placed / expired |
| expires_at | TEXT | NO | | 2 hours from creation |
| created_at | TEXT | NO | DEFAULT now | |

### `group_order_participants`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| session_id | TEXT | NO | FK group_order_sessions(id) | |
| user_id | TEXT | NO | FK user_identities(id) | |
| items | TEXT | NO | DEFAULT '[]' | JSON array of selected items |
| subtotal | INTEGER | YES | | INR |
| joined_at | TEXT | NO | DEFAULT now | |
| PRIMARY KEY(session_id, user_id) | | | | |

## Migration Strategy

1. Prerequisite: `user_identities` + `user_external_identities` tables from `tasks/db/02-user-identity.md` must exist first.
2. Create `friendships`, `squads`, `squad_members`, `group_order_sessions`, `group_order_participants` tables.
3. Backfill friends: parse USER.md `friends: [@username1, @username2]` for each user. For each @username, attempt to find a matching `user_external_identities` row. If found, create a `friendships` row with `status = 'accepted'` (grandfathered). If not found, store as a pending lookup.
4. Implement `group_order` tool referenced by the group-order skill.
5. Create `preferences/social.md` seed file or redirect the skill to read from the friendships table.

**Files that will need code changes:**
- `src/db/mod.rs` — new `SocialStore` sub-trait
- `src/db/postgres.rs`, `src/db/libsql/` — implement
- `migrations/` + `src/db/libsql_migrations.rs` — new tables
- `src/tools/builtin/` — new `group_order` tool
- `skills/group-order/SKILL.md` — update to reference correct data sources
- `skills/social-engine/SKILL.md` — update friend management to use DB instead of USER.md

## SOUL.md / Spec Compliance Notes

- PERSONIFI_SPEC Phase 7 "Order history correlation": `friendships JOIN order_events WHERE status = 'accepted'` returns count only — never individual user details.
- SOUL.md "Never share user A's data with user B": enforced at query layer by returning aggregates, not rows.
- Hostel grouping queries are a deliberate, scoped cross-tenant read of `user_preferences.residence` (a non-sensitive field). Must return counts/aggregates only.

## Coordination Notes

- **product-review agent owns the WebApp code surface** — see tasks/review/04-telegram-webapp.md. This schema (specifically `friendships`) is the data layer the WebApp will read and write.
- Friend discovery mechanism: Telegram Bot API cannot resolve @username to user_id. Recommended: deep link invite (`t.me/AriaBot?start=friend_<user_id>`). product-review agent should wire this into the WebApp design.
- The `user_identities` table from tasks/db/02-user-identity.md is a **hard prerequisite** for this schema.

## Open Questions

1. **Friend discovery:** Deep link invite vs phone number matching vs Telegram contact sharing. Deep link is most practical for MVP.
2. **UPI phone in `group_order_participants`:** Removed from proposed schema pending resolution — collecting per-session UPI phone conflicts with SOUL.md phone privacy rules. Group order split should use the SecretStore-stored phone, not re-collect it.
3. **Hostel grouping queries:** Require cross-tenant reads of `user_preferences.residence`. Ensure queries return only counts/aggregates, never individual user data.
