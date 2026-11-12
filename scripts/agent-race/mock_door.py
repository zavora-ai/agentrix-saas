#!/usr/bin/env python3
"""A local stand-in for the converted site's agent door, for testing the new-site agent before
go-live. It mirrors crates/agentrix-node/src/door/mod.rs: AWP discovery, MCP over HTTP with an
Mcp-Session-Id, request_custom_order (held, idempotent by key), ask_site, and held-call status.

It is a test double, never the race's evidence: results against it name 127.0.0.1 as the target.

    python mock_door.py [--port 8765]
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CALLS: dict[str, dict] = {}
BY_KEY: dict[str, str] = {}

TOOLS = [
    {
        "name": "ask_site",
        "description": "Ask a question; the answer comes only from this site's pages, with citations, or says the site doesn't say.",
        "inputSchema": {"type": "object", "properties": {"question": {"type": "string", "maxLength": 800}},
                        "required": ["question"], "additionalProperties": False},
    },
    {
        "name": "request_custom_order",
        "description": "Request a custom order. It is held for the owner, who approves or declines it; poll the returned status. Nothing is charged.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "description": {"type": "string", "maxLength": 2000, "description": "What to make, in the customer's words."},
                "date": {"type": "string", "format": "date", "description": "When it is wanted, YYYY-MM-DD."},
                "name": {"type": "string", "maxLength": 200},
                "email": {"type": "string", "format": "email", "maxLength": 254},
            },
            "required": ["description", "date", "name", "email"],
            "additionalProperties": False,
        },
    },
]


def validate(args: dict) -> str | None:
    allowed = {"description", "date", "name", "email"}
    extra = set(args) - allowed
    if extra:
        return f"Unknown field: {sorted(extra)[0]}."
    for k in allowed:
        if not str(args.get(k, "")).strip():
            return f"{k} is required."
    if "@" not in args["email"]:
        return "email is not an email address."
    try:
        if dt.date.fromisoformat(args["date"]) < dt.date.today():
            return "date is in the past."
    except ValueError:
        return "date must be YYYY-MM-DD."
    return None


class Handler(BaseHTTPRequestHandler):
    def _send(self, code: int, body: dict | None = None, headers: dict | None = None, ctype="application/json"):
        data = json.dumps(body).encode() if body is not None else b""
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):  # quiet
        pass

    def do_GET(self):
        if self.path == "/.well-known/awp":
            return self._send(200, {
                "awp": "2.0", "site": "mock-door.local",
                "mcp": {"url": "/mcp", "protocolVersions": ["2025-06-18", "2025-03-26"]},
                "heldCalls": "/awp/v2/calls/{id}",
                "capabilities": {"capabilities": TOOLS},
                "terms": {"orders": "Every order is held for the owner's approval.", "payments": "No payment is taken through this door."},
            }, ctype="application/awp+json")
        if self.path.startswith("/awp/v2/calls/"):
            call = CALLS.get(self.path.rsplit("/", 1)[-1])
            return self._send(200, call) if call else self._send(404)
        self._send(404)

    def do_POST(self):
        if self.path != "/mcp":
            return self._send(404)
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        rid, method, params = body.get("id"), body.get("method", ""), body.get("params") or {}
        if "id" not in body:
            return self._send(202)
        def ok(result):
            return {"jsonrpc": "2.0", "id": rid, "result": result}
        if method == "initialize":
            return self._send(200, ok({"protocolVersion": "2025-06-18", "capabilities": {"tools": {"listChanged": False}},
                                       "serverInfo": {"name": "mock door", "version": "0"}}),
                              headers={"Mcp-Session-Id": str(uuid.uuid4())})
        if not self.headers.get("Mcp-Session-Id"):
            return self._send(400, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32000, "message": "No session."}})
        if method == "tools/list":
            return self._send(200, ok({"tools": TOOLS}))
        if method == "tools/call":
            name, args = params.get("name"), params.get("arguments") or {}
            if name == "request_custom_order":
                err = validate(args)
                if err:
                    return self._send(200, ok({"isError": True, "content": [{"type": "text", "text": err}]}))
                key = (params.get("_meta") or {}).get("agenticwebprotocol.com/idempotency-key") or json.dumps(args, sort_keys=True)
                cid = BY_KEY.setdefault(key, "call_" + uuid.uuid4().hex[:12])
                call = CALLS.setdefault(cid, {"id": cid, "capability": "request_custom_order", "state": "pending",
                                              "status": f"/awp/v2/calls/{cid}",
                                              "created_at": dt.datetime.now(dt.timezone.utc).isoformat()})
                text = f"Held for the owner's approval. Nothing is charged. Poll {call['status']} for the decision."
                return self._send(200, ok({"isError": False, "content": [{"type": "text", "text": text}], "structuredContent": call}))
            if name == "ask_site":
                answer = {"kind": "answer", "text": "Custom cakes need 3 days' notice; chocolate is one of four sponges [1]."}
                sources = [{"n": 1, "title": "Custom cakes", "url": "/custom-cakes/"}]
                return self._send(200, ok({"isError": False, "content": [{"type": "text", "text": answer["text"]}],
                                           "structuredContent": {"answer": answer, "sources": sources}}))
            return self._send(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32602, "message": "Unknown tool."}})
        self._send(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "Method not found."}})


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=8765)
    port = ap.parse_args().port
    print(f"mock door on http://127.0.0.1:{port} (a test double, not the converted site)", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
