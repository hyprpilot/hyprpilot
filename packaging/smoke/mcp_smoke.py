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
Runs every check, then exits non-zero if any of them failed.
"""

import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time

BIN = sys.argv[1] if len(sys.argv) > 1 else "./target/debug/hyprpilot"
SKILLS_DIR = os.path.expanduser("~/.config/nvim/utils/agents/skills")
TASKS_EXT = "io.modelcontextprotocol/tasks"
HERE = os.path.dirname(os.path.abspath(__file__))


def stub_config():
    """Write a config whose 'vendor CLI' is the stub script.

    Generated rather than checked in because `command` is resolved
    against the SESSION's cwd, not the repo — a relative path here fails
    with a bare "No such file or directory" that looks like a spawn bug.
    """
    path = os.path.join(tempfile.mkdtemp(prefix="hyprpilot-smoke-"), "config.toml")
    with open(path, "w") as fh:
        fh.write(
            "[[agents]]\n"
            'id = "stub"\n'
            'provider = "claude-code"\n'
            f'command = "{os.path.join(HERE, "stub-agent.sh")}"\n'
            "args = []\n\n"
            "[[profiles]]\n"
            'id = "smoke"\n'
            'agent = "stub"\n'
            "harness = { enabled = true }\n"
            "mcp = { enabled = false }\n"
        )
    return path

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
#
# Uses a stub vendor CLI (packaging/smoke/stub-agent.sh) so this needs no
# model, network or credentials. Everything below was a real bug at some
# point: cancelling a finished task used to kill the running turn and
# delete the transcript, and a terminal task's lastUpdatedAt used to move
# on every poll. Assertions that cannot fail are worse than none.
print("=== hyprpilot mcp harness — client WITH tasks (task path) ===")
s = Server(["mcp", "harness"], config=stub_config())
info = s.initialize(version="2026-07-28", capabilities={"extensions": {TASKS_EXT: {}}})
check("negotiated 2026-07-28", info.get("protocolVersion") == "2026-07-28",
      info.get("protocolVersion"))

spawned = tool_result(s.call("tools/call", {
    "name": "spawn",
    "arguments": {"profile": "smoke", "prompt": "go", "cwd": "/tmp"}}))
check("spawn returns a task", spawned.get("resultType") == "task",
      str(spawned.get("resultType")))
task_id = spawned.get("taskId", "")
handle = (spawned.get("_meta") or {}).get("io.hyprpilot/session", "")
check("handle reachable without parsing the id", bool(handle) and handle in task_id,
      f"{handle!r} in {task_id!r}")

# Poll to a terminal state.
status, got = None, {}
for _ in range(40):
    got = tool_result(s.call("tasks/get", {"taskId": task_id}))
    status = got.get("status")
    if status != "working":
        break
    time.sleep(0.5)
check("tasks/get reaches a terminal state", status == "completed", str(status))
inner = got.get("result") or {}
check("completed result has content text",
      bool(inner.get("content")) and inner["content"][0].get("type") == "text")
check("completed result has structuredContent", "structuredContent" in inner)
check("tasks/get carries the session handle",
      (got.get("_meta") or {}).get("io.hyprpilot/session") == handle)

# A terminal task must not change — SEP-2663 says terminal states are
# immutable, so `lastUpdatedAt` may not track the observation.
first = got.get("lastUpdatedAt")
time.sleep(1.1)
again = tool_result(s.call("tasks/get", {"taskId": task_id}))
check("terminal task does not change on re-poll",
      again.get("status") == "completed" and again.get("lastUpdatedAt") == first,
      f"{first} -> {again.get('lastUpdatedAt')}")
check("createdAt agrees between mint and poll",
      again.get("createdAt") == spawned.get("createdAt"),
      f"{spawned.get('createdAt')} vs {again.get('createdAt')}")

# Turn 2, then turn 1 must still be terminal.
sent = tool_result(s.call("tools/call", {
    "name": "session_send", "arguments": {"session": handle, "prompt": "again"}}))
turn2 = sent.get("taskId", "")
check("session_send returns its own task", sent.get("resultType") == "task" and turn2 != task_id,
      f"{task_id} -> {turn2}")
check("turn 1 stays terminal after turn 2 starts",
      tool_result(s.call("tasks/get", {"taskId": task_id})).get("status") == "completed")

# Cancelling a FINISHED task must be a no-op — it must not touch the
# conversation, and must not reap the transcript.
s.call("tasks/cancel", {"taskId": task_id})
check("cancelling a finished task leaves it completed",
      tool_result(s.call("tasks/get", {"taskId": task_id})).get("status") == "completed")
check("cancelling a finished task does not destroy the session",
      "error" not in s.call("tasks/get", {"taskId": turn2}))
check("the session is still addressable by its own tools",
      "error" not in s.call("tools/call",
                            {"name": "session_status", "arguments": {"session": handle}}))
s.stop()

# ── verdict ───────────────────────────────────────────────────────────
#
# Without this the script printed FAIL and exited 0, so every assertion
# above was advisory — the same "a test that cannot fail" trap the task
# section was written to close.
print()
if failures:
    print(f"FAILED ({len(failures)}):")
    for label in failures:
        print(f"  - {label}")
    sys.exit(1)
print("all checks passed")

