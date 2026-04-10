# Agent<>User Conversation History — MODIFIED

**Status:** REVISED (supersedes 04-conversation-history.md)
**Changes from original:**
1. Confirmed conversation storage ALREADY WORKS — no schema changes needed for raw storage
2. Algorithmic reasoning explicitly marked as FUTURE WORK (not deferred ambiguously)
3. Connected `order_events` extraction to preference enrichment pipeline (tasks/db/01-user-prefs-modified.md)
4. Added metadata column for message tagging (phase, contains_otp, cart_action)
5. Clarified what "future algorithmic work" means and what it operates on

---

## What Already Works (No Changes Needed)

These are CONFIRMED from codebase exploration:

1. **`conversations` table** — per-user, per-channel. Created via `get_or_create_assistant_conversation()`. Has `metadata JSONB`.
2. **`conversation_messages` table** — every turn stored: `id, conversation_id, role, content, created_at`. Append-only. Survives restart.
3. **`job_actions` table** — every tool call logged: `tool_name, input, output, cost, duration, success`. Linked to conversations via `agent_jobs`.
4. **`llm_calls` table** — token/cost tracking per LLM call, linked to `conversation_id`.
5. **Context loading** — `thread_ops.rs:129` calls `list_conversation_messages(thread_uuid)`, `rebuild_chat_messages_from_db()` (line 1845) reconstructs full history for LLM.
6. **Compaction** — `context_monitor.rs` triggers at 80% context usage. Strategies: MoveToWorkspace (80-85%), Summarize (85-95%), Truncate (>95%).

**Bottom line:** Agent<>human conversation is ALREADY stored in the database and loaded for every LLM call. No schema changes needed for basic persistence.

## What Needs to Be Built

### 1. `order_events` Table (from original task — still valid)

Schema unchanged from original:

| Column | Type | Notes |
|---|---|---|
| id | TEXT PK | UUID |
| user_id | TEXT NOT NULL | FK to user |
| conversation_id | TEXT | Which conversation placed it |
| platform | TEXT NOT NULL | `swiggy-food` / `zomato` |
| restaurant_name | TEXT NOT NULL | |
| restaurant_id | TEXT | Platform-specific |
| items | TEXT DEFAULT '[]' | JSON array |
| total_amount | INTEGER | INR |
| coupon_applied | TEXT | |
| payment_method | TEXT | |
| order_id_external | TEXT | Platform order ID |
| ordered_at | TEXT NOT NULL | ISO-8601 |
| day_of_week | INTEGER NOT NULL | 0-6 (pre-computed) |
| hour_of_day | INTEGER NOT NULL | 0-23 (pre-computed) |

**Who writes it:** Post-order Rust hook. After `place_food_order` / `checkout_cart` tool succeeds, parse the tool response and INSERT. This is a code hook, not LLM-driven.

**Where the hook lives:** In the `ChatDelegate::after_iteration()` method (dispatcher.rs) or as a post-tool-execution interceptor. When tool name matches `place_food_order` or `checkout_cart` and result is success:
1. Parse order details from tool output JSON
2. INSERT into `order_events`
3. Call `enrich_preferences_from_order()` (tasks/db/01-user-prefs-modified.md)
4. Call `update_order_patterns_md()` (tasks/db/01-user-prefs-modified.md)

### 2. `metadata` Column on `conversation_messages`

Add `metadata TEXT NOT NULL DEFAULT '{}'` to `conversation_messages`.

**Use cases:**
- `{"phase": 3.5}` — which Aria phase was active when this message was sent
- `{"contains_otp": true}` — flag for future redaction
- `{"cart_action": "add", "item": "Chicken Biryani"}` — track cart activity in chat mode
- `{"order_placed": true, "order_id": "..."}` — mark order confirmation messages

**This enables future algorithmic work without re-parsing message content.**

### 3. ORDER_PATTERNS.md Seed File

Owned by tasks/db/01-user-prefs-modified.md. Created there and written by the post-order hook.

## Future Algorithmic Work (EXPLICITLY DEFERRED)

The following are documented as future work. They are NOT in current scope. The schemas above lay the foundation.

### Pattern Detection Engine

**What it does:** Queries `order_events` for recurring patterns. Writes to `detected_patterns` table.

**`detected_patterns` table (create now, populate later):**

| Column | Type | Notes |
|---|---|---|
| id | TEXT PK | UUID |
| user_id | TEXT NOT NULL | |
| pattern_type | TEXT NOT NULL | `day_cuisine`, `time_restaurant`, `weekly_order`, `budget_trend` |
| description | TEXT NOT NULL | Human-readable: "Fridays: biryani from Paradise" |
| confidence | REAL NOT NULL | 0.0-1.0 |
| data | TEXT DEFAULT '{}' | JSON: supporting evidence |
| detected_at | TEXT NOT NULL | |
| last_matched_at | TEXT | Last time pattern was confirmed |
| active | INTEGER DEFAULT 1 | |

**Detection algorithms (future):**
- Day-of-week × cuisine frequency (SQL: `SELECT day_of_week, items, COUNT(*) FROM order_events GROUP BY ...`)
- Time-of-day × restaurant affinity
- Budget trend (rolling average of `total_amount`)
- Restaurant rotation detection

**Trigger:** Background routine (`src/agent/routine_engine.rs`) running on a schedule (e.g., daily). Reads `order_events`, computes patterns, writes `detected_patterns`.

### Chat Suggestion Pipeline (Future)

**What it does:** Reads `detected_patterns` + `user_preferences` + recent `order_events` → generates suggestion text → injected into LLM system prompt during chat mode.

**Where it injects:** In `system_prompt()` (workspace/mod.rs:1131), after loading ORDER_PATTERNS.md. If `detected_patterns` has active patterns, format them as a section:

```
## Suggestions for Today
- It's Friday — you usually order biryani from Paradise (~₹280)
- Zomato has a 40% off deal at Paradise today
- Your hostel is trending on Meghana Foods
```

**This is NOT built now.** ORDER_PATTERNS.md provides a manual/LLM-written approximation until the algorithmic pipeline is built.

### MCP Usage Optimization (Future)

**What it does:** Analyzes `job_actions` to identify which tool call sequences produce the best outcomes.

**Example insights:**
- "For biryani searches, Zomato returns better results than Swiggy 70% of the time"
- "Users who see price comparison before cart creation have 40% higher conversion"
- "3-step search (keyword → restaurant → menu) works better than 2-step (keyword → menu)"

**This feeds into skill refinement** — the SKILL.md instructions can be tuned based on actual tool call effectiveness.

**NOT in scope. Listed here as a guide for future work.**

### Conversation Analysis (Future)

**What it does:** NLP over `conversation_messages` to detect:
- Satisfaction signals ("thanks", "perfect", order placed after suggestion)
- Frustration signals ("no", "not that", "I said...", repeated corrections)
- Preference shifts discussed in chat but not yet reflected in orders

**NOT in scope. Listed here to justify the `metadata` column addition.**

## Files That Need Changes

| File | Change | Type |
|---|---|---|
| `src/db/mod.rs` | `OrderEventStore` trait methods + `DraftCartItem` struct | additive |
| `src/db/postgres.rs` | Implement `order_events` queries | additive |
| `src/db/libsql/` | Same for libSQL | additive |
| `migrations/V15__order_events.sql` | Create `order_events` + `detected_patterns` tables | new migration |
| `migrations/V16__conversation_metadata.sql` | Add `metadata` column to `conversation_messages` | new migration |
| `src/db/libsql_migrations.rs` | Both tables + column for libSQL | additive |
| `src/agent/dispatcher.rs` or `src/tools/execute.rs` | Post-tool hook for order event extraction | additive |
| `src/workspace/seeds/ORDER_PATTERNS.md` | Seed file (owned by 01-user-prefs-modified) | new file |

## Migration Numbering Coordination

Multiple tasks propose new migrations. Tentative numbering:
- V15: `draft_cart_items` (tasks/db/06-draft-cart.md)
- V16: `user_preferences` (tasks/db/01-user-prefs.md)
- V17: `user_identities` + `user_external_identities` (tasks/db/02-user-identity.md)
- V18: `friendships` + `squads` + `squad_members` + `group_order_*` (tasks/db/03-social-graph.md)
- V19: `order_events` + `detected_patterns` (this task)
- V20: `auth_status` (tasks/db/05-auth-tokens.md)
- V21: `conversation_messages.metadata` column (this task)

**These numbers will shift based on implementation order.** The key constraint: `user_identities` must exist before `friendships` (FK dependency).

## Open Questions (Updated)

1. (Original) OTP redaction: defer. Tag with metadata `{"contains_otp": true}` now, implement redaction later.
2. (Original) Who writes `order_events`: code hook (recommended, more reliable than LLM).
3. (New) Should the post-order hook be in `ChatDelegate::after_iteration()` or in `execute_tool_with_safety()` (src/tools/execute.rs)? Tool-level hook is more universal (works for job and container delegates too), but chat delegate is simpler for MVP.
4. (New) `conversation_messages.metadata` for libSQL: libSQL doesn't support `ALTER TABLE ADD COLUMN` cleanly. Use the rebuild approach documented in `src/db/CLAUDE.md`.
