# Aria User Journey — Source of Truth

**Status:** APPROVED
**Date:** 2026-04-09
**Supersedes:** PERSONIFI_SPEC.md Phase 3–4 flow (spec remains canonical for onboarding, auth, grocery, table booking, community engine)

---

## Design Principles

1. **Two commands, not seven.** Users interact via `/chat` and `/order`. No `/groceries`, `/table`, `/home`, `/active`. All discovery happens in chat; all transactions happen in order.
2. **Local cart as bridge.** Chat mode builds a persistent draft cart. `/order` pushes it to the chosen platform. The user is never surprised by a platform cart they didn't finalize.
3. **MCP servers are infrastructure, not features.** Swiggy and Zomato MCP servers are pre-registered at deployment. Users never see "install MCP" prompts.
4. **Token budget matters.** Phase-gate tools so the LLM only sees what it needs. Chat mode gets ~12 info-only tools. Order mode gets ~20 transaction tools. Never all 48+.
5. **Conversation is the product.** Every user<>agent exchange is stored in the database. Future algorithmic reasoning (pattern detection, nudges) operates on this history.
6. **Preferences are living data.** Onboarding captures initial preferences. Order history and chat interactions refine them over time.

---

## Command Model

| Command | Mode | Tools Available | Purpose |
|---------|------|----------------|---------|
| `/chat` | Chat mode (default) | Info-only: search, menu browse, coupons, order history, price comparison | Explore, compare, decide. Build a draft cart. |
| `/order` | Order mode | Transaction: cart push, checkout, payment, tracking + info tools | Finalize draft cart on chosen platform. Place real order. |

**Default:** New conversations start in chat mode. No explicit `/chat` needed unless switching back from order mode.

**Telegram home screen buttons** (`Order Food`, `Groceries`, `Book Table`, `Chat`) still work as before — they set `aria_phase` in session metadata. But `/chat` and `/order` are the primary interaction model.

---

## Revised User Flow

### Phase 0–2: Auto-Provision + Welcome + Onboarding

**No changes from PERSONIFI_SPEC.md.** `/start` triggers auto-provision, Aria greeting (from `GREETING.md`), and Q1–Q7 onboarding. Preferences stored in USER.md + `user_preferences` table (write-through).

### Phase 3: Home Screen

**No changes.** Telegram inline keyboard with 4 buttons. Buttons set `aria_phase` via `maybe_track_aria_phase()`.

### Phase 3.5 (Chat Mode) — The Primary Experience

**Trigger:** Default mode, or user sends `/chat`, or user presses Chat button.

**What the user can do:**
- Ask about restaurants, cuisines, deals
- Browse menus, compare prices across Swiggy and Zomato
- Get suggestions based on order history and preferences
- **Add items to a local draft cart** (persisted in DB, survives session restart)
- Remove/modify items in the draft cart
- See cart summary at any time ("what's in my cart?")
- Get proactive suggestions ("You usually get biryani on Fridays...")

**Tools exposed (info-only, ~12 tools):**
- `swiggy-food:search_restaurants`, `zomato:get_restaurants_for_keyword` (search)
- `swiggy-food:get_restaurant_menu`, `zomato:get_menu_items_listing` (menu browse)
- `swiggy-food:search_menu`, `zomato:get_restaurant_menu_by_categories` (menu search)
- `swiggy-food:fetch_food_coupons`, `zomato:get_cart_offers` (deal check)
- `swiggy-food:get_food_orders`, `zomato:get_order_history` (past orders)
- `swiggy-food:get_addresses`, `zomato:get_addresses` (address lookup)

**Tools NOT exposed:** Cart creation, order placement, checkout, tracking.

**Draft cart operations** (built-in Rust tool, not MCP):
- `draft_cart_add(item_name, restaurant_name, platform, variant, quantity, price)` — adds item
- `draft_cart_remove(item_index)` — removes item
- `draft_cart_view()` — returns current cart contents
- `draft_cart_clear()` — empties cart

**Suggestion engine (reads from):**
- USER.md preferences (diet, cuisines, budget)
- `order_events` table (what they ordered, when, from where)
- `detected_patterns` table (future: algorithmic patterns)
- Current conversation context

**Intent detection → order transition:**
When the user seems ready ("let's order", "place it", "checkout", "I'm done picking"), Aria suggests:
```
"Your cart has 2 items from Paradise (₹450 est). Ready to order?
 [/order to place it] [Keep browsing]"
```

### Phase 4 (Order Mode) — Transaction Execution

**Trigger:** User sends `/order`.

**Preconditions checked:**
1. Draft cart must have items. If empty: "Your cart is empty. Browse some restaurants first in /chat mode."
2. Auth tokens validated for the platform(s) in the cart. If expired: silent re-auth (phone from SecretsStore, ask only for OTP).

**What happens:**
1. Show draft cart summary with cross-platform price comparison (if items exist on both platforms)
2. User picks platform (or says "cheapest")
3. **Lock platform for this order**
4. Push draft cart items to platform cart:
   - Swiggy: `update_food_cart(restaurantId, cartItems, addressId)`
   - Zomato: `create_cart(res_id, items, address_id, payment_type)`
5. Fetch and apply best coupon
6. Show full bill breakdown — **explicit user confirmation required**
7. Place order
8. Track order
9. On order completion: extract order details → write to `order_events` table
10. Clear draft cart
11. Return to chat mode

**Tools exposed (transaction + info, ~20 tools):**
All chat mode tools PLUS:
- `swiggy-food:update_food_cart`, `zomato:create_cart` (cart push)
- `swiggy-food:get_food_cart` (cart review)
- `swiggy-food:apply_food_coupon` (coupon apply)
- `swiggy-food:place_food_order`, `zomato:checkout_cart` (order placement)
- `swiggy-food:track_food_order`, `zomato:get_order_tracking_info` (tracking)

**After order placed:** Mode automatically reverts to chat. User does not need to send `/chat`.

### Phase 5–6: Grocery + Table Booking

**No flow changes from PERSONIFI_SPEC.** These are triggered by home screen buttons or natural language. They do NOT use the draft cart (Instamart and Dineout have their own cart models). `/order` is food-only for now.

### Phase 7: Community Engine

**No flow changes from PERSONIFI_SPEC.** Social features feed into chat mode suggestions via ranking signals.

---

## Draft Cart Schema

**Storage:** Database table (not workspace markdown). Persists across sessions. Scoped by user_id.

### `draft_cart_items`

| Column | Type | Null | Constraints | Notes |
|---|---|---|---|---|
| id | TEXT | NO | PK | UUID |
| user_id | TEXT | NO | INDEX | IronClaw user ID (telegram:{id} for now) |
| restaurant_name | TEXT | NO | | Display name |
| restaurant_id | TEXT | YES | | Platform-specific ID (if known) |
| platform | TEXT | NO | | `swiggy-food` / `zomato` / `unknown` |
| item_name | TEXT | NO | | |
| item_id | TEXT | YES | | Platform-specific item ID |
| variant | TEXT | YES | | Size/variant description |
| quantity | INTEGER | NO | DEFAULT 1 | |
| unit_price | INTEGER | YES | | INR, if known from menu browse |
| added_at | TEXT | NO | DEFAULT now | ISO-8601 |
| metadata | TEXT | NO | DEFAULT '{}' | JSON: addons, customization notes |

**Indexes:** `idx_draft_cart_user(user_id)`, `idx_draft_cart_user_restaurant(user_id, restaurant_name)`

**Constraints:**
- One active cart per user (all items belong to same cart context)
- Multi-restaurant items allowed (comparison use case) — but at `/order` time, user must pick ONE restaurant
- Cart items have no expiry (user clears manually or on successful order)

**Lifecycle:**
- **Write:** `draft_cart_add` tool (built-in Rust, not MCP)
- **Read:** `draft_cart_view` tool, also read at `/order` entry
- **Delete:** `draft_cart_remove` (single item), `draft_cart_clear` (all items), auto-clear after successful order
- **No platform sync:** Draft cart lives entirely in IronClaw. Only pushed to platform at `/order` time.

---

## MCP Server Pre-Registration

**Requirement:** Swiggy and Zomato MCP servers must be configured at deployment time, not discovered at runtime.

**Current mechanism:** `~/.ironclaw/mcp-servers.json` loaded at startup (`src/tools/mcp/config.rs:412`). Also: `settings` table key `mcp_servers`.

**What must change:**
- Ship a default `mcp-servers.json` with Swiggy Food, Swiggy Instamart, Swiggy Dineout, and Zomato server configs pre-populated.
- Remove or suppress any "install MCP server" prompts from the agent's tool list when in Aria mode.
- The `extension_tools` (install/search/manage) should NOT be exposed to Aria users — they are developer tools.

**Implementation:** In `src/tools/registry.rs`, when registering tools, check if the deployment is Aria-mode (env var or config flag). If so, skip registration of `extension_install`, `extension_search`, `extension_remove`, etc.

---

## Token Reduction Strategy

### Problem
`dispatcher.rs:294` fetches ALL registered tool definitions every iteration. With 48+ tools (built-in + MCP), each tool definition is ~400-800 bytes of JSON schema. This consumes 2-5K tokens per LLM call just for tool definitions the agent will never use in the current phase.

### Solution: Phase-Gated Tool Filtering

**Where to implement:** `src/agent/dispatcher.rs` lines 293-313 (the `before_llm_call` method in `ChatDelegate`).

**Current flow:**
```
tool_defs = self.agent.tools().tool_definitions().await;  // ALL tools
tool_defs = attenuate_tools(&tool_defs, &active_skills);  // trust filter only
reason_ctx.available_tools = tool_defs;
```

**Proposed flow:**
```
tool_defs = self.agent.tools().tool_definitions().await;
tool_defs = attenuate_tools(&tool_defs, &active_skills);
tool_defs = phase_filter_tools(tool_defs, session.aria_phase());  // NEW
reason_ctx.available_tools = tool_defs;
```

**`phase_filter_tools` logic:**
- Read `aria_phase` from session metadata
- If `aria_phase` is set (Aria mode), filter tool_defs against a phase→tool allowlist
- If `aria_phase` is NOT set (generic IronClaw), pass through unchanged
- Allowlists defined as a `HashMap<AriaPhase, HashSet<String>>` built at startup

**Phase→Tool allowlists:**

| Phase | Allowed tool name prefixes | Estimated tool count |
|---|---|---|
| 0-2 (Onboarding) | None (pure conversation) | 0 MCP tools |
| 3 (Home) | None | 0 MCP tools |
| 3.5 (Chat) | `swiggy-food_search_`, `swiggy-food_get_restaurant_menu`, `swiggy-food_fetch_food_coupons`, `swiggy-food_get_food_orders`, `swiggy-food_get_addresses`, `zomato_get_restaurants_`, `zomato_get_menu_`, `zomato_get_cart_offers`, `zomato_get_order_history`, `zomato_get_addresses` + `draft_cart_*` | ~12 |
| 4 (Order) | Chat tools + `swiggy-food_update_food_cart`, `swiggy-food_get_food_cart`, `swiggy-food_apply_food_coupon`, `swiggy-food_place_food_order`, `swiggy-food_track_food_order`, `zomato_create_cart`, `zomato_checkout_cart`, `zomato_get_order_tracking_info` | ~20 |
| 5 (Grocery) | `swiggy-instamart_*` | ~8 |
| 6 (Table) | `swiggy-dineout_*` | ~8 |

**Token savings:** From ~48 tools (~3-5K tokens) to ~12 tools (~800-1200 tokens) in chat mode. ~60-75% reduction in tool definition tokens per LLM call.

**Built-in tools:** `echo`, `time`, `json`, `memory_*` are always available (they're small and universally useful). `shell`, `write_file`, `read_file` are NOT exposed to Aria users (they're developer tools).

---

## Conversation History and Future Algorithmic Work

### What Exists Today (no changes needed)
- `conversations` table — per-user, per-channel conversation metadata
- `conversation_messages` table — every user and assistant message, append-only
- `job_actions` table — every tool call with input/output
- `llm_calls` table — token/cost tracking

All conversation data already survives restart. Loaded via `list_conversation_messages` for LLM context.

### What Needs to Be Built (order_events extraction)
- After every successful food order, extract: restaurant, items, total, platform, timestamp → `order_events` table
- This is a Rust code hook in the post-order flow, NOT an LLM-driven write
- See `tasks/db/04-conversation-history.md` for schema

### Future Work (marked explicitly — not in current scope)
- **Pattern detection algorithm:** Query `order_events` for recurring patterns (day-of-week × cuisine, time × restaurant). Write results to `detected_patterns` table. Run as background routine or on-demand.
- **Chat suggestion engine:** Read `detected_patterns` + `user_preferences` + recent `order_events` to generate personalized suggestions during chat mode. Inject as context in system prompt, not as a tool call.
- **Conversation analysis:** NLP over `conversation_messages` to detect preference shifts, satisfaction signals, frequently discussed topics. Feed back into preference evolution.
- **MCP usage optimization:** Analyze `job_actions` to identify which tool call sequences are most effective for different user intents. Use to refine skill instructions.

---

## Preference Lifecycle

### Phase 1: Capture (Onboarding)
Q1-Q7 writes to USER.md + `user_preferences` table. Static until updated.

### Phase 2: Enrich (Post-Order) — Build Now
After each successful order, update `user_preferences`:
- Increment `order_count`
- Update `last_order_platform`, `last_order_restaurant`
- If ordered cuisine not in `cuisines[]`, consider adding (threshold: 3+ orders)

### Phase 3: Evolve (Algorithmic) — Future Work
- Detect diet changes from order history (user said "veg" but orders chicken regularly)
- Detect budget drift (average order total vs stated budget range)
- Surface preference conflicts in chat: "You said veg but you've been ordering chicken lately. Want me to update your preferences?"

---

## Summary of Changes from PERSONIFI_SPEC.md

| Area | PERSONIFI_SPEC says | This document says | Reason |
|---|---|---|---|
| Commands | Implicit (button-driven) | Explicit: `/chat` + `/order` only | User control, reduce confusion |
| Cart in chat mode | No cart tools | Draft cart (local, persisted) | Let users build decisions over time |
| Order flow entry | Button → full flow | `/order` → push draft cart | Separates decision from transaction |
| MCP registration | Implicit | Pre-registered at deployment | No user-facing "install" friction |
| Tool exposure | Spec says phase-gate, code doesn't | Phase-gated via `phase_filter_tools()` | Token reduction, focus |
| Grocery/table commands | Separate phases | Same flow via home buttons | Simplify command surface |
| Preference updates | Static (onboarding only) | Evolve post-order | Better suggestions over time |
| Algorithm on history | Proactive agent (Phase 3) | Marked as future work | Ship core flow first |
| Conversation storage | Not specified | Already works, document it | Acknowledge existing infrastructure |
