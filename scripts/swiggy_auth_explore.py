#!/usr/bin/env python3
"""
Swiggy Auth + MCP Explorer
--------------------------
Full flow: send OTP → verify → get token → explore all tools.

Usage:
    python3 scripts/swiggy_auth_explore.py
"""

import json, sys, os, urllib.request, urllib.error, base64, hashlib, secrets

BASE = "https://mcp.swiggy.com"
CLIENT_ID = "swiggy-mcp"
REDIRECT_URI = "http://localhost/callback"
ENDPOINTS = {"food": f"{BASE}/food", "instamart": f"{BASE}/im", "dineout": f"{BASE}/dineout"}

# ── PKCE ──────────────────────────────────────────────────────────────────────

def gen_pkce():
    verifier = base64.urlsafe_b64encode(secrets.token_bytes(32)).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(
        hashlib.sha256(verifier.encode()).digest()
    ).rstrip(b"=").decode()
    return verifier, challenge

# ── HTTP ──────────────────────────────────────────────────────────────────────

def post_json(url, payload, extra_headers=None):
    data = json.dumps(payload).encode()
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if extra_headers:
        headers.update(extra_headers)
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            resp_headers = dict(r.headers)
            body = r.read().decode()
            if body.startswith("data:"):
                merged = {}
                for line in body.splitlines():
                    if line.startswith("data:") and line.strip() != "data:":
                        try: merged.update(json.loads(line[5:].strip()))
                        except: pass
                return merged, resp_headers
            return json.loads(body), resp_headers
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        raise RuntimeError(f"HTTP {e.code}: {body[:500]}")

# ── Auth flow ─────────────────────────────────────────────────────────────────

def auth_flow(phone):
    verifier, challenge = gen_pkce()
    print(f"\n→ Sending OTP to +91{phone} ...")
    body, _ = post_json(f"{BASE}/auth/send-otp", {
        "phone": phone, "countryCode": "+91",
        "codeChallenge": challenge, "redirectUri": REDIRECT_URI
    })
    if not body.get("success"):
        raise RuntimeError(f"send-otp failed: {body}")
    data = body["data"]
    user_id = data["userId"]
    session_info = data["sessionInfo"]
    print(f"✓ OTP sent  (userId={user_id[:12]}...)")

    otp = input("Enter the OTP you received: ").strip()

    print("→ Verifying OTP ...")
    body, _ = post_json(f"{BASE}/auth/verify-otp", {
        "userId": user_id, "sessionInfo": session_info,
        "otp": otp, "codeChallenge": challenge, "redirectUri": REDIRECT_URI
    })
    if not body.get("success"):
        raise RuntimeError(f"verify-otp failed: {body}")
    auth_code = body["data"]["authorization_code"]
    print("✓ OTP verified")

    print("→ Exchanging code for token ...")
    body, _ = post_json(f"{BASE}/auth/token", {
        "grant_type": "authorization_code",
        "code": auth_code, "code_verifier": verifier,
        "client_id": CLIENT_ID, "redirect_uri": REDIRECT_URI
    })
    token = body.get("access_token") or body.get("opaque_code")
    if not token:
        raise RuntimeError(f"No token in response: {body}")
    print(f"✓ Token obtained  ({token[:16]}...{token[-6:]})")
    return token

# ── MCP Explorer ──────────────────────────────────────────────────────────────

def explore(name, url, token):
    print(f"\n{'='*62}")
    print(f"  {name.upper()}  →  {url}")
    print(f"{'='*62}")
    hdrs = {"Authorization": f"Bearer {token}"}

    try:
        resp, rh = post_json(url, {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "ironclaw-explorer", "version": "0.1"}
            }
        }, hdrs)
    except RuntimeError as e:
        print(f"  ❌ initialize failed: {e}")
        return []

    sid = rh.get("Mcp-Session-Id") or rh.get("mcp-session-id")
    if sid:
        hdrs["Mcp-Session-Id"] = sid

    if "error" in resp:
        print(f"  ❌ {resp['error']}")
        return []

    r = resp.get("result", {})
    si = r.get("serverInfo", {})
    print(f"  Server : {si.get('name','?')} v{si.get('version','?')}")
    print(f"  Caps   : {list(r.get('capabilities', {}).keys())}")

    # initialized notification (fire-and-forget, ignore errors)
    try:
        post_json(url, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}, hdrs)
    except: pass

    try:
        resp, _ = post_json(url, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}, hdrs)
    except RuntimeError as e:
        print(f"  ❌ tools/list failed: {e}")
        return []

    if "error" in resp:
        print(f"  ❌ tools/list error: {resp['error']}")
        return []

    tools = resp.get("result", {}).get("tools", [])
    print(f"\n  {len(tools)} tool(s) found:\n")

    names = []
    for t in tools:
        n = t.get("name", "?")
        d = t.get("description", "")
        schema = t.get("inputSchema", {})
        props = schema.get("properties", {})
        req = schema.get("required", [])
        names.append(n)

        print(f"  ┌─ {n}")
        if d:
            for i, chunk in enumerate([d[j:j+72] for j in range(0, len(d), 72)]):
                print(f"  │  {chunk}")
        if props:
            print(f"  │  Params:")
            for pn, pd in props.items():
                pt = pd.get("type") or "?"
                if isinstance(pt, list): pt = "|".join(pt)
                pdesc = pd.get("description", "")[:80]
                r_mark = " ✱" if pn in req else ""
                print(f"  │    {pn} ({pt}){r_mark}  {pdesc}")
        else:
            print(f"  │  Params: (none / see description)")
        print(f"  └─")
    return names

# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    phone = "9289289123"
    print(f"Phone: +91{phone}")

    try:
        token = auth_flow(phone)
    except RuntimeError as e:
        print(f"\n❌ Auth failed: {e}")
        sys.exit(1)

    print(f"\n{'─'*62}")
    print("  Exploring Swiggy MCP tools ...")
    print(f"{'─'*62}")

    summary = {}
    for name, url in ENDPOINTS.items():
        summary[name] = explore(name, url, token)

    print(f"\n{'='*62}")
    print("  TOOL SUMMARY")
    print(f"{'='*62}")
    for name, tools in summary.items():
        print(f"  {name:12s}: {tools}")
    print()

if __name__ == "__main__":
    main()
