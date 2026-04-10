# Telegram WebApp for Social Friends Management — MODIFIED

**Status:** REVISED (supersedes 04-telegram-webapp.md)
**Changes from original:**
1. Added friend search UX specification (how users find each other)
2. Added 1st vs 2nd degree visual distinction
3. Added WebApp ↔ social DB integration detail (references db/03-social-graph.md schema)
4. Noted that Telegram WASM channel does NOT yet handle callback_query or web_app_data (agent 3 finding)

---

## What the Original Task Got Right

- WASM Telegram channel needs `web_app_data` handler (still true)
- WebApp needs `initData` HMAC-SHA256 validation (still true)
- Frontend served as static files via web gateway (still true)
- `user_identities` table is a prerequisite (still true)

## What the Original Task Missed

### 1. Telegram WASM Channel Has No Callback Query Support

Agent exploration found (`channels-src/telegram/src/lib.rs`): The Telegram channel handles text messages, media, group chats, and forum topics. **Callback queries and inline keyboards are NOT implemented.** This means:
- The home screen buttons (Order Food, Groceries, Book Table, Chat) don't work yet
- `maybe_track_aria_phase()` (agent_loop.rs:992) checks for `metadata.type == "callback_query"` but the WASM channel never produces this metadata
- WebApp buttons (`web_app` type in inline keyboard) also won't work

**This is a prerequisite blocker.** Before WebApp can work, the WASM Telegram channel must handle:
1. `callback_query` updates (for inline keyboard buttons)
2. `web_app_data` updates (for WebApp submissions)
3. Sending messages with `reply_markup` (inline keyboard JSON)

**Files:** `channels-src/telegram/src/lib.rs` — this is the WASM source, not Rust. Must be modified and recompiled.

### 2. Friend Search UX

The original task said "A small HTML/JS app for adding/removing friends" but didn't specify HOW users find each other.

**Search mechanisms (prioritized):**

| Method | Implementation | Privacy | MVP? |
|---|---|---|---|
| **Deep link invite** | `t.me/AriaBot?start=friend_{user_id}` — user shares link, friend clicks, auto-creates pending friendship | High (opt-in) | YES |
| **Name search** | WebApp queries `user_identities` by `display_name ILIKE '%query%'` | Medium (names visible) | YES |
| **Hostel browse** | WebApp queries `user_preferences WHERE residence = '{hostel}'` — shows anonymized list of Aria users in same hostel | Medium (residence visible to hostel mates) | YES |
| **Phone match** | Hash phone from SecretsStore, compare hashes | High (hash-only) | NO (complex) |
| **Telegram contacts** | Use Telegram `getContacts` API | N/A (not available to bots) | NO |

**MVP approach:** Deep link + name search + hostel browse. User opens WebApp, sees:
1. **My Friends** tab — list of accepted friends, pending requests
2. **Find Friends** tab — search by name, browse hostel mates
3. **Invite** button — generates deep link to share

### 3. 1st vs 2nd Degree Visual Distinction

**1st degree (direct friends):**
- Stored in `friendships` table with `status = 'accepted'`
- Shown with full display name + Telegram username
- Can be invited to group orders
- Their order activity contributes to restaurant ranking ("Rahul ordered here")

**2nd degree (hostel mates / friends-of-friends):**
- Hostel mates: users with same `residence` in `user_preferences`
- Friends-of-friends: reachable via 2-hop friendship path
- Shown anonymized: "12 Aria users in HB-1" (not individual names)
- Their activity contributes to ranking only as aggregate: "Popular in your hostel"
- Cannot be invited to group orders directly (must become 1st degree first)

**WebApp display:**

```
┌─────────────────────────────────┐
│  My Friends (3)                 │
│  ┌───────────────────────────┐  │
│  │ 👤 Rahul K.    [Remove]  │  │
│  │ 👤 Priya M.    [Remove]  │  │
│  │ 👤 Arjun S.    [Remove]  │  │
│  └───────────────────────────┘  │
│                                 │
│  Pending (1)                    │
│  ┌───────────────────────────┐  │
│  │ 👤 Sneha R.  [Accept][X] │  │
│  └───────────────────────────┘  │
│                                 │
│  Your Hostel: HB-1 (18 users)  │
│  ┌───────────────────────────┐  │
│  │ Trending: Paradise Biryani│  │
│  │ 5 people ordered today    │  │
│  │           [Add as Friend] │  │
│  └───────────────────────────┘  │
│                                 │
│  [🔗 Invite Link]  [🔍 Search] │
└─────────────────────────────────┘
```

### 4. WebApp ↔ Social DB Integration

**Backend API endpoints (served by web gateway):**

| Endpoint | Method | Auth | Purpose |
|---|---|---|---|
| `/webapp/auth` | POST | `initData` HMAC | Validate Telegram identity, return session token |
| `/webapp/friends` | GET | Session token | List 1st degree friends + pending requests |
| `/webapp/friends/request` | POST | Session token | Send friend request (body: `{addressee_id}`) |
| `/webapp/friends/accept` | POST | Session token | Accept pending request (body: `{friendship_id}`) |
| `/webapp/friends/remove` | DELETE | Session token | Remove friendship (body: `{friendship_id}`) |
| `/webapp/friends/search` | GET | Session token | Search by name (`?q=rahul`) |
| `/webapp/hostel` | GET | Session token | Hostel aggregate: user count, trending restaurants |
| `/webapp/invite-link` | GET | Session token | Generate deep link for sharing |

**All endpoints query the `friendships` table from `tasks/db/03-social-graph.md`.** The `user_identities` + `user_external_identities` tables from `tasks/db/02-user-identity.md` are required for mapping Telegram ID → internal ID.

**SOUL.md compliance:**
- `/webapp/hostel` returns ONLY aggregates (count, trending restaurant name), never individual user data
- `/webapp/friends/search` returns only users who have opted into being searchable (add a `searchable` boolean to `user_preferences` or `user_identities`)
- Friend request requires explicit acceptance (bidirectional)

## Required Changes (Updated from Original)

### Prerequisite: WASM Telegram Channel Updates

| What | File | Change |
|---|---|---|
| Handle `callback_query` | `channels-src/telegram/src/lib.rs` | Parse `callback_query` from webhook/poll updates, create `IncomingMessage` with `metadata.type = "callback_query"` |
| Handle `web_app_data` | `channels-src/telegram/src/lib.rs` | Parse `web_app_data` from message updates, create `IncomingMessage` with `metadata.type = "web_app_data"` |
| Send inline keyboards | `channels-src/telegram/src/lib.rs` | Support `reply_markup` JSON in `send_message()` for inline keyboard buttons |
| Send WebApp buttons | `channels-src/telegram/src/lib.rs` | Support `web_app` button type in inline keyboard |
| Recompile WASM | Build pipeline | Rebuild Telegram channel WASM module |

### Backend

| What | File | Change |
|---|---|---|
| WebApp auth endpoint | `src/channels/web/handlers/webapp_auth.rs` | NEW: validate `initData` HMAC, issue session token |
| Friends API endpoints | `src/channels/web/handlers/webapp_friends.rs` | NEW: CRUD for friendships, search, hostel aggregates |
| Route registration | `src/channels/web/server.rs` | Add `/webapp/*` routes |
| Static file serving | `src/channels/web/handlers/static_files.rs` | Serve WebApp HTML/JS from `/webapp/friends/` |

### Frontend

| What | Location | Notes |
|---|---|---|
| Friends WebApp | `src/channels/web/static/webapp/friends/index.html` | Single HTML file with embedded JS. Uses `window.Telegram.WebApp` SDK. |
| CSS | Inline or `style.css` | Telegram WebApp theme variables for dark/light mode |

### Database Prerequisites

- `user_identities` + `user_external_identities` (tasks/db/02-user-identity.md)
- `friendships` (tasks/db/03-social-graph.md)
- `user_preferences` with `residence` field (tasks/db/01-user-prefs.md)

## Coordination Notes

- **Callback query handling is the FIRST thing to build.** Without it, home screen buttons don't work, phase tracking doesn't work, and WebApp buttons can't be sent. This is not just a WebApp prerequisite — it's a prerequisite for the entire Aria experience.
- The WASM channel source is in `channels-src/telegram/` — this is separate from the Rust codebase. Changes require rebuilding the WASM module.
- Bot token for `initData` validation: currently in WASM channel credentials. The Rust-side `/webapp/auth` handler needs the same secret. Coordinate via SecretsStore or shared env var.
