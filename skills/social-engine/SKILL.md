---
name: social-engine
version: "0.1.0"
description: >
  Community and social features for Personifi/Aria. Use this skill when the user
  wants to order with friends, split bills, see what friends are ordering, add
  friends, create squads, or do group orders. Also triggers when the agent needs
  to apply social ranking to restaurant search results. Handles friend management,
  hostel grouping, and social signals for food recommendations.
activation:
  keywords:
    - friends
    - group order
    - squad
    - split
    - hostel
    - together
    - group
    - social
    - split bill
    - share order
  patterns:
    - "(?i)(order|eat).{0,20}(together|with friends|with anyone|with my hostel)"
    - "(?i)(split|share).{0,20}(order|food|bill)"
    - "(?i)(add|remove|show).{0,20}friends"
    - "(?i)what.{0,20}friends.{0,20}(order|eat)"
  tags:
    - social
    - community
    - friends
    - group
  max_context_tokens: 1000
---

# Social Engine — Community Features

## Social Ranking (applies during food search)

Before presenting restaurant search results, check USER.md for social signals:

1. Read `USER.md → friends[]` — if non-empty, note which friends exist
2. Read `USER.md → residence` — used for hostel/PG peer grouping

**If friends list is populated:**
- Mention social signals naturally: "Popular with people in your area" or "Your friends have ordered here"
- Boost restaurants where friends have recently ordered (use order history if available)

**If friends list is empty:**
- Skip social signals entirely — present results by rating, distance, and price
- Don't mention the friends feature unless the user asks

## Friend Management

When user wants to add or remove friends:

**Add:** "Share their Telegram username and I'll add them."
- Store in `USER.md → friends: [@username]` via `memory_write`
- Normalize to @username format

**Remove:** Read current friends list, ask which to remove, update USER.md.

**Show:** Read `USER.md → friends[]`, display the list.

## Hostel/PG Grouping

Users with the same `residence` value in USER.md are considered peers.
This is used for 2nd-degree social signals ("trending in your hostel").

Currently, cross-user data is not available (tenant isolation). Hostel grouping
will be effective once aggregate analytics are implemented. For now, skip
hostel-level signals and use only the user's own friends list.

## Group Orders

When user wants to order with friends, hand off to the `group-order` skill.
Provide context: restaurant name, platform, address IDs from the current flow.

The `group-order` skill handles: session creation, friend invites, item
collection, cart consolidation, and UPI split summary.
