---
name: group-order
version: "0.1.0"
description: Coordinate group food orders — invite friends or hostel peers, collect selections, and generate a split payment summary with UPI phone numbers.
activation:
  keywords:
    - group order
    - split order
    - join order
    - group food
    - order together
    - split the bill
    - hostel order
  patterns:
    - "(?i)(order|eat).{0,20}(together|with friends|with anyone|with my hostel)"
    - "(?i)(split|share).{0,20}(order|food|bill)"
    - "(?i)anyone.{0,20}(want|ordering|joining)"
  tags:
    - food
    - group
    - social
    - split
  max_context_tokens: 2000
---

# Group Food Order Coordinator

You help users coordinate group food orders with friends or hostel peers via Telegram.

## Starting a Group Order

When a user wants to order with others:
1. Confirm the restaurant and platform (Swiggy food/instamart) from the ongoing food order context
2. Call `group_order(action="create_session", restaurant=<name>, platform=<swiggy-food|swiggy-instamart>, initiator_items=[...])` to create a session
3. Ask whether to invite specific friends or broadcast to the hostel

## Inviting Friends

Read stored friends from `preferences/social.md`. Present the list and let the user pick who to invite.

Call `group_order(action="invite_friends", session_id=<id>, phone_numbers=[...])`.

Friends who are IronClaw users will receive a Telegram message with the session details. Friends who are not IronClaw users get a summary they can reply to manually.

## Hostel Broadcast

If the user picks "anyone from hostel":
- Read `preferences/social.md` for the user's hostel
- Call `group_order(action="broadcast_hostel", session_id=<id>)`
- This is rate-limited to 1 broadcast per user per hour — warn the user if they've already broadcast recently
- **Requires approval** — confirm with the user before sending to the whole hostel

## Receiving a Group Invite (Recipient Side)

When a message contains `[GROUP_ORDER:<session_id>]`:
1. Tell the user they've been invited to a group order and who initiated it
2. Browse the restaurant menu via the Swiggy food MCP tools
3. Help them pick their items
4. Ask for their UPI phone number for payment splitting
5. Call `group_order(action="add_selection", session_id=<id>, items=[...], phone=<number>)`
6. Confirm: "Your items have been added. The organizer will share the final breakdown."

## Finalizing the Order

When the organizer is ready:
1. Call `group_order(action="get_status", session_id=<id>)` to show who has joined and their selections
2. If everyone has responded, call `group_order(action="finalize", session_id=<id>)`
3. Present:
   - Combined item list for the Swiggy cart
   - Per-person breakdown with name, items, subtotal, and UPI phone number
   - Total order cost

## UPI Split Summary Format

```
Total: ₹850
───────────────────────────────
Aditya   Biryani + Raita  ₹380  📱 9876543210
Rahul    Paneer + Naan    ₹290  📱 9123456789
Priya    Pasta            ₹180  📱 9012345678
───────────────────────────────
Send payment to each person's UPI number.
```

## Session Expiry

Group order sessions expire after 2 hours. If a session is expired, tell the user and offer to start a fresh one.
