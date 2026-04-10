# Agent↔User Conversation History — Data Model Analysis

**Status:** ANALYSIS
**Area:** conversation, history, proactive foundation
**Relates to:** tasks/db/01-user-prefs.md (ORDER_PATTERNS.md gap)

## Current State

**Where it lives today:**

- `conversations` table (PG + libSQL) — Tracks conversation metadata: id, channel, user_id, thread_id, started_at, last_activity, metadata. Created per channel/user pair.
- `conversation_messages` table (PG + libSQL) — Individual messages: id, conversation_id, role, content, created_at. Append-only.
- `ConversationStore` trait (`src/db/mod.rs:356`) — 15+ methods including `create_conversation`, `add_conversation_message`, `list_conversation_messages_paginated`, `ensure_conversation`, `get_or_create_assistant_conversation`.
- `job_actions` table — Tool call audit trail: tool_name, input (JSONB), output, cost, duration, success. Linked to agent_jobs which links to conversations.
- `llm_calls` table — Token/cost tracking per LLM call, linked to conversation_id.
- `memory_documents` / `memory_chunks` — Workspace memory with FTS + vector search. This is where ORDER_PATTERNS.md would live (if it existed).
- Embedding subsystem (`src/workspace/embeddings.rs`, `src/config/embeddings.rs`) — Uses OpenAI text-embedding-3-small. Requires `OPENAI_API_KEY` env var. The "Failed to generate embedding: Authentication failed" error means this key is not set or invalid. **Vector search falls back to FTS-only when embeddings fail.**

**Lifecycle:**
- **Conversation creation:** `get_or_create_assistant_conversation(user_id, channel)` is called when a user sends a message.
- **Message persistence:** `add_conversation_message(conversation_id, role, content)` appends each turn. Both user messages and assistant responses are stored.
- **Survives restart:** YES. Both `conversations` and `conversation_messages` are in the database. **Conversation messages already survive container restart.**
- **Session context:** The agent loads recent messages from the database when resuming a conversation via `list_conversation_messages_paginated`.
- **Embedding failure:** Affects workspace `memory_search` quality only. Does NOT affect conversation message storage.

**What works:**
1. Conversation messages survive restart — already in the database.
2. Full audit trail: `conversation_messages` + `job_actions` + `llm_calls`.
3. Pagination support via `list_conversation_messages_paginated`.
4. Tenant isolation — `conversations.user_id` column + `conversation_belongs_to_user` check.
5. FTS search works — even with broken embeddings, full-text search over workspace documents functions.

## Gaps and Risks

- **No extracted patterns/facts layer.** Raw conversation turns exist, but there is no derived "user ordered biryani on Friday at 8pm" fact table. ORDER_PATTERNS.md was supposed to serve this role but is never written. The proactive agent has no structured data to query.
- **Embedding subsystem broken.** `OPENAI_API_KEY` not configured for the embedding provider. Degrades workspace search to FTS-only. Blocks hybrid search quality needed for proactive pattern detection but does not block conversation storage.
- **No OTP/phone redaction in conversation logs.** When a user sends "9289289123" as their phone or "129432" as an OTP, these are stored verbatim in `conversation_messages.content`. SOUL.md: "OTP codes are sensitive. Never store them after verification." Phone in plaintext in logs violates the spirit of SOUL.md even if not explicitly prohibited.
- **No order event extraction.** When a food order is placed (tool call to `place_food_order`), the result (order ID, restaurant, items, total, platform) is in `job_actions.output_raw` but not in a queryable order events table. Proactive features need: "user X ordered Y from Z on date D."
- **`conversation_messages` has no metadata column.** Cannot tag a message as "contains OTP" or "order confirmation" without parsing content. A metadata/tags column would enable selective redaction and event extraction.

## Proposed Schema

Three layers as requested:

### Layer 1: Raw Conversation Log (already exists — no schema changes)

**Tables:** `conversations` + `conversation_messages` — already present and functional.

**Enhancement only:** Add a `metadata` column to `conversation_messages`:

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| metadata | TEXT | NO | DEFAULT '{}' | JSON: tags, redacted flag, phase |

This enables tagging messages (e.g., `{"contains_otp": true, "phase": 4, "redacted": true}`) without changing the core schema.

### Layer 2: Extracted Events (NEW)

#### `order_events`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| user_id | TEXT | NO | FK user_identities(id) | |
| conversation_id | TEXT | YES | FK conversations(id) | Which conversation placed it |
| platform | TEXT | NO | | `swiggy-food` / `zomato` |
| restaurant_name | TEXT | NO | | |
| restaurant_id | TEXT | YES | | Platform-specific ID |
| items | TEXT | NO | DEFAULT '[]' | JSON array of item names |
| total_amount | INTEGER | YES | | INR |
| coupon_applied | TEXT | YES | | Coupon code |
| payment_method | TEXT | YES | | `cod` / `upi_qr` / `pay_later` |
| order_id_external | TEXT | YES | | Platform order ID |
| ordered_at | TEXT | NO | | ISO-8601 |
| day_of_week | INTEGER | NO | | 0=Mon, 6=Sun (pre-computed for pattern queries) |
| hour_of_day | INTEGER | NO | | 0-23 (pre-computed for pattern queries) |

**Indexes:** `idx_order_events_user_date(user_id, ordered_at DESC)`, `idx_order_events_user_dow(user_id, day_of_week)`, `idx_order_events_restaurant(user_id, restaurant_name)`

**Who writes:** Post-order hook in the food-ordering flow. After `place_food_order` / `checkout_cart` succeeds, extract order details from the tool response and INSERT. Code hook is more reliable than LLM-driven write.

#### `detected_patterns`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| user_id | TEXT | NO | FK user_identities(id) | |
| pattern_type | TEXT | NO | | `day_cuisine`, `time_restaurant`, `weekly_order` |
| description | TEXT | NO | | "Fridays: biryani from Paradise" |
| confidence | REAL | NO | | 0.0–1.0 |
| data | TEXT | NO | DEFAULT '{}' | JSON: supporting evidence |
| detected_at | TEXT | NO | | |
| last_matched_at | TEXT | YES | | Last time pattern was confirmed |
| active | INTEGER | NO | DEFAULT 1 | 0/1 boolean |

**Who writes:** Future background job (proactive agent) that queries `order_events` and detects recurrence. **NOT built now** — table exists as foundation only.

### Layer 3: Short-Term Session Context (ephemeral — no schema needed)

Already works: the agent loads recent `conversation_messages` from the DB. Context window size managed by LLM provider token limit. No schema changes needed.

## Migration Strategy

1. Add `metadata TEXT NOT NULL DEFAULT '{}'` to `conversation_messages`. For PG: `ALTER TABLE`. For libSQL: versioned migration via `src/db/libsql_migrations.rs` (see `src/db/CLAUDE.md` for the rebuild approach since libSQL lacks ALTER TABLE).
2. Create `order_events` table.
3. Create `detected_patterns` table.
4. Backfill `order_events` from `job_actions WHERE tool_name IN ('place_food_order', 'checkout_cart') AND success = true`. Parse `output_raw` for order details.
5. Fix embedding subsystem: ensure `OPENAI_API_KEY` is configured. Config issue, not a schema issue.
6. Create `ORDER_PATTERNS.md` seed file for LLM-authored patterns (separate from `detected_patterns` which is code-authored).

**Files that will need code changes:**
- `src/db/mod.rs` — new `OrderEventStore` sub-trait (or extend `ConversationStore`)
- `src/db/postgres.rs`, `src/db/libsql/` — implement
- `migrations/` + `src/db/libsql_migrations.rs`
- `src/tools/builtin/` or `src/agent/` — post-order hook to extract order events
- `src/config/embeddings.rs` — document `OPENAI_API_KEY` as required for production
- `src/workspace/seeds/ORDER_PATTERNS.md` — create seed file

## SOUL.md / Spec Compliance Notes

- SOUL.md: "OTP codes are sensitive. Never store them after verification." — The `metadata` column enables flagging OTP messages for redaction. Actual redaction requires a write-path interceptor.
- SOUL.md: "Order history is private. Each user's workspace is tenant-isolated." — `order_events.user_id` scoping enforces this.
- PERSONIFI_SPEC Phase 3 proactive nudges — `order_events` + `detected_patterns` are the prerequisite data layer. Not building proactive features now; just laying the groundwork.

## Coordination Notes

- None — this concern is self-contained.

## Open Questions

1. Should OTP messages be redacted from `conversation_messages` on write? Options: (a) redact on write — clean but loses the turn, (b) mark with metadata + redact on read — preserves audit trail, (c) accept the risk for now since DB is server-side only. SOUL.md implies option (a).
2. Who writes `order_events` — the skill (via LLM tool call) or a code hook? A code hook is more reliable (LLM might forget). Requires parsing tool outputs in Rust.
3. Is the embedding failure actually blocking anything today? If semantic search is not used in production flows (only FTS), it may be low priority. But it degrades `memory_search` quality for the social ranking features in Phase 7.
