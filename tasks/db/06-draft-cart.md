# Draft Cart — Local Persistent Cart for Chat Mode

**Status:** NEW TASK (not in original task set)
**Area:** cart, chat-mode, food-ordering
**Relates to:** tasks/USER_JOURNEY.md (source of truth), tasks/review/03-skill-hardening-modified.md (phase-gated tools)

## Why This Exists

PERSONIFI_SPEC has no concept of a local cart. All carts live on the platform (Swiggy/Zomato) and are session-local until checkout. The user's vision is different: chat mode is for exploring and deciding, `/order` is for executing. A persistent draft cart bridges the two — users accumulate items during chat, then push to platform in one shot.

## Current State

**No draft cart exists anywhere in the codebase.**

- `skills/food-ordering/SKILL.md:125` — `update_food_cart` calls Swiggy's API directly. Cart lives on Swiggy's servers, session-scoped.
- `skills/food-ordering/SKILL.md:128` — `create_cart` calls Zomato's API. Same pattern.
- `skills/grocery-ordering/SKILL.md:116` — `update_cart` for Instamart. Same.
- No `draft_cart` table in any migration (`migrations/V1-V14`, `src/db/libsql_migrations.rs`).
- No draft cart tool in `src/tools/builtin/` or `src/tools/registry.rs`.
- No workspace file for cart state (USER.md has no cart fields).

## Proposed Schema

### `draft_cart_items`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| user_id | TEXT | NO | INDEX | IronClaw user ID |
| restaurant_name | TEXT | NO | | Display name (from search results) |
| restaurant_id | TEXT | YES | | Platform-specific restaurant ID |
| platform | TEXT | NO | | `swiggy-food` / `zomato` / `unknown` |
| item_name | TEXT | NO | | |
| item_id | TEXT | YES | | Platform-specific item ID |
| variant | TEXT | YES | | "Large", "Half", etc. |
| quantity | INTEGER | NO | DEFAULT 1 | |
| unit_price | INTEGER | YES | | INR, captured at browse time |
| addons | TEXT | NO | DEFAULT '[]' | JSON array of addon names |
| added_at | TEXT | NO | DEFAULT now | ISO-8601 |

**Indexes:**
- `idx_draft_cart_user(user_id)` — primary lookup
- `idx_draft_cart_user_restaurant(user_id, restaurant_name)` — group by restaurant

**Both backends:** PG migration + libSQL migration in `src/db/libsql_migrations.rs`.

## Required Tools (Built-in Rust, NOT MCP)

### `DraftCartTool` — `src/tools/builtin/draft_cart.rs`

Single tool with `action` parameter (like `swiggy_auth`/`zomato_auth` pattern).

**Actions:**

| Action | Parameters | Returns | Notes |
|---|---|---|---|
| `add` | `item_name, restaurant_name, platform, variant?, quantity?, unit_price?, item_id?, restaurant_id?, addons?` | Cart item ID + updated cart summary | Appends row to `draft_cart_items` |
| `remove` | `item_id` (the draft cart UUID, not platform ID) | Updated cart summary | Deletes single row |
| `view` | (none) | Full cart: items grouped by restaurant, with estimated totals | Reads all rows for user |
| `clear` | (none) | Confirmation | Deletes all rows for user |
| `update` | `item_id, quantity?, variant?` | Updated cart summary | Modifies existing row |

**Implementation pattern:**
```rust
pub struct DraftCartTool {
    store: Arc<dyn Database>,
}

impl Tool for DraftCartTool {
    fn name(&self) -> &str { "draft_cart" }
    fn domain(&self) -> ToolDomain { ToolDomain::Orchestrator }
    // ...
}
```

**Requires `Database` access** (not just SecretsStore). Follow the pattern of `memory_write`/`memory_read` which also take a store reference.

**Registration:** `src/tools/registry.rs` — register alongside memory tools (line ~341). Needs `Arc<dyn Database>` passed in, same as `MemoryWriteTool`.

### Where `DraftCartTool` reads the user_id

From `JobContext.user_id` (same as all other built-in tools). The `JobContext` is created in `dispatcher.rs:148` with `message.user_id`.

## Database Trait Extension

### Option A: Extend `Database` trait (recommended)

Add to `src/db/mod.rs`:

```rust
// Draft cart operations
async fn add_draft_cart_item(&self, user_id: &str, item: DraftCartItem) -> Result<String, DatabaseError>;
async fn remove_draft_cart_item(&self, user_id: &str, item_id: &str) -> Result<(), DatabaseError>;
async fn update_draft_cart_item(&self, user_id: &str, item_id: &str, updates: DraftCartUpdate) -> Result<(), DatabaseError>;
async fn list_draft_cart_items(&self, user_id: &str) -> Result<Vec<DraftCartItem>, DatabaseError>;
async fn clear_draft_cart(&self, user_id: &str) -> Result<u64, DatabaseError>;
```

Implement in both `src/db/postgres.rs` and `src/db/libsql/`.

### Option B: New `DraftCartStore` sub-trait

If the `Database` trait is already too large, create a separate `DraftCartStore` trait and implement it. Follow the pattern of `ConversationStore`, `RoutineStore`, etc.

## Integration with `/order` Flow

When user sends `/order`:
1. `SubmissionParser` parses it as `Submission::PhaseSwitch { phase: AriaPhase::Order }` (see `tasks/review/03-skill-hardening-modified.md`)
2. Handler in `agent_loop.rs` reads draft cart via `store.list_draft_cart_items(user_id)`
3. If empty → respond "Cart is empty, browse in /chat first"
4. If items span multiple restaurants → ask user to pick one restaurant
5. If items span multiple platforms → show cross-platform price comparison
6. User picks platform → lock platform
7. Push items to platform cart (MCP tool calls)
8. Continue with normal checkout flow (coupons → confirm → place → track)
9. On successful order → `store.clear_draft_cart(user_id)`

## Item ID Resolution

**Problem:** During chat mode, the user browses menus. The agent gets `item_id` and `restaurant_id` from MCP search/menu responses. These must be stored in the draft cart so they can be used to create the platform cart at `/order` time.

**Solution:** The `draft_cart_add` action accepts optional `item_id` and `restaurant_id`. The food-ordering skill (or a new `order-execution` skill) uses these IDs when pushing to the platform. If IDs are missing (user described item by name only), the skill must re-search the menu at `/order` time to resolve them.

## Token Impact

Draft cart tool definitions add ~400 tokens (1 tool, 5 actions). But it replaces the need for platform cart tools in chat mode (which would be ~1200 tokens for 3 tools). Net saving in chat mode.

## Files That Need Changes

| File | Change | Type |
|---|---|---|
| `src/db/mod.rs` | `DraftCartItem` struct + trait methods | additive |
| `src/db/postgres.rs` | Implement draft cart queries | additive |
| `src/db/libsql/` | Implement draft cart queries | additive |
| `migrations/V15__draft_cart.sql` | Create `draft_cart_items` table | additive |
| `src/db/libsql_migrations.rs` | libSQL version of table | additive |
| `src/tools/builtin/draft_cart.rs` | New tool implementation | new file |
| `src/tools/builtin/mod.rs` | Export `DraftCartTool` | additive |
| `src/tools/registry.rs` | Register `DraftCartTool` (~line 370) | additive |
| `skills/food-ordering/SKILL.md` | Update to reference draft cart in `/order` flow | modify |

## Open Questions

1. Should draft cart items expire? (e.g., prices become stale after 24h). Recommendation: no expiry, but show "prices may have changed" warning if item is >2h old.
2. Should the draft cart support grocery/table items? Recommendation: food-only for now. Instamart has its own cart model (multi-store). Dineout has no cart concept.
3. Multi-restaurant carts: allow or force single-restaurant? Recommendation: allow multi-restaurant in draft (comparison), but at `/order` time force single-restaurant selection.
