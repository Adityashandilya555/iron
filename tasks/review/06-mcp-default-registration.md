# MCP Servers Must Be Pre-Registered — No Install Prompts

**Status:** NEW TASK
**Area:** mcp, deployment, tool-registry
**Relates to:** tasks/USER_JOURNEY.md, tasks/review/03-skill-hardening-modified.md (tool filtering)

## Problem

Users should never see "install MCP server" or "search extension" prompts. Swiggy and Zomato MCP servers are infrastructure — they must be available the moment the bot starts.

## Current State

### MCP Config Loading (`src/tools/mcp/config.rs:412-547`)
- Reads from `~/.ironclaw/mcp-servers.json` (file-based) or `settings` table key `mcp_servers` (DB-based)
- If file doesn't exist or is empty → no MCP tools registered
- MCP tools discovered at startup via `list_tools()` RPC to each server

### Tool Registration (`src/app.rs:540-683`)
- `create_client_from_config()` creates MCP client per server
- Tools extracted from MCP capabilities, registered into global `ToolRegistry`
- Named as `{server_name}_{tool_name}` (e.g., `swiggy-food_search_restaurants`)

### Extension Tools Always Registered (`src/tools/registry.rs:476-486`)
- `extension_install`, `extension_search`, `extension_list`, `extension_remove`, `extension_enable`, `extension_disable`, `extension_info`, `extension_update`
- These are developer/admin tools. They let the LLM install new MCP servers at runtime.
- **Problem:** These are exposed to ALL users, including Aria Telegram users. The LLM may decide to suggest "let me install the Swiggy MCP server" if it doesn't find the tools.

## Required Changes

### 1. Ship Default MCP Config for Aria Deployments

**New file:** `config/aria-mcp-servers.json` (checked into repo, deployed alongside binary)

Contents (template — actual URLs from deployment):
```json
{
  "swiggy-food": {
    "url": "${SWIGGY_FOOD_MCP_URL}",
    "transport": "http",
    "oauth": { "enabled": true },
    "enabled": true,
    "description": "Swiggy Food delivery — search, menu, cart, order, track"
  },
  "swiggy-instamart": {
    "url": "${SWIGGY_INSTAMART_MCP_URL}",
    "transport": "http",
    "oauth": { "enabled": true },
    "enabled": true,
    "description": "Swiggy Instamart groceries — search, cart, checkout, track"
  },
  "swiggy-dineout": {
    "url": "${SWIGGY_DINEOUT_MCP_URL}",
    "transport": "http",
    "oauth": { "enabled": true },
    "enabled": true,
    "description": "Swiggy Dineout table booking — search, slots, book"
  },
  "zomato-mcp-server": {
    "url": "${ZOMATO_MCP_URL}",
    "transport": "http",
    "oauth": { "enabled": true },
    "enabled": true,
    "description": "Zomato — search, menu, cart, order, track"
  }
}
```

### 2. MCP Config Resolution Order (`src/tools/mcp/config.rs`)

**Current:** file → DB fallback
**Proposed:** bundled default → file override → DB override

Add a `load_bundled_mcp_config()` that reads from a compile-time or deployment-time default. User's `~/.ironclaw/mcp-servers.json` merges on top (can override URLs, disable servers). DB settings merge on top of that.

### 3. Suppress Extension Tools in Aria Mode (`src/tools/registry.rs:476-486`)

**Current:** Extension tools always registered.
**Proposed:** Check for Aria mode (env var `ARIA_MODE=true` or config flag). If Aria mode:
- Do NOT register `extension_install`, `extension_search`, `extension_remove`, `extension_update`
- Keep `extension_list`, `extension_info` (read-only, useful for debugging)
- This prevents the LLM from ever suggesting "let me install an MCP server"

**Alternative (lighter touch):** Don't suppress registration, but add these tool names to the phase-gated denylist so they're never included in `available_tools` for any Aria phase. This is simpler and doesn't require an Aria mode flag.

### 4. Suppress Developer Tools in Aria Mode

Same logic for tools that Aria Telegram users should never see:
- `shell`, `read_file`, `write_file`, `list_dir`, `apply_patch` (dev tools)
- `create_job`, `list_jobs`, `job_status`, `cancel_job` (job tools — Aria uses chat, not jobs)
- `image_generate`, `image_edit` (not relevant to food ordering)
- `skill_install`, `skill_remove` (admin tools)

**Implementation:** Add a `ARIA_DENYLIST` constant in `src/tools/registry.rs`:
```rust
const ARIA_DENYLIST: &[&str] = &[
    "shell", "read_file", "write_file", "list_dir", "apply_patch",
    "create_job", "list_jobs", "job_status", "cancel_job",
    "extension_install", "extension_search", "extension_remove", "extension_update",
    "skill_install", "skill_remove",
    "image_generate", "image_edit", "image_analyze",
];
```

Use `tool_definitions_excluding(&ARIA_DENYLIST)` in the dispatcher when `aria_phase` is set in session metadata. This is the lightest-touch approach — no Aria mode flag needed, just check session metadata.

## Files That Need Changes

| File | Change | Type |
|---|---|---|
| `config/aria-mcp-servers.json` | Default MCP config template | new file |
| `src/tools/mcp/config.rs` | Bundled config loading + merge logic | modify |
| `src/agent/dispatcher.rs:293-313` | Use `tool_definitions_excluding()` with Aria denylist when `aria_phase` is set | modify |
| `src/tools/registry.rs` | Add `ARIA_DENYLIST` constant | additive |

## Token Impact

Suppressing ~15 irrelevant tools saves ~1-2K tokens per LLM call for Aria users. Combined with phase-gated MCP tool filtering (tasks/review/03-skill-hardening-modified.md), total savings is ~3-5K tokens per call.

## Coordination Notes

- Phase-gated tool filtering (tasks/review/03-skill-hardening-modified.md) and this task are complementary. This task removes tools that should NEVER be seen by Aria users. Phase-gating removes MCP tools not relevant to the current phase.
- The bundled MCP config must have correct URLs for the deployment environment. Use env var substitution or a config generation step in the deployment pipeline.
