---
name: onboarding
version: 0.1.0
description: >
  Handles first-time user onboarding for Personifi/Aria. Use this skill when a new
  user sends /start, when USER.md doesn't exist in their workspace, or when onboarding
  is incomplete (missing fields). Triggers on: /start command, first message from
  unknown user, "set up my profile", "change my preferences", missing user preferences.
  Guides through 3-6 questions to capture name, residence, diet, cuisines, budget, and
  friends. Stores all data in workspace USER.md.
activation:
  patterns:
    - "/start"
    - "set up my profile"
    - "change my preferences"
    - "update my preferences"
  keywords:
    - "onboarding"
    - "setup"
    - "profile"
  tags:
    - "onboarding"
    - "setup"
  max_context_tokens: 3000
---

# Onboarding — New User Setup

## Detection

Read USER.md first (`memory_read target:"USER.md"`), then determine resume point:

| USER.md state | Action |
|---------------|--------|
| File missing OR name empty | Greet already sent — acknowledge name, move to Q2 |
| name filled, phone empty | Resume at Q2 |
| phone filled, residence empty | Resume at Q3 |
| residence filled, diet empty | Resume at Q4 |
| diet filled, cuisines empty/`[]` | Resume at Q5 |
| cuisines filled, budget_max empty/0 | Resume at Q6 |
| budget_max > 0, onboarding_completed empty | Ask Q7 (optional) |
| onboarding_completed has a date | Skip onboarding — show home screen |

This works across session restarts because USER.md persists in the tenant-isolated workspace.

## Greeting (Phase 1)

First message to new user:

```
Yo! 👋 I'm Aria — your food & hangout buddy for Bangalore.

I can order food (Swiggy + Zomato), grab groceries (Instamart),
and book tables at restaurants. Let me get to know you real quick
so I can make better picks.

What's your name?
```

**Tone:** Casual, warm, Bangalore college student energy. No corporate language.

## Questions (Phase 2)

Ask ONE question at a time. Wait for response before next.

### Q1: Name
```
Prompt: "What's your name?"
Store:  USER.md → name: "{response}"
```

### Q2: Phone Number
```
Prompt: "What's your phone number? I'll use this to connect Swiggy and Zomato — you won't have to enter it again."
Store:  USER.md → phone: "{response}"
Validate: 10-digit Indian mobile number (starts with 6-9)
```
This is the PERMANENT auth credential. Never ask again. Used for all platform OTP flows.

### Q3: Residence
```
Prompt: "Nice, {name}! Where do you stay — hostel, PG, or apartment? And which one?"
Store:  USER.md → residence: "{response}"
        USER.md → residence_type: "hostel" | "pg" | "apartment" | "home"
```
Parse the response to extract both type and specific name (e.g., "Hostel Block A, MIT campus").

### Q4: Diet
```
Prompt: "Veg, non-veg, or eggetarian?"
Store:  USER.md → diet: "veg" | "non-veg" | "eggetarian"
```
Accept variations: "non veg", "nonveg", "I eat everything", "pure veg", "egg is fine".

### Q5: Favourite Cuisines
```
Prompt: "What's your go-to food? Pick a few: South Indian, North Indian, Chinese, Italian, Biryani, Burgers, Street Food, Healthy, Desserts... or tell me your own!"
Store:  USER.md → cuisines: ["south indian", "biryani", "chinese"]
```
Accept free-form. Normalize to lowercase. Support multiple.

### Q6: Budget
```
Prompt: "What's your usual order budget? Like ₹150, ₹300, ₹500+?"
Store:  USER.md → budget_min: 0
        USER.md → budget_max: 300
```
Parse ranges ("200-400"), single values ("around 300"), or labels ("cheap", "moderate", "fancy").

### Q7: Friends (Optional)
```
Prompt: "Last one — want to add any friends? Share their Telegram usernames and I can help you order together later. Or say 'skip' for now."
Store:  USER.md → friends: ["@rahul_k", "@priya_m"]
```
If skip: `friends: []`. This can always be updated later.

## USER.md Schema

```markdown
# User Profile

## Identity
- name: Adi
- telegram_id: 123456789
- phone: 9289289123

## Location
- city: Bangalore
- residence: HB-1, BSF Campus, Govindapura
- residence_type: hostel

## Preferences
- diet: non-veg
- cuisines: [biryani, south indian, chinese, street food]
- budget_min: 150
- budget_max: 400

## Social
- friends: [@rahul_k, @priya_m]
- squads: []

## History
- onboarding_completed: 2026-04-06
- last_order_platform: swiggy
- last_order_restaurant: Paradise Biryani
- order_count: 0
```

## State Tracking

State is derived from USER.md field completeness — no session metadata needed.
After each answer, write the field to USER.md immediately via `memory_write`.
On the next session, read USER.md and resume from the first missing field.

If user sends a non-answer mid-onboarding (e.g., "what can you do?"), answer their
question, then resume: "Anyway — back to setup! {next_question}"

If user says "skip" at any point, mark remaining fields as empty strings and set
`onboarding_completed` to today's date, then show the home screen.

## Post-Onboarding (Phase 3 → Home Screen)

After Q7 (or if user skips):

```
All set, {name}! 🎉 Here's what I can do:

🍔 Order Food — Swiggy + Zomato, best deals
🛒 Groceries — Instamart, 10-min delivery
🍽 Book Table — Dineout deals, instant booking
💬 Chat — Ask me anything about food in Bangalore

What are you in the mood for?
```

Show Telegram inline keyboard (2 rows, with callback data):
Row 1: [🍔 Order Food](order_food) [🛒 Groceries](groceries)
Row 2: [🍽 Book Table](book_table) [💬 Chat](chat_mode)

## Preference Updates

If user later says "I'm veg now" or "update my preferences":
1. Read current USER.md
2. Ask which field to change
3. Update and confirm

Don't re-run full onboarding for preference updates — just modify the specific field.
