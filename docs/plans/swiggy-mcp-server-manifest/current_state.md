# Swiggy MCP Integration — Current State

**Last Updated**: 2026-04-02  
**Session Summary**: Full debugging session that diagnosed, fixed, and deployed the swiggy_auth parameter coercion bug.

---

## What Works Now

### Auth Flow (FIXED & DEPLOYED)
- `swiggy_auth(action="status")` — works, returns connection state
- `swiggy_auth(action="start_auth", phone="...", country_code="...")` — **FIXED**: phone as JSON number (9289289123) now correctly coerced to string
- `swiggy_auth(action="complete_auth", otp="...")` — code path correct, not yet end-to-end tested
- Phone and OTP are redacted in logs via `sensitive_params()`
- Diagnostic logging added: `swiggy_auth: execute() params raw_phone=... raw_cc=...`
- Diagnostic logging added: `swiggy_auth: OTP request accepted by Swiggy swiggy_user_id=... session_info=... phone=... country_code=...`

### API Endpoints (VERIFIED by curl)
- `POST https://mcp.swiggy.com/auth/send-otp` — works directly, no browser consent needed
- `GET https://mcp.swiggy.com/.well-known/oauth-authorization-server` — returns OAuth metadata
- `https://mcp.swiggy.com/auth/authorize` — browser-only consent+phone+OTP page (NOT needed for API flow)
- `/auth/register` (DCR) — returns 404, not supported by Swiggy

### OAuth Metadata (from well-known endpoint)
```json
{
  "issuer": "https://mcp.swiggy.com/auth",
  "authorization_endpoint": "https://mcp.swiggy.com/auth/authorize",
  "token_endpoint": "https://mcp.swiggy.com/auth/token",
  "registration_endpoint": "https://mcp.swiggy.com/auth/register",
  "scopes_supported": ["mcp:tools", "mcp:resources", "mcp:prompts"],
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "code_challenge_methods_supported": ["S256"]
}
```

### Deployed Commits (on `Adityashandilya555/iron` staging branch)
| Commit | Description |
|--------|-------------|
| `3a253dc7` | feat: add phone+OTP MCP auth and food ordering skills |
| `39abae16` | fix: resolve MCP auth and config issues |
| `ac0bfbcd` | fix: accept phone/country_code/otp as string or number |
| `c925475a` | fix: coerce number/bool→string in global coercion layer |
| `cfb2be0c` | fix: add require_str_coerced() + simplify parameter handling (latest) |

### Docker Deployment
- Image built with `docker build --no-cache -t ironclaw:latest .` (full clean build, ~32 min)
- Running via `docker compose -f docker-compose.prod.yml up -d`
- Server: Aria droplet at `/opt/ironclaw`
- Confirmed: `swiggy_auth` tool registered at startup

---

## What Is NOT Working

### OTP Delivery Issue (OPEN)
- **Symptom**: Swiggy API returns `{"success":true, "data":{"userId":"...", "sessionInfo":"..."}}` but test user (9625000934) did NOT receive SMS OTP
- **Confirmed**: User has active Swiggy account with that number
- **Status**: Unknown — could be Swiggy SMS gateway issue, carrier block, or DND
- **Next step**: Test with another known number, or check if OTP arrives after longer delay

### complete_auth / Token Exchange (NOT YET TESTED)
- The `complete_auth(otp=...)` path calls `/auth/verify-otp` → `/auth/token`
- Token stored under `mcp_swiggy-food_access_token`, `mcp_swiggy-instamart_access_token`, `mcp_swiggy-dineout_access_token`
- Not tested end-to-end because OTP hasn't been received yet

### MCP Tool Calls (NOT YET TESTED)
- After auth, the MCP client at `https://mcp.swiggy.com/food` (and `/im`, `/dineout`) needs to accept the Bearer token
- Not tested since auth hasn't completed

---

## Code State

### `src/tools/builtin/swiggy_auth.rs`
**Key constants:**
```rust
const SWIGGY_MCP_BASE: &str = "https://mcp.swiggy.com";
const SWIGGY_CLIENT_ID: &str = "swiggy-mcp";
const SWIGGY_REDIRECT_URI: &str = "http://localhost/callback";
const OTP_SESSION_TIMEOUT_SECS: i64 = 600;  // 10 minutes
const SWIGGY_SERVERS: &[&str] = &["swiggy-food", "swiggy-instamart", "swiggy-dineout"];
```

**Auth flow (server-side, no browser):**
1. `start_auth(phone, country_code)`:
   - Generates PKCE (verifier + challenge)
   - POST `/auth/send-otp` with `{phone, countryCode, codeChallenge, redirectUri}`
   - Stores `PendingAuth{swiggy_user_id, session_info, pkce_verifier, ...}` in memory (keyed by user_id, 10min TTL)
   - Returns "OTP sent to +91XXXXXXXXXX"

2. `complete_auth(otp)`:
   - POST `/auth/verify-otp` with `{userId, sessionInfo, otp, codeChallenge, redirectUri}` → `authorization_code`
   - POST `/auth/token` with `{grant_type, code, code_verifier, client_id, redirect_uri}` → `access_token`
   - Stores token in SecretsStore for all 3 Swiggy servers
   - Clears pending state

3. `status()`:
   - Checks SecretsStore for `mcp_swiggy-food_access_token`

**Parameter handling:**
- `require_str_coerced(params, "phone")` — handles String, Number, Bool
- `require_str_coerced(params, "otp")` — same
- country_code: match arm handles String (with/without +), Number (prepends +), missing (defaults "+91")

### `src/tools/tool.rs`
- Added `require_str_coerced()` function — public, returns `Result<String, ToolError>`

### `registry/mcp-servers/swiggy-*.json`
- All 3 manifests use `"auth": "none"` — tokens handled by SwiggyAuthTool, not DCR

---

## Skill Configuration

Skills live in the workspace `skills/` directory (user-placed, trusted). Food ordering skill instructs the agent to:
1. Call `swiggy_auth(action="status")` first
2. If not connected, ask user for phone number
3. Call `swiggy_auth(action="start_auth", phone="...")`
4. Ask user for OTP
5. Call `swiggy_auth(action="complete_auth", otp="...")`
6. Use Swiggy MCP tools

---

## Remaining Known Issues

See `issues.md` in this directory for full details. Current priority:

| Priority | Issue | Status |
|----------|-------|--------|
| P0 | OTP SMS not delivered despite API success | OPEN — needs investigation |
| P1 | complete_auth / token exchange not tested | BLOCKED by OTP issue |
| P1 | MCP tool calls with Bearer token not tested | BLOCKED by OTP issue |
| P2 | Ephemeral pending state (lost on server restart during OTP window) | Known limitation |
| P2 | MCP servers need to be installed/activated after auth | Skill update needed |

---

## How to Debug

### Enable verbose Swiggy logs
```bash
docker compose -f docker-compose.prod.yml logs -f ironclaw | grep -E "swiggy_auth|swiggy"
```

### Check if OTP request was accepted
Look for:
```
DEBUG swiggy_auth: OTP request accepted by Swiggy swiggy_user_id=<hash> session_info=<uuid> phone=9XXXXXXXXX country_code=+91
```

### Test auth flow manually
```bash
# Send OTP
curl -X POST https://mcp.swiggy.com/auth/send-otp \
  -H "Content-Type: application/json" \
  -d '{"phone":"9XXXXXXXXX","countryCode":"+91","codeChallenge":"test123","redirectUri":"http://localhost/callback"}'

# Verify OTP (after receiving it)
curl -X POST https://mcp.swiggy.com/auth/verify-otp \
  -H "Content-Type: application/json" \
  -d '{"userId":"<userId>","sessionInfo":"<sessionInfo>","otp":"<otp>","codeChallenge":"test123","redirectUri":"http://localhost/callback"}'
```
