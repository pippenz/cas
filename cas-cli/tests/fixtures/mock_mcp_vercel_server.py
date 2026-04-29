#!/usr/bin/env python3
"""Minimal stdio MCP server fixture used by `cas-cli/tests/integrate_lifecycle_test.rs`.

Implements just enough of the MCP protocol (line-delimited JSON-RPC over
stdio) to let `cmcp_core::ProxyEngine::from_configs` perform the
initialize handshake and respond to a couple of `tools/call` requests
with canned Vercel-shaped JSON. No real Vercel API touched. Tracked in
task **cas-2dc9** (item 4).

Protocol notes:
- One JSON object per line on stdin, responses one-per-line on stdout.
- Notifications (no `id`) are silently consumed.
- Unknown methods return a JSON-RPC error for that id; the test only
  exercises `initialize`, `tools/list`, and `tools/call`.

Keep this file small — it is a test fixture, not a reusable library.
"""

import json
import sys


def respond(req_id, result):
    sys.stdout.write(
        json.dumps({"jsonrpc": "2.0", "id": req_id, "result": result}) + "\n"
    )
    sys.stdout.flush()


def respond_error(req_id, code, message):
    sys.stdout.write(
        json.dumps(
            {
                "jsonrpc": "2.0",
                "id": req_id,
                "error": {"code": code, "message": message},
            }
        )
        + "\n"
    )
    sys.stdout.flush()


def text_content(payload):
    """Wrap a Python object as MCP `content[].text` (a JSON string)."""
    return [{"type": "text", "text": json.dumps(payload)}]


CANNED_PROJECTS = [
    {"id": "prj_FIXTURE_FRONT", "name": "fixture-frontend", "accountId": "team_F"},
    {"id": "prj_FIXTURE_BACK", "name": "fixture-backend", "accountId": "team_F"},
]


def handle(req):
    method = req.get("method")
    req_id = req.get("id")
    if req_id is None:
        # Notification (e.g. notifications/initialized) — no response.
        return

    if method == "initialize":
        respond(
            req_id,
            {
                "protocolVersion": req.get("params", {}).get(
                    "protocolVersion", "2024-11-05"
                ),
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "mock-mcp-vercel", "version": "0.0.1"},
            },
        )
        return

    if method == "tools/list":
        respond(
            req_id,
            {
                "tools": [
                    {
                        "name": "list_projects",
                        "description": "Mock vercel.list_projects",
                        "inputSchema": {"type": "object", "properties": {}},
                    },
                    {
                        "name": "get_project",
                        "description": "Mock vercel.get_project",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "projectId": {"type": "string"},
                            },
                            "required": ["projectId"],
                        },
                    },
                ]
            },
        )
        return

    if method == "tools/call":
        params = req.get("params") or {}
        name = params.get("name")
        args = params.get("arguments") or {}
        if name == "list_projects":
            respond(req_id, {"content": text_content(CANNED_PROJECTS)})
            return
        if name == "get_project":
            pid = args.get("projectId")
            match = next((p for p in CANNED_PROJECTS if p["id"] == pid), None)
            if match is not None:
                respond(req_id, {"content": text_content(match)})
            else:
                # Not-found shape: empty content body, isError false.
                respond(req_id, {"content": text_content(None)})
            return
        respond_error(req_id, -32601, f"unknown tool: {name}")
        return

    if method in ("ping", "shutdown", "exit"):
        respond(req_id, {})
        return

    respond_error(req_id, -32601, f"method not found: {method}")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            sys.stderr.write(f"mock_mcp_vercel: bad json: {e}\n")
            continue
        try:
            handle(req)
        except Exception as e:  # noqa: BLE001 — fixture, surface and continue
            sys.stderr.write(f"mock_mcp_vercel: handler error: {e}\n")


if __name__ == "__main__":
    main()
