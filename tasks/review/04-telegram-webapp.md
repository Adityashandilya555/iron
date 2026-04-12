# Telegram WebApp for Social Friends Management

## Hypothesis

A Telegram WebApp button should open an in-chat web UI for managing friends, replacing/augmenting onboarding Q7. This is a new feature, not a bug.

## Evidence (files + line numbers)

- No `src/channels/telegram/` directory exists. Telegram is implemented as a WASM channel.
- `src/channels/wasm/` — Telegram implemented as a compiled WASM module, not native Rust.
- `src/channels/wasm/setup.rs:1-60` — WASM channels loaded from directory, registered with webhook routes, injected with credentials. Telegram is one such WASM channel.
- `src/channels/wasm/host.rs` — Host functions exposed to WASM channel modules. WebApp-specific handling would need to be added here.
- `src/channels/web/server.rs` — Web gateway server. Could serve WebApp HTML/JS as a static route.
- `src/channels/web/handlers/static_files.rs` — Static file serving handler. WebApp frontend could live here.
- `src/agent/agent_loop.rs:984-1019` — `maybe_track_aria_phase()` already handles `callback_query` from Telegram inline keyboards. `web_app_data` is a different Telegram update type with no current handler.
- `PERSONIFI_SPEC.md:67-69` — Q7: "Add friends (optional)" stores `friends[]` in USER.md.
- `PERSONIFI_SPEC.md:257-299` — Phase 7 Community Engine: friends stored as Telegram user IDs in USER.md, social graph for ranking.
- `src/workspace/seeds/IDENTITY.md` — No friends-specific data structure beyond USER.md.
- `agent-config/agents-manifest.md:63` — Phase 2 Onboarding: Q7 (friends) is skippable. WebApp would provide a richer alternative.

## Root Cause

This is a new feature. The current architecture has no WebApp support:

1. **No `web_app_data` handler.** Telegram sends `web_app_data` as a message update type when a WebApp submits data. The WASM Telegram channel needs to parse this and forward it as an `IncomingMessage` with appropriate metadata.

2. **No WebApp button emission.** Telegram inline keyboards support a `web_app` button type (`{"text": "Manage Friends", "web_app": {"url": "https://..."}}`). The agent needs to include this in the home screen keyboard or during onboarding Q7.

3. **No WebApp frontend.** A small HTML/JS app needs to be built and served at a public URL. Telegram validates the URL domain against the bot's configured WebApp domain.

4. **No auth binding.** When a WebApp opens, Telegram provides `initData` containing the user's Telegram ID, signed with the bot token (HMAC-SHA256). IronClaw needs to validate this signature and map it to the correct user session/workspace.

## Required Changes (specific files and what to change)

### WASM Telegram channel module (compiled binary — not in Rust src/)
- **Current:** Handles `message` and `callback_query` update types.
- **Required:** Add handling for `web_app_data` update type. Parse the submitted data and create an `IncomingMessage` with `metadata.type = "web_app_data"` and the payload in `metadata.web_app_data`.
- **Change type:** additive (in WASM channel source, not Rust)
- **Ripple:** Requires rebuilding the WASM channel binary.

### `src/channels/wasm/host.rs`
- **Required:** Expose any new host functions needed by the WASM channel for WebApp button construction (e.g., `emit_webapp_button`).
- **Change type:** additive

### `src/agent/agent_loop.rs`
- **Current:** `maybe_track_aria_phase()` handles `callback_query` only.
- **Required:** Add `maybe_handle_webapp_data()` method that detects `metadata.type == "web_app_data"`, parses the friends list from the payload, and writes to USER.md via workspace. Also update the social graph DB (see `tasks/db/03-social-graph.md`).
- **Change type:** additive

### New: `src/channels/web/handlers/webapp_auth.rs`
- **Purpose:** Validate Telegram `initData` HMAC-SHA256 signature (bot token as key). Map Telegram user ID to IronClaw user session. Return a short-lived session token for WebApp API calls.
- **Change type:** additive (new file)
- **Ripple:** Needs access to bot token from SecretsStore (currently in WASM channel credentials).

### `src/channels/web/server.rs` or new `src/channels/web/handlers/webapp.rs`
- **Required:** Serve the friends management WebApp HTML/JS at a dedicated route (e.g., `/webapp/friends`). Register the auth endpoint (`/webapp/auth`).
- **Change type:** additive

### New: WebApp frontend (`src/channels/web/static/webapp/friends/`)
- **Purpose:** Small HTML/JS page for adding/removing friends. Uses `window.Telegram.WebApp` SDK for init, auth, and close. Makes API calls to IronClaw backend using the session token from `/webapp/auth`.
- **Change type:** additive (new static files)

## Spec Impact (does this force a PERSONIFI_SPEC.md change?)

Yes — additive only:
- Phase 2 Q7 should note: "Friends can also be managed via the WebApp button on the home screen."
- Phase 3 Home Screen should add a "Manage Friends" WebApp button to the inline keyboard.
- Phase 7 Community Engine should note the WebApp as the primary friends management UI (replaces the current text-based Q7 flow for returning users).

## Coordination Notes

- **database-specialist owns the friends/social-graph schema.** The current approach of storing `friends[]` as Telegram user IDs in USER.md markdown is not queryable for Phase 7 social ranking. A proper relational schema is needed. Reference: `tasks/db/03-social-graph.md`.
- The WASM Telegram channel module is a compiled binary. Changes to handle `web_app_data` require rebuilding the WASM module. Locate the WASM channel source before estimating scope.
- Telegram requires the WebApp domain to be configured in BotFather. The tunnel URL must be stable or bot settings updated on each deployment — prefer a fixed domain.
- The `initData` validation bot token lives in the WASM channel's credentials. The Rust-side `/webapp/auth` handler needs access to this same secret — coordinate how to share it (SecretsStore key vs env var).
