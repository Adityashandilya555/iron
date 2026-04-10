# User Preferences — MODIFIED (Preference Evolution Added)

**Status:** REVISED (supersedes 01-user-prefs.md)
**Changes from original:**
1. Added preference evolution mechanism (post-order updates)
2. Added `user_preferences` fields for tracking preference drift
3. Connected preferences to chat mode suggestion quality
4. Added ORDER_PATTERNS.md ownership (original flagged as missing, nobody owned it)

---

## What the Original Task Got Right

- Hybrid storage: USER.md for LLM + `user_preferences` table for code (still the right approach)
- Schema for `user_preferences` table (still valid, with additions below)
- No typed parsing of USER.md is a real risk (still true)
- Partial onboarding writes losing data (still true)

## What the Original Task Missed

### 1. Preferences Are Static After Onboarding

The original task treats preferences as write-once during onboarding, read-many afterward. The user's vision is that preferences improve over time based on behavior. The table needs fields to support this.

### 2. ORDER_PATTERNS.md Is Unowned

Both 01-user-prefs.md and 04-conversation-history.md flag ORDER_PATTERNS.md as missing but neither claims ownership. This file is the LLM-readable summary of a user's order patterns — it feeds into chat mode suggestions.

### 3. No Connection to Chat Mode Suggestions

The original task doesn't explain HOW preferences improve conversation quality. The link is: preferences → skill context → LLM system prompt → better suggestions in chat mode.

---

## Schema Additions to `user_preferences`

Add these columns to the schema proposed in the original task:

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| cuisine_confidence | TEXT | NO | DEFAULT '{}' | JSON: `{"biryani": 0.9, "chinese": 0.6}` — order-frequency-weighted |
| actual_avg_spend | INTEGER | YES | | Computed from `order_events`, INR |
| preferred_platforms | TEXT | NO | DEFAULT '{}' | JSON: `{"swiggy-food": 12, "zomato": 8}` — order counts per platform |
| preferred_restaurants | TEXT | NO | DEFAULT '[]' | JSON: top 5 by frequency `[{"name": "Paradise", "count": 7, "platform": "swiggy-food"}]` |
| last_preference_update | TEXT | YES | | ISO-8601 — when post-order enrichment last ran |
| preference_version | INTEGER | NO | DEFAULT 1 | Incremented on each enrichment pass |

## Preference Evolution Mechanism

### Phase 2: Post-Order Enrichment (Build Now)

**Trigger:** After successful order (when `order_events` row is written — see tasks/db/04-conversation-history.md).

**Implementation:** A Rust function called from the post-order hook (same place that writes `order_events`).

```rust
async fn enrich_preferences_from_order(
    store: &dyn Database,
    user_id: &str,
    order: &OrderEvent,
) -> Result<(), DatabaseError> {
    let prefs = store.get_user_preferences(user_id).await?;

    // 1. Update order count + last order info
    prefs.order_count += 1;
    prefs.last_order_platform = Some(order.platform.clone());
    prefs.last_order_restaurant = Some(order.restaurant_name.clone());

    // 2. Update actual average spend (running average)
    if let Some(total) = order.total_amount {
        prefs.actual_avg_spend = Some(match prefs.actual_avg_spend {
            Some(avg) => (avg * (prefs.order_count - 1) + total) / prefs.order_count,
            None => total,
        });
    }

    // 3. Update platform preference counts
    // Increment count for order.platform in preferred_platforms JSON

    // 4. Update restaurant frequency
    // Add/increment order.restaurant_name in preferred_restaurants JSON
    // Keep top 5 by count

    // 5. Update cuisine confidence
    // For each item in order, extract cuisine tag (from restaurant category)
    // Increment confidence score, normalize to 0.0-1.0

    prefs.last_preference_update = Some(now_iso8601());
    prefs.preference_version += 1;

    store.update_user_preferences(user_id, &prefs).await?;

    // 6. Regenerate USER.md from updated preferences (write-through)
    // This ensures the LLM system prompt reflects current preferences
    regenerate_user_md(workspace, &prefs).await?;
}
```

**Where this code lives:** `src/tools/builtin/` (new file, e.g., `preference_enrichment.rs`) or as a method on the `Database` trait helper.

**When it runs:** Called from the same post-order hook that writes `order_events`. NOT a background job — runs synchronously after order placement.

### Phase 3: Algorithmic Evolution (Future Work)

- Detect diet drift: user said "veg" but `order_events` shows 5+ non-veg orders → surface in chat: "Your recent orders have been non-veg. Want me to update your preferences?"
- Detect budget drift: `actual_avg_spend` diverges from `budget_max` by >30% → adjust suggestions
- Detect cuisine expansion: user tries new cuisine 3+ times → add to `cuisines[]`
- Detect time-of-day patterns: lunch vs dinner preferences differ

**These are NOT built now.** The `user_preferences` schema supports them. Implementation deferred to the proactive agent system.

## ORDER_PATTERNS.md Ownership

**This task now owns ORDER_PATTERNS.md creation.**

### Seed File: `src/workspace/seeds/ORDER_PATTERNS.md`

```markdown
# Order Patterns

<!-- Auto-updated after each order. Read by the proactive agent and chat mode suggestions. -->

## Recent Orders
(none yet)

## Detected Patterns
(none yet — patterns are detected after 5+ orders)

## Preferred Restaurants
(updated from order history)

## Time Patterns
(updated from order history)
```

### Who Writes It

The same post-order enrichment hook that updates `user_preferences` also updates ORDER_PATTERNS.md in the user's workspace:

```rust
async fn update_order_patterns_md(
    workspace: &Workspace,
    order: &OrderEvent,
    prefs: &UserPreferences,
) -> Result<(), WorkspaceError> {
    // Read current ORDER_PATTERNS.md
    // Append to "Recent Orders" section (keep last 20)
    // Update "Preferred Restaurants" from prefs.preferred_restaurants
    // Write back via workspace.write_primary()
}
```

**LLM consumption:** ORDER_PATTERNS.md is read by `system_prompt()` if it exists in the workspace. The skill context (food-ordering, mcp-orchestrator) references it for suggestion generation.

### Registration in `seed_if_empty()`

Add ORDER_PATTERNS.md to `src/workspace/mod.rs:1624` seed list, alongside USER.md, AGENTS.md, etc.

## How Preferences Improve Chat Quality

**The connection:** `system_prompt()` (workspace/mod.rs:1131) loads USER.md into every LLM call. USER.md contains preferences. If preferences are richer and more current, the LLM gives better suggestions.

**Example:**
- Static preferences (onboarding only): `cuisines: [biryani, chinese]` → LLM suggests biryani and chinese
- Enriched preferences (after 10 orders): `cuisines: [biryani, chinese, pizza], preferred_restaurants: [Paradise (7 orders), Dominos (3 orders)], actual_avg_spend: 280` → LLM suggests "Paradise like usual?" on Friday, or "You've been spending ~₹280 avg, here's a deal that fits"

**ORDER_PATTERNS.md adds temporal context:** "Last 3 Fridays: biryani from Paradise" → LLM suggests same on Friday without being explicitly told.

## Files That Need Changes (Beyond Original Task)

| File | Change | Type |
|---|---|---|
| `src/workspace/seeds/ORDER_PATTERNS.md` | New seed file | new file |
| `src/workspace/mod.rs` | Add ORDER_PATTERNS.md to `seed_if_empty()` seed list | additive |
| `src/tools/builtin/preference_enrichment.rs` OR inline in post-order hook | Post-order preference update logic | new file |
| `src/db/mod.rs` | `update_user_preferences()` method | additive |
| `src/db/postgres.rs`, `src/db/libsql/` | Implement update | additive |
| `migrations/` | Add new columns to `user_preferences` table | additive |

## Open Questions (Updated)

1. (From original) Should `memory_write` to USER.md be intercepted to sync to `user_preferences`? **Recommendation:** Yes, but only for the onboarding flow. Post-order updates go table → USER.md (reverse direction). Bidirectional sync adds complexity for little value.
2. (New) How often should ORDER_PATTERNS.md be regenerated? After every order (simple) or batched daily (less write overhead)? **Recommendation:** After every order — the write is cheap and immediate freshness matters for suggestions.
3. (New) Should `cuisine_confidence` be computed from order items or restaurant categories? Restaurant categories are easier (available in search results) but less precise. **Recommendation:** Restaurant categories for now; item-level analysis is future work.
