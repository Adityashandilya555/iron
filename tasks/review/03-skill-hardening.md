# Skill-to-Rust Hardening and Deterministic Phase Control

## Hypothesis

MCP orchestration is entirely in SKILL.md files with no Rust enforcement. Phase detection relies on LLM inference. Required: (a) hot-path hardening into Rust, (b) `/active` and `/chat` slash commands for deterministic phase control, (c) a tool registry the agent consults guided by SOUL.md and IDENTITY.md.

## Evidence (files + line numbers)

- `skills/mcp-orchestrator/SKILL.md:1-93` — Meta-skill providing platform registry, cross-platform search strategy, address resolution. All in markdown. No code enforcement of phase gates.
- `skills/food-ordering/SKILL.md:1-166` — Complete food ordering flow (auth, search, menu, cart, coupons, order, tracking). All in markdown instructions to the LLM.
- `src/agent/agent_loop.rs:984-1019` — `maybe_track_aria_phase()`: Telegram callback buttons (`order_food`, `groceries`, `book_table`, `chat_mode`) set `session.metadata["aria_phase"]` to a float. This is the **only** code-level phase tracking.
- `src/agent/session.rs` — `set_aria_phase(phase: f64)` stores phase in session metadata. But nothing reads this phase to gate tool access.
- `src/agent/CLAUDE.md:125` — "Skills are selected deterministically (no LLM call) — see `skills/selector.rs`." Skills are chosen by keyword matching, not by phase state.
- `src/tools/registry.rs:1-80` — `ToolRegistry` manages all tools. No phase-gating logic. All registered tools available to the LLM at all times.
- `src/agent/submission.rs` — No `/active` or `/chat` commands exist. Parser handles `/undo`, `/redo`, `/compact`, etc., but no phase-switching commands.
- `.claude/agent-config/agents-manifest.md:274` — "On transition: Unload old phase tools, load new phase tools. Never have both active." This spec requirement is **not enforced** in code.
- `src/agent/dispatcher.rs` — Injects skill context into LLM prompt but does not filter the tool list based on phase.

## Root Cause

Fundamental gap between spec and implementation: agents-manifest.md defines strict phase-gated tool exposure (lines 26-275), but the runtime exposes all registered tools to the LLM regardless of phase. The only phase-awareness in code is `maybe_track_aria_phase()` which writes metadata that nothing reads.

Skills provide orchestration instructions as LLM prompt text, but the LLM can ignore them. There is no enforcement layer.

## Required Changes (specific files and what to change)

### Phase A — Deterministic Phase Commands

#### `src/agent/submission.rs`
- **Current:** No `/active` or `/chat` commands.
- **Required:** Add `Submission::PhaseSwitch { phase: AriaPhase }` variant. Parse:
  - `/active` → Phase 4 (food ordering)
  - `/chat` → Phase 3.5 (chat mode)
  - `/groceries` → Phase 5
  - `/table` → Phase 6
  - `/home` → Phase 3
- **Change type:** additive
- **Ripple:** `agent_loop.rs` handle_message match block needs a new arm for `PhaseSwitch`.

#### `src/agent/agent_loop.rs`
- **Current:** `maybe_track_aria_phase()` only triggers on Telegram callback_query.
- **Required:** Add a handler for `Submission::PhaseSwitch` that calls `session.set_aria_phase()` and responds with a confirmation message. Reuse the same phase-setting logic.
- **Change type:** additive

### Phase B — Phase-Gated Tool Filtering

#### `src/tools/registry.rs`
- **Current:** `ToolRegistry` returns all tools for LLM tool definitions.
- **Required:** Add a `tools_for_phase(phase: f64) -> Vec<ToolDefinition>` method that filters tools based on the agents-manifest.md mapping. Requires a phase-to-tools mapping data structure (e.g., `HashMap<OrderedFloat<f64>, HashSet<String>>` built from a config file or hardcoded from the manifest).
- **Change type:** additive
- **Ripple:** `dispatcher.rs` must call `tools_for_phase()` instead of listing all tools when building the LLM request.

#### `src/agent/dispatcher.rs`
- **Current:** Builds LLM request with all available tool definitions.
- **Required:** Read `aria_phase` from session metadata. Pass to `ToolRegistry::tools_for_phase()` to get the phase-appropriate subset. Only include those in the LLM request.
- **Change type:** refactor
- **Ripple:** Affects all LLM calls for Aria users. Non-Aria IronClaw usage should not be affected — gate behind a check for `aria_phase` presence in session metadata.

### Phase C — Hot-Path Rust Hardening (after Phase B)

**Priority order:** OTP auth flows (already done for Zomato/Swiggy) → search (highest frequency) → cart management (money safety per SOUL.md) → checkout.

#### New: `src/tools/builtin/food_search.rs` (composite tool)
- **Purpose:** Wraps `swiggy:search_restaurants` + `zomato:get_restaurants_for_keyword` into a single `search_food` tool. Runs both in parallel, merges/deduplicates results, applies USER.md preferences, returns unified response.
- **Maps from:** `skills/food-ordering/SKILL.md` sections 2-3 (Address Resolution + Search).
- **Change type:** additive (new file)

#### New: `src/tools/builtin/food_cart.rs` (composite tool)
- **Purpose:** Wraps cart operations with cross-platform comparison logic.
- **Maps from:** `skills/food-ordering/SKILL.md` sections 6-7 (Comparison + Cart).
- **Change type:** additive (new file)

## Spec Impact (does this force a PERSONIFI_SPEC.md change?)

Partially. The spec states "Phase-gated tool exposure" as a design decision, and slash commands are not mentioned. Adding `/active`, `/chat`, `/groceries`, `/table`, `/home` is an additive UX change that should be documented under Phase 3 (Home Screen) as an alternative to button presses.

agents-manifest.md Phase 3.5 says `trigger: "User presses Chat OR sends free-form message at home screen"`. Adding `/chat` as a trigger is compatible but should be noted in the spec.

## Coordination Notes

- Phase-gated tool filtering must be configurable (not hardcoded) so non-Aria IronClaw deployments are unaffected.
- The food search composite tool needs access to MCP clients for both Swiggy and Zomato. Check how `McpTool` instances are currently created in `src/tools/mcp/`.
- No database changes needed — this is runtime-only.
