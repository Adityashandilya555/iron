# Skill-to-Rust Hardening and Deterministic Phase Control — MODIFIED

**Status:** REVISED (supersedes 03-skill-hardening.md)
**Changes from original:**
1. Only TWO user commands: `/chat` and `/order` (removed `/active`, `/groceries`, `/table`, `/home`)
2. Added phase-gated tool filtering implementation detail (dispatcher.rs integration)
3. Added draft cart tool awareness
4. Added Aria denylist for developer/admin tools
5. Connected to tasks/USER_JOURNEY.md as source of truth

---

## What the Original Task Got Right

- Phase detection relies on LLM inference — no code enforcement (still true)
- `maybe_track_aria_phase()` writes metadata that nothing reads (still true)
- `ToolRegistry` returns all tools regardless of phase (still true)
- Skills are keyword-matched, not phase-gated (still true)
- Callback buttons set `aria_phase` in session metadata (still true, still the only code-level phase tracking)

## What the Original Task Got Wrong

- Proposed 5 slash commands (`/active`, `/chat`, `/groceries`, `/table`, `/home`). User wants only 2: `/chat` and `/order`.
- Did not address the draft cart concept (chat mode builds cart, `/order` executes it).
- Did not address developer tool suppression for Aria users.
- Phase C (hot-path Rust hardening) is deprioritized — the MCP servers ARE the tools, skills provide orchestration. Composite Rust tools are premature optimization.

---

## Required Changes

### Phase A — Two Deterministic Commands

#### `src/agent/submission.rs`

**Add new `Submission` variant:**
```rust
AriaMode { mode: AriaCommandMode }
```

**New enum:**
```rust
pub enum AriaCommandMode {
    Chat,   // /chat — switch to chat mode (phase 3.5)
    Order,  // /order — switch to order mode (phase 4.0)
}
```

**Parse rules (add before the `SystemCommand` checks):**
- `/chat` → `Submission::AriaMode { mode: AriaCommandMode::Chat }`
- `/order` → `Submission::AriaMode { mode: AriaCommandMode::Order }`

**No other Aria commands.** Grocery (phase 5) and table booking (phase 6) are triggered by home screen buttons (callback_query) or natural language, not slash commands.

#### `src/agent/agent_loop.rs` — handle_message match block

**Add handler for `Submission::AriaMode`:**

```rust
Submission::AriaMode { mode } => {
    match mode {
        AriaCommandMode::Chat => {
            session.lock().await.set_aria_phase(3.5);
            Ok(Some("Switched to chat mode. Browse restaurants, compare prices, build your cart.".into()))
        }
        AriaCommandMode::Order => {
            // Check draft cart
            let cart_items = store.list_draft_cart_items(&message.user_id).await?;
            if cart_items.is_empty() {
                Ok(Some("Your cart is empty. Browse some restaurants in /chat mode first, then /order when you're ready.".into()))
            } else {
                session.lock().await.set_aria_phase(4.0);
                // Inject cart summary into the next LLM context so the agent
                // starts the checkout flow with cart awareness
                let cart_summary = format_draft_cart(&cart_items);
                // Process as user input with cart context prepended
                self.process_user_input(message, tenant, session, thread_id,
                    &format!("[User switched to order mode. Draft cart:\n{}]\nI want to place this order.", cart_summary)
                ).await
            }
        }
    }
}
```

**Key difference from original task:** `/order` is not just a phase switch. It reads the draft cart and injects it as context for the LLM to start the checkout flow.

### Phase B — Phase-Gated Tool Filtering

#### `src/agent/dispatcher.rs` (lines 293-313, `before_llm_call`)

**Current code (line 294):**
```rust
let tool_defs = self.agent.tools().tool_definitions().await;
```

**Modified code:**
```rust
let tool_defs = self.agent.tools().tool_definitions().await;

// Apply Aria-mode tool filtering if session has an aria_phase
let tool_defs = {
    let sess = self.session.lock().await;
    if let Some(phase) = sess.aria_phase() {
        crate::tools::aria_filter::filter_tools_for_phase(tool_defs, phase)
    } else {
        tool_defs
    }
};
```

#### New file: `src/tools/aria_filter.rs`

**Purpose:** Phase-gated tool allowlist for Aria mode. Only consulted when `aria_phase` is set in session metadata. Non-Aria IronClaw deployments are completely unaffected.

**Contents:**
- `ARIA_DENYLIST` — tools NEVER shown to Aria users (dev tools, extension tools, job tools)
- `CHAT_ALLOWLIST` — MCP tool name prefixes allowed in chat mode
- `ORDER_ALLOWLIST` — MCP tool name prefixes allowed in order mode (superset of chat)
- `GROCERY_ALLOWLIST`, `TABLE_ALLOWLIST` — for phases 5, 6

**`filter_tools_for_phase(tools, phase) -> Vec<ToolDefinition>`:**
1. Remove all tools in `ARIA_DENYLIST`
2. For MCP tools (names containing `_` with known server prefixes): keep only those matching the phase allowlist
3. Always keep: `draft_cart`, `memory_*`, `echo`, `time`, `swiggy_auth`, `zomato_auth`
4. Return filtered list

**Registration:** Add `pub mod aria_filter;` to `src/tools/mod.rs`.

### Phase C — Hot-Path Rust Hardening (DEPRIORITIZED)

The original task proposed `food_search.rs` and `food_cart.rs` composite tools. These are **deprioritized** because:
1. MCP servers ARE the tools. Wrapping them adds a translation layer without clear value.
2. The draft cart tool handles the local cart. Platform carts are managed by MCP tools directly.
3. Skills provide orchestration. The LLM follows skill instructions to call tools in the right order.

**If hot-path hardening becomes needed later:** Start with search (highest frequency, parallelizable across platforms) and auth (already done for Swiggy/Zomato).

---

## Spec Impact

The following additions should be noted in PERSONIFI_SPEC.md or USER_JOURNEY.md:
- `/chat` and `/order` as the two Aria-specific Telegram commands
- Draft cart as a bridge between chat mode and order mode
- Phase-gated tool filtering as the token reduction mechanism

## Files That Need Changes

| File | Change | Type |
|---|---|---|
| `src/agent/submission.rs` | `AriaCommandMode` enum + `/chat`, `/order` parsing | additive |
| `src/agent/agent_loop.rs` | Handler for `Submission::AriaMode` | additive |
| `src/tools/aria_filter.rs` | Phase-gated tool filtering logic | new file |
| `src/tools/mod.rs` | Export `aria_filter` module | additive |
| `src/agent/dispatcher.rs:293-313` | Call `filter_tools_for_phase` in `before_llm_call` | modify |
| `src/agent/session.rs` | No change — `aria_phase()` and `set_aria_phase()` already exist | none |

## Coordination Notes

- `tasks/db/06-draft-cart.md` must land first — `/order` handler reads draft cart items.
- `tasks/review/06-mcp-default-registration.md` provides the Aria denylist constant. Coordinate to avoid duplication.
- `maybe_track_aria_phase()` (agent_loop.rs:992) should be updated to also handle `/chat` and `/order` mode switches from the command handler, OR the command handler sets phase directly (proposed approach above).
