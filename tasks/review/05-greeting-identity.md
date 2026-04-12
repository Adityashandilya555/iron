# Bootstrap Greeting vs Aria Identity — Status: Fixed, Minor Cleanup Remains

## Hypothesis

IronClaw's default greeting was sent on first contact instead of Aria's greeting from agent-config/IDENTITY.md.

## Evidence (files + line numbers)

- `src/agent/agent_loop.rs:41` — `const BOOTSTRAP_GREETING: &str = include_str!("../workspace/seeds/GREETING.md");` — Greeting is statically included from a seed file at compile time.
- `src/workspace/seeds/GREETING.md` (content) — `"Yo! [wave emoji] I'm Aria -- your food & hangout buddy for Bangalore.\n\nI can order food (Swiggy + Zomato), grab groceries (Instamart), and book tables at restaurants. Let me get to know you real quick so I can make better picks.\n\nWhat's your name?"` — This IS the Aria greeting, not a generic IronClaw greeting.
- `src/workspace/seeds/IDENTITY.md` — Full Aria identity. Seeded into per-user workspace at `seed_if_empty()`.
- `src/workspace/mod.rs:1624-1664` — `seed_if_empty()` seeds IDENTITY.md, SOUL.md, AGENTS.md, USER.md, and conditionally BOOTSTRAP.md. Sets `bootstrap_pending` flag.
- `src/agent/agent_loop.rs:438-463` — On startup (owner workspace), if `bootstrap_pending` is true, persists `BOOTSTRAP_GREETING` to DB and broadcasts to the `"gateway"` channel.
- `src/agent/agent_loop.rs:1317-1345` — Per-user path: in `handle_message`, calls `tenant_ctx()` which creates per-user workspace and calls `seed_if_empty()`. If `take_bootstrap_pending()` returns true, persists greeting and broadcasts to `message.channel` (i.e., `"telegram"`). This happens **before** `process_user_input` (line 1350), so the greeting is sent before the LLM processes the user's first message.
- Recent commit `988793ec` — "fix(tests): update BOOTSTRAP_GREETING_MARKER and greeting assertions for Aria persona" — Fixed test assertions to match the Aria greeting, confirming greeting content was recently changed from IronClaw default to Aria.
- Recent commit `1e06ee4a` — "feat(personifi): add skills, zomato auth, seed updates, and MCP config" — Likely where GREETING.md was updated to Aria persona.

## Root Cause

**The issue is fixed.** Evidence:

1. `GREETING.md` contains Aria's personality-correct greeting (casual, Bangalore-local, asks "What's your name?" which is onboarding Q1).
2. `BOOTSTRAP_GREETING` is included at compile time from this file and used in both the owner-bootstrap path (line 454) and the per-user bootstrap path (line 1338).
3. `IDENTITY.md` is seeded into the workspace by `seed_if_empty()` (line 1628) before the LLM processes any user input, so the system prompt includes Aria's identity from the first turn onward.
4. The greeting is sent as a static string (no LLM call). IDENTITY.md doesn't need to be loaded for the greeting — it just needs GREETING.md to contain the right text.
5. Commit `988793ec` explicitly updated test assertions to match the Aria persona.

## Remaining Issues

### Issue A: Dual bootstrap path (minor)
- `src/agent/agent_loop.rs:438-463` — Owner-bootstrap path sends greeting to `"gateway"` channel with user `"default"`.
- `src/agent/agent_loop.rs:1317-1345` — Per-user bootstrap path sends to `message.channel` (correct for Telegram).
- **Risk:** For the owner user (the first account), both paths may fire, causing a duplicate greeting. The per-user path is correct. The owner-bootstrap path is probably dead code for a Telegram-first deployment where no user sends to `"gateway"`.

### Issue B: Compile-time constant (design note, not a bug)
- `BOOTSTRAP_GREETING` is `include_str!` — cannot be customized per-user or per-deployment without recompilation.
- This is the correct design for speed, but note it if greeting personalization is ever needed.

## Required Changes (specific files and what to change)

### `src/agent/agent_loop.rs` (lines 438-463) — Optional cleanup
- **Current:** Owner-bootstrap sends greeting to `"gateway"` channel, which is not the Telegram channel.
- **Required (optional):** Remove the owner-bootstrap path or broadcast to all registered channels. The per-user path (lines 1317-1345) handles the correct case. Removing the owner-bootstrap eliminates the duplicate greeting risk.
- **Change type:** optional refactor / cleanup

### No other changes needed.

## Spec Impact (does this force a PERSONIFI_SPEC.md change?)

No. The spec (Phase 1, "Send hardcoded greeting with Bangalore-local personality") matches the current implementation exactly.

## Coordination Notes

- No database changes needed.
- The owner-bootstrap vs per-user-bootstrap dual path is the only remaining concern. If left as-is, it's harmless for Telegram users (owner-bootstrap message goes to `"gateway"`, not Telegram). Only worth cleaning up if it causes test noise or confusion.
