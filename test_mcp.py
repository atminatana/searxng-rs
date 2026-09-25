#!/usr/bin/env python3
"""MCP health check - stdlib only."""
import json, sys, urllib.request, urllib.error

BASE_URL = "http://127.0.0.1:3001/mcp"
TIMEOUT = 10
session_id = None
request_id = 0


def post(payload):
    global session_id
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
    }
    if session_id:
        headers["Mcp-Session-Id"] = session_id

    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(BASE_URL, data=data, headers=headers, method="POST")

    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            body = resp.read().decode("utf-8")
            h = dict(resp.headers.items())
            if "mcp-session-id" in h:
                session_id = h["mcp-session-id"]
            return h, body
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        return dict(e.headers.items()), body
    except urllib.error.URLError as e:
        raise ConnectionError(f"Cannot connect: {e}")


def call(method, params=None):
    global request_id
    request_id += 1
    payload = {"jsonrpc": "2.0", "method": method, "id": request_id}
    if params is not None:
        payload["params"] = params

    print(f"\n>>> {method} (id={request_id})")
    try:
        headers, body = post(payload)
    except ConnectionError as e:
        print(f"FAIL: {e}")
        return None

    print(f"Session: {session_id}")
    print(f"HTTP headers: {headers}")
    print(f"Body: {body[:1500]}")
    print(f"Body (repr): {body[:1500]!r}")

    if not body:
        print("FAIL: empty response")
        return None

    # Parse SSE or plain JSON
    if body.startswith("data:"):
        data_lines = [line[5:].strip() for line in body.splitlines()
                      if line.startswith("data:") and line[5:].strip()]
        print(f"  [debug] SSE data lines (non-empty): {data_lines!r}")
        if not data_lines:
            print(f"FAIL: no data: lines: {body[:200]!r}")
            return None
        parsed = None
        for i, dl in enumerate(data_lines):
            try:
                cand = json.loads(dl)
            except json.JSONDecodeError as e:
                print(f"  [debug] data[{i}] not JSON ({e}): {dl[:120]!r}")
                continue
            if cand.get("id") == request_id:
                parsed = cand
                print(f"  [debug] matched data[{i}] (id={request_id})")
                break
            if parsed is None:
                parsed = cand
        if parsed is None:
            print(f"FAIL: cannot parse any data line: {body[:300]!r}")
            return None
    else:
        try:
            parsed = json.loads(body)
        except json.JSONDecodeError:
            print(f"FAIL: not JSON: {body[:200]!r}")
            return None

    if "error" in parsed:
        print(f"FAIL: JSON-RPC error: {parsed['error']}")
        return None
    if parsed.get("id") != request_id:
        print(f"FAIL: id mismatch: expected {request_id}, got {parsed.get('id')}")
        return None

    print(f"OK: {method}")
    return parsed


def main():
    # 1. Initialize
    init = call("initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "mcp-healthcheck", "version": "1.0"},
    })
    if init is None:
        return 1

    # 2. Initialized notification (no response expected)
    print("\n>>> notifications/initialized")
    try:
        h, body = post({"jsonrpc": "2.0", "method": "notifications/initialized", "id": None})
        print(f"HTTP {h.get('Status', 'OK')}, body: {body[:200]}")
    except ConnectionError as e:
        print(f"FAIL: {e}")
        return 1

    # 3. Tools list
    tools = call("tools/list")
    if tools is None:
        return 1

    tool_names = [t.get("name") for t in tools.get("result", {}).get("tools", [])]
    print(f"\nAvailable tools: {tool_names}")

    if "engine_status" not in tool_names:
        print("WARN: engine_status not found in tool list!")
    else:
        # 4. engine_status call
        status = call("tools/call", {"name": "engine_status", "arguments": {"engine": "bing"}})
        if status is None:
            return 1
        content = status.get("result", {}).get("content", [])
        print(f"\nengine_status result: {content}")

    # 5. Search
    search = call("tools/call", {
        "name": "search",
        "arguments": {
            "query": "test query",
            "engines": "bing",
            "safesearch": 0,
            "language": "ru",
            "pageno": 1,
        },
    })
    if search is not None:
        content = search.get("result", {}).get("content", [])
        print(f"\nsearch result: {content}")

    print("\n=== Check complete ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
