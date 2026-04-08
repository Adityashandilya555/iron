#!/usr/bin/env python3
"""
Swiggy MCP Explorer
-------------------
Authenticates against all 3 Swiggy MCP endpoints and dumps every tool's
full schema so we can find gaps in the IronClaw swiggy tool implementation.

Usage:
    python3 scripts/swiggy_mcp_explore.py <bearer_token>

Or set env var:
    SWIGGY_TOKEN=<token> python3 scripts/swiggy_mcp_explore.py
"""

import json
import sys
import os
import urllib.request
import urllib.error

ENDPOINTS = {
    "food":      "https://mcp.swiggy.com/food",
    "instamart": "https://mcp.swiggy.com/im",
    "dineout":   "https://mcp.swiggy.com/dineout",
}

def post(url: str, payload: dict, headers: dict) -> tuple[dict, dict]:
    """Send a JSON-RPC request, return (response_body, response_headers)."""
    data = json.dumps(payload).encode()
    req = urllib.request.Request(url, data=data, headers={
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        **headers,
    }, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            resp_headers = dict(resp.headers)
            body = resp.read().decode()
            # Handle SSE or plain JSON
            if body.startswith("data:"):
                lines = [l[5:].strip() for l in body.splitlines() if l.startswith("data:") and l.strip() != "data:"]
                merged = {}
                for line in lines:
                    try:
                        merged.update(json.loads(line))
                    except Exception:
                        pass
                return merged, resp_headers
            return json.loads(body), resp_headers
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        print(f"  HTTP {e.code}: {body[:300]}")
        return {}, {}

def explore_endpoint(name: str, url: str, token: str):
    print(f"\n{'='*60}")
    print(f"  ENDPOINT: {name}  ({url})")
    print(f"{'='*60}")

    auth_headers = {"Authorization": f"Bearer {token}"}

    # Step 1: initialize
    init_req = {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "ironclaw-explorer", "version": "0.1.0"},
        }
    }
    resp, resp_headers = post(url, init_req, auth_headers)
    if not resp:
        print("  ❌ initialize failed — server unreachable or rejected token")
        return []

    session_id = resp_headers.get("Mcp-Session-Id") or resp_headers.get("mcp-session-id")
    if session_id:
        auth_headers["Mcp-Session-Id"] = session_id
        print(f"  Session-Id: {session_id}")

    result = resp.get("result", {})
    server_info = result.get("serverInfo", {})
    print(f"  Server: {server_info.get('name','?')} v{server_info.get('version','?')}")
    print(f"  Protocol: {result.get('protocolVersion','?')}")
    server_caps = result.get("capabilities", {})
    print(f"  Capabilities: {list(server_caps.keys())}")

    if "error" in resp:
        print(f"  ❌ initialize error: {resp['error']}")
        return []

    # Step 2: initialized notification
    notif = {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}
    post(url, notif, auth_headers)

    # Step 3: list tools
    list_req = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
    resp, _ = post(url, list_req, auth_headers)

    if "error" in resp:
        print(f"  ❌ tools/list error: {resp['error']}")
        return []

    tools = resp.get("result", {}).get("tools", [])
    print(f"\n  Found {len(tools)} tools:\n")

    tool_names = []
    for tool in tools:
        tname = tool.get("name", "?")
        tdesc = tool.get("description", "")
        schema = tool.get("inputSchema", {})
        props = schema.get("properties", {})
        required = schema.get("required", [])
        tool_names.append(tname)

        print(f"  ┌─ {tname}")
        if tdesc:
            # Wrap long descriptions
            words = tdesc.split()
            line, lines = [], []
            for w in words:
                line.append(w)
                if len(" ".join(line)) > 70:
                    lines.append(" ".join(line[:-1]))
                    line = [w]
            if line:
                lines.append(" ".join(line))
            for i, l in enumerate(lines):
                prefix = "  │  " if i == 0 else "  │    "
                print(f"{prefix}{l}")

        if props:
            print(f"  │  Parameters:")
            for pname, pdef in props.items():
                ptype = pdef.get("type", pdef.get("anyOf", [{}])[0].get("type", "?"))
                pdesc = pdef.get("description", "")
                req_marker = " ✱" if pname in required else ""
                print(f"  │    {pname} ({ptype}){req_marker}: {pdesc[:80]}")
        else:
            print(f"  │  Parameters: (none)")
        print(f"  └─")

    return tool_names

def main():
    token = None
    if len(sys.argv) > 1:
        token = sys.argv[1]
    else:
        token = os.environ.get("SWIGGY_TOKEN")

    if not token:
        try:
            token = input("Paste your Swiggy Bearer token: ").strip()
        except (EOFError, KeyboardInterrupt):
            print("\nUsage: python3 scripts/swiggy_mcp_explore.py <bearer_token>")
            sys.exit(1)
    if not token:
        print("No token provided.")
        sys.exit(1)

    print(f"Token: {token[:12]}...{token[-6:] if len(token) > 18 else ''}  (length {len(token)})")

    all_tools = {}
    for name, url in ENDPOINTS.items():
        tools = explore_endpoint(name, url, token)
        all_tools[name] = tools

    print(f"\n{'='*60}")
    print("  SUMMARY")
    print(f"{'='*60}")
    for name, tools in all_tools.items():
        print(f"  {name}: {tools}")

    print("\nDone. Paste this output so we can find gaps in the IronClaw swiggy tool.")

if __name__ == "__main__":
    main()
