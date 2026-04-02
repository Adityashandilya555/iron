# Swiggy MCP Server — Issues & Status

**Last Updated**: 2026-04-02  
**Session**: Full debugging session — parameter coercion fixed and deployed. Auth flow reaches Swiggy API successfully. OTP delivery issue now under investigation.

---

## Active Issues

---

### Issue 1 (P0): OTP Not Delivered Despite API Success

**Status**: OPEN — Under Investigation

**Symptom:**
Swiggy `/auth/send-otp` returns HTTP 200 with `{"success":true, "data":{"userId":"...", "sessionInfo":"..."}}` but the test user (9625000934) does not receive an SMS OTP. User confirmed they have an active Swiggy account with that number.

**Evidence from logs:**
```
DEBUG swiggy_auth: execute() params raw_phone=Some(String("9625000934")) raw_cc=None
DEBUG Tool call succeeded tool=swiggy_auth elapsed_ms=328 result_size_bytes=72
```

**What we know:**
- The API call succeeds (328ms, 200 OK)
- `otp_resp.success == true` and `data` is present (otherwise tool would have returned an error)
- The pending state was stored (session_info saved in memory)
- Same API tested with curl for a different number got OTP successfully

**Possible causes:**
1. Swiggy SMS gateway silently drops OTPs for numbers on TRAI DND registry
2. Carrier-level filtering of transactional SMS
3. Swiggy rate-limiting per-number (OTP already sent recently, throttled)
4. Number not verified on Swiggy's backend despite having an account
5. SMS delayed — may arrive after several minutes

**Next steps:**
1. Try resending OTP to the same number after 5+ minutes
2. Test with a different number that is known to receive Swiggy OTPs
3. Test the curl command directly on the server to rule out any proxy/header difference:
   ```bash
   curl -X POST https://mcp.swiggy.com/auth/send-otp \
     -H "Content-Type: application/json" \
     -d '{"phone":"9625000934","countryCode":"+91","codeChallenge":"test123","redirectUri":"http://localhost/callback"}'
   ```
4. Check the actual Swiggy response `message` field by adding response body logging

---

### Issue 2 (P1): complete_auth Not Yet Tested End-to-End

**Status**: BLOCKED by Issue 1

**What needs testing:**
1. `POST /auth/verify-otp` with `{userId, sessionInfo, otp, codeChallenge, redirectUri}` — should return `authorization_code`
2. `POST /auth/token` with `{grant_type, code, code_verifier, client_id, redirect_uri}` — should return `access_token`
3. Token stored under `mcp_swiggy-food_access_token` in SecretsStore
4. Token injected as `Authorization: Bearer <token>` in MCP requests to `mcp.swiggy.com/food`

**Current code path** (`src/tools/builtin/swiggy_auth.rs` `complete_auth()`):
- Retrieves `PendingAuth` from in-memory map (keyed by `user_id`, 10min TTL)
- Calls verify-otp, then token endpoint
- Stores tokens for all 3 servers: `mcp_swiggy-food_access_token`, `mcp_swiggy-instamart_access_token`, `mcp_swiggy-dineout_access_token`

---

### Issue 3 (P1): MCP Tool Calls Not Tested After Auth

**Status**: BLOCKED by Issue 1 and 2

After `complete_auth` succeeds:
- MCP client needs to use the stored Bearer token for requests to `https://mcp.swiggy.com/food`
- Token injection is via `McpServerConfig::token_secret_name()` → `mcp_swiggy-food_access_token`
- The MCP servers (`registry/mcp-servers/swiggy-*.json`) use `"auth": "none"` — token injection happens via the standard SecretsStore lookup path in `mcp/client.rs`

This needs full end-to-end verification once OTP issue is resolved.

---

### Issue 4 (P2): Ephemeral Pending Auth State

**Status**: Known limitation, acceptable for MVP

**Symptom**: If the IronClaw server restarts between `start_auth` and `complete_auth`, the pending auth state (PendingAuth struct containing userId, sessionInfo, PKCE verifier) is lost. The user would need to start the auth flow again.

**Root cause**: `SwiggyAuthTool.pending` is `Arc<Mutex<HashMap<String, PendingAuth>>>` — in-memory only.

**Fix (future)**: Persist PendingAuth to DB with TTL matching `OTP_SESSION_TIMEOUT_SECS` (600s).

---

### Issue 5 (P2): Swiggy MCP Servers Need Installation After Auth

**Status**: Needs skill/flow update

**Symptom**: After `complete_auth` succeeds, the Swiggy MCP servers (`swiggy-food`, `swiggy-instamart`, `swiggy-dineout`) may not be installed/activated in the session. The agent needs to call `tool_install` for each server.

**Fix**: Update the food-ordering skill to include MCP server installation step after auth, or auto-install in `complete_auth` success path.

---

## Resolved Issues

---

### RESOLVED: Parameter Type Coercion (phone/OTP as JSON number)

**Fixed in**: `cfb2be0c` (2026-04-02)

**Was**: LLM sent `{"phone": 9289289123}` (JSON number). `require_str()` returned None for non-string values, producing "missing 'phone' parameter" error at elapsed_ms=0. Prior fix attempts (`ac0bfbcd`, `c925475a`) were correct code but Docker layer caching served the old binary.

**Fix**:
- Added `require_str_coerced()` to `src/tools/tool.rs` — handles String, Number, Bool
- Replaced manual coercion blocks in `swiggy_auth.rs` with `require_str_coerced()`
- Added `sensitive_params() -> &["phone", "otp"]` for log redaction
- Added diagnostic debug logging with raw param types
- Added 2 regression tests (5/5 pass, 26/26 coercion tests pass, zero clippy warnings)
- Deployed with `docker build --no-cache` to bypass Docker layer caching

**Verified**: Logs now show `raw_phone=Some(String("9625000934"))` — phone correctly handled.

---

### RESOLVED: Auth Method Mismatch (DCR vs phone+OTP)

**Fixed in**: `39abae16`

**Was**: `registry/mcp-servers/swiggy-*.json` had `"auth": "dcr"`. System would attempt OAuth DCR against Swiggy, which returns 404 for `/auth/register`. Swiggy uses phone+OTP, not DCR.

**Fix**: Changed all 3 registry manifests to `"auth": "none"`. `SwiggyAuthTool` handles auth independently and stores tokens directly in SecretsStore under the keys the MCP client looks up.

---

### RESOLVED: API Flow Verification (consent step not needed)

**Verified**: 2026-04-02 via curl test

`POST https://mcp.swiggy.com/auth/send-otp` works directly without any prior consent page interaction. The consent page at `/auth/authorize` is browser-only (React SPA). The API endpoints (`/auth/send-otp`, `/auth/verify-otp`, `/auth/token`) are all accessible server-side.

---

## Summary Table

| # | Priority | Issue | Status |
|---|----------|-------|--------|
| 1 | P0 | OTP not delivered despite API success | OPEN — investigating |
| 2 | P1 | complete_auth not tested end-to-end | BLOCKED by #1 |
| 3 | P1 | MCP tool calls not tested after auth | BLOCKED by #1 |
| 4 | P2 | Ephemeral pending state lost on restart | Known limitation |
| 5 | P2 | MCP servers need installation after auth | Skill update needed |
| — | ✅ | phone/OTP JSON number coercion | RESOLVED cfb2be0c |
| — | ✅ | Auth method DCR mismatch | RESOLVED 39abae16 |
| — | ✅ | Consent step required? | RESOLVED (not needed) |
