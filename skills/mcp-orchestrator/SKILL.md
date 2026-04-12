---
name: mcp-orchestrator
version: 0.2.0
description: >
  Master orchestration skill for Personifi/Aria. Provides platform registry,
  cross-platform search strategy, and address resolution for multi-platform
  food ordering. Use when the agent needs to route between Swiggy and Zomato,
  resolve addresses across platforms, or handle cross-platform errors.
  Phase detection and tool exposure rules live in AGENTS.md (always loaded).
  Per-phase tool sequences live in the phase-specific skills.
activation:
  patterns:
    - "order food"
    - "order groceries"
    - "book a table"
    - "order with friends"
  keywords:
    - "swiggy"
    - "zomato"
    - "instamart"
    - "dineout"
    - "restaurant"
    - "food"
    - "groceries"
    - "biryani"
    - "pizza"
    - "hungry"
    - "order"
    - "delivery"
    - "booking"
    - "table"
  tags:
    - "food"
    - "ordering"
    - "routing"
    - "mcp"
  max_context_tokens: 1500
---

# MCP Orchestrator — Platform Registry & Cross-Platform Strategy

Phase detection, tool exposure rules, and auth patterns are in AGENTS.md (always loaded).
This skill provides the platform-specific details that phase skills need.

## Platform Registry

| Platform | MCP Server | Auth Token Key | Shared Auth? |
|----------|-----------|----------------|--------------|
| Swiggy Food | `swiggy-food` | `mcp_swiggy-food_access_token` | Yes (all 3 Swiggy) |
| Swiggy Instamart | `swiggy-instamart` | `mcp_swiggy-instamart_access_token` | Yes |
| Swiggy Dineout | `swiggy-dineout` | `mcp_swiggy-dineout_access_token` | Yes |
| Zomato | `zomato-mcp-server` | `mcp_zomato-mcp-server_access_token` | No (separate) |

One Swiggy login covers Food + Instamart + Dineout. Zomato has its own auth.

## Address Resolution

Swiggy and Zomato use different address ID systems:
- **Swiggy:** `get_addresses()` returns `{ id: "cutd1devqekb5je3fclg", addressLine: "..." }`
- **Zomato:** `get_saved_addresses_for_user()` returns `{ address_id: "751200265", location_name: "..." }`

On first ordering request, call BOTH address endpoints in parallel. Match to USER.md residence. Cache the pair for the session.

## Cross-Platform Search (Food — Phase 4)

Fire both searches in parallel:
1. `swiggy-food:search_restaurants(addressId, query)`
2. `zomato-mcp-server:get_restaurants_for_keyword(address_id, keyword)`

Merge results: match by restaurant name (fuzzy) + locality. De-duplicate.
- Same restaurant on both → show both prices, let user pick
- Unique to one → show with platform badge (🟠 Swiggy / 🔴 Zomato)
- If one search fails → use the other (graceful degradation)

Apply USER.md preferences: `diet` filter, `budget` filter, `cuisines` boost.

## Error Handling

| Error | Action |
|-------|--------|
| MCP server unreachable | Fall back to other platform; if both down, tell user |
| Auth token expired | Re-auth with USER.md phone, ask OTP only, resume |
| Restaurant closed | Suggest open alternatives from same search |
| Item out of stock | Suggest similar items from same category |
| Payment failed | Show error, suggest retry or different method |

## Phase Transitions

When user switches intent mid-conversation (e.g., "actually book a table at Toit"):
- Detect the shift instantly
- Switch to the new phase's flow
- Don't ask "do you want to switch?" — just do it
