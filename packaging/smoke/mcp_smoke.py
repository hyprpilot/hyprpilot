#!/usr/bin/env python3
"""Drive the three in-tree MCP servers over real stdio.

Two things the Rust tests cannot cover, because both are properties of
the serialized wire rather than of any function:

1. **Non-declaring clients see no change.** Every result still carries
   BOTH `content` and `structuredContent` — a structured-only result
   renders as "Unknown" in opencode — and no SEP-2663 task appears for a
   caller that never asked for one.
2. **Declaring clients get the task path.** The same `spawn` returns
   `resultType: "task"`, `tasks/get` resolves it, and the session handle
   is reachable from `_meta` without parsing the task id.

Usage:  python3 packaging/smoke/mcp_smoke.py [path-to-hyprpilot]
Exits non-zero on the first failed expectation.
"""

import json
import os
import queue
import subprocess
import sys
import threading
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/debug/hyprpilot"
SKILLS_DIR = os.path.expanduser("~/.config/nvim/utils/agents/skills")
TASKS_EXT = "io.modelcontextprotocol/tasks"

failures = []


def check(label, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {label}" + (f"  {detail}" if detail else ""))
    if not ok:
        failures.append(label)
    return ok


class Server:
    """A hyprpilot MCP subcommand spoken to over stdio."""

    def __init__(self, argv, config=None):
        cmd = [BIN] + (["--config", config] if config else []) + argv
        self.p = subprocess.Popen(
            cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.q = queue.Queue()
        threading.Thread(target=lambda: [self.q.put(l) for l in self.p.stdout],
                         daemon=True).start()
        self.n = 0

    def call(self, method, params=None, timeout=120, meta=None):
        self.n += 1
        params = dict(params or {})
        if meta:
            params["_meta"] = meta
        self.p.stdin.write(json.dumps(
            {"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                msg = json.loads(self.q.get(timeout=timeout))
            except Exception:
                continue
            if msg.get("id") == self.n:
                return msg
        return {}

    def initialize(self, version="2025-11-25", capabilities=None):
        r = self.call("initialize", {
            "protocolVersion": version,
            "capabilities": capabilities or {},
            "clientInfo": {"name": "smoke", "version": "0"},
        })
        self.p.stdin.write(json.dumps(
            {"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()
        return r.get("result", {})

    def stop(self):
        try:
            self.p.stdin.close()
        finally:
            self.p.terminate()


def tool_result(msg):
    return msg.get("result", {}) or {}


# ── 1. mcp serve ──────────────────────────────────────────────────────
print("=== hyprpilot mcp serve ===")
s = Server(["mcp", "serve"])
info = s.initialize()
check("serverInfo.name", info.get("serverInfo", {}).get("name") == "hyprpilot",
      info.get("serverInfo", {}).get("name"))
tools = [t["name"] for t in tool_result(s.call("tools/list")).get("tools", [])]
check("tools/list", tools == ["open"], str(tools))
s.stop()

# ── 2. mcp skills ─────────────────────────────────────────────────────
print("=== hyprpilot mcp skills ===")
s = Server(["mcp", "skills", "--skill-dir",
            json.dumps({"dir": SKILLS_DIR, "ignore": []})])
info = s.initialize()
check("serverInfo.name",
      info.get("serverInfo", {}).get("name") == "hyprpilot_skills",
      info.get("serverInfo", {}).get("name"))
listed = tool_result(s.call("tools/call", {"name": "list_skills", "arguments": {}}))
check("list_skills has content text",
      bool(listed.get("content")) and listed["content"][0].get("type") == "text")
check("list_skills has structuredContent",
      isinstance(listed.get("structuredContent"), dict))
s.stop()

# ── 3. mcp harness — the NON-declaring client (fallback path) ─────────
print("=== hyprpilot mcp harness — client WITHOUT tasks ===")
s = Server(["mcp", "harness"])
info = s.initialize()
check("serverInfo.name",
      info.get("serverInfo", {}).get("name") == "hyprpilot_harness",
      info.get("serverInfo", {}).get("name"))
caps = info.get("capabilities", {})
check("advertises claude/channel", "claude/channel" in (caps.get("experimental") or {}))
check("advertises tasks extension", TASKS_EXT in (caps.get("extensions") or {}),
      str(list((caps.get("extensions") or {}))))

profiles = tool_result(s.call("tools/call",
                              {"name": "list_profiles", "arguments": {}}))
check("list_profiles content + structured",
      bool(profiles.get("content")) and isinstance(profiles.get("structuredContent"), dict))
check("no resultType for a legacy peer", "resultType" not in profiles,
      str(sorted(profiles.keys())))
check("no task minted without declaring", profiles.get("resultType") != "task")
s.stop()

# ── 4. mcp harness — the DECLARING client (task path) ─────────────────
print("=== hyprpilot mcp harness — client WITH tasks ===")
s = Server(["mcp", "harness"])
info = s.initialize(version="2026-07-28",
                    capabilities={"extensions": {TASKS_EXT: {}}})
check("negotiated 2026-07-28", info.get("protocolVersion") == "2026-07-28",
      info.get("protocolVersion"))

profiles = tool_result(s.call("tools/call",
                              {"name": "list_profiles", "arguments": {}}))
check("an instant tool is NOT a task", profiles.get("resultType") != "task",
      str(profiles.get("resultType")))

print()
if failures:
    print(f"FAILED: {len(failures)} check(s): {', '.join(failures)}")
    sys.exit(1)
print("ALL PASS")
