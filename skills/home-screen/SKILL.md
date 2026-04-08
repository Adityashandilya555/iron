---
name: home-screen
version: 0.1.0
description: >
  Handles the Aria home screen (Phase 3). Activated when the user is at the home screen,
  presses a home screen button, returns from an ordering flow, or sends a greeting to a
  returning-user session. Routes button callback data (order_food, groceries, book_table,
  chat_mode) to the correct phase. Also handles "back to home" and "main menu" requests.
activation:
  patterns:
    - "order_food"
    - "groceries"
    - "book_table"
    - "chat_mode"
    - "back to home"
    - "main menu"
    - "start over"
  keywords:
    - "home"
    - "menu"
    - "back"
    - "start"
  tags:
    - "home"
    - "phase3"
    - "routing"
  max_context_tokens: 2000
---

# Home Screen — Phase 3

## Purpose

Show the Phase 3 home screen and route button presses to the correct phase.
**No MCP tools are loaded in this phase** — this is pure UI routing.

## Returning User: First Message of Session

When USER.md has `onboarding_completed` set AND no ordering conversation has
happened yet in this session:

1. Read USER.md to get `name`
2. Send the home screen with their name personalised:

```
Hey {name}! 👋 What are you in the mood for today?

🍔 Order Food — Swiggy + Zomato, best deals
🛒 Groceries — Instamart, 10-min delivery
🍽 Book Table — Dineout deals, instant booking
💬 Chat — Ask me anything about food in Bangalore
```

Telegram inline keyboard (2 rows):
```
[🍔 Order Food](order_food) [🛒 Groceries](groceries)
[🍽 Book Table](book_table) [💬 Chat](chat_mode)
```

**Skip the home screen** if the user's first message already has clear ordering
intent (e.g., "order biryani", "book a table at Toit") — route directly to that phase.

## Button Callback Routing

When the user presses a home screen button, Telegram sends a `callback_query`.
Route immediately based on the callback data:

| Callback data | Phase | First thing to say |
|---------------|-------|--------------------|
| `order_food` | Phase 4 (Food Ordering) | "What are you craving? Tell me a restaurant, cuisine, or dish." |
| `groceries` | Phase 5 (Grocery Ordering) | "What do you need? I'll check Instamart." |
| `book_table` | Phase 6 (Table Booking) | "Where do you want to dine? Tell me the restaurant or area in Bangalore." |
| `chat_mode` | Phase 3.5 (Chat Mode) | "Ask away! What do you want to know about food, places, or deals?" |

**On button press:**
- Do NOT ask "are you sure?" — button press = confirmed intent
- Do NOT show the home screen again — jump straight into the phase
- Acknowledge briefly then start the flow

## Return to Home

When user says "home", "back", "main menu", "start over", or equivalent:
1. Unload any active phase tools (no MCP tools in Phase 3)
2. Show the home screen message with inline keyboard again
3. Reset any active ordering context (don't carry over cart state)

## Proactive Nudges (Before Buttons)

If `ORDER_PATTERNS.md` exists and has data, show ONE pattern-based nudge ABOVE
the inline keyboard buttons. Keep it short — one line max.

Examples:
- "Friday again! Last week: Paradise Biryani 🍚 — want a repeat?"
- "You usually order South Indian on Mondays. Meghana? 🌿"
- "Late-night craving detected 😏 — Domino's is open."

Always offer a way to ignore: show the normal home screen below the nudge.
Do NOT show nudges if ORDER_PATTERNS.md is missing or empty.

## Phase Tracking

The selected phase is already stored in `session.metadata["aria_phase"]` by the
time this skill runs (the Rust dispatcher sets it from the callback data before
the LLM is called). You do not need to write phase state anywhere — just route.

## Tone

- Casual, warm — Bangalore college student energy
- Don't say "Please select an option" — that's corporate
- First message of session for returning user should feel like a quick re-connect,
  not a formal greeting
