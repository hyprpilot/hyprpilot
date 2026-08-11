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


def stub_config(profile_ids=("smoke",)):
    """Write a config whose 'vendor CLI' is the stub script.

    Generated rather than checked in because `command` is resolved
    against the SESSION's cwd, not the repo — a relative path here fails
    with a bare "No such file or directory" that looks like a spawn bug.

    Every profile is harness-enabled, so anything the delegate-scope
    section filters out was filtered by the SCOPE and not by the
    target-side opt-in.
    """
    path = os.path.join(tempfile.mkdtemp(prefix="hyprpilot-smoke-"), "config.toml")
    with open(path, "w") as fh:
        fh.write(
            "[[agents]]\n"
            'id = "stub"\n'
            'provider = "claude-code"\n'
            f'command = "{os.path.join(HERE, "stub-agent.sh")}"\n'
            "args = []\n"
        )
        for pid in profile_ids:
            fh.write(
                "\n[[profiles]]\n"
                f'id = "{pid}"\n'
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

# ── 2b. mcp skills — granular references ──────────────────────────────
# Against a PURPOSE-BUILT root rather than the live one: the assertions
# below pin exact names, counts and shadowing, and the live catalogue is
# edited constantly.
print("=== hyprpilot mcp skills — granular references ===")


def reference_fixture():
    """A skill declaring: a plain reference, one with its own
    frontmatter, a name collision, and a declared-but-missing file."""
    root = tempfile.mkdtemp(prefix="hyprpilot-refs-")
    shared = os.path.join(root, "references")
    os.makedirs(shared)
    with open(os.path.join(shared, "output-diff.md"), "w") as fh:
        fh.write("SHARED-OUTPUT-DIFF-BODY\n")
    with open(os.path.join(shared, "titled.md"), "w") as fh:
        fh.write("---\nname: renamed\ndisableModelInvocation: false\n---\nTITLED-BODY\n")
    with open(os.path.join(shared, "dup.md"), "w") as fh:
        fh.write("SHARED-DUP-BODY\n")

    skill = os.path.join(root, "refskill")
    os.makedirs(os.path.join(skill, "references"))
    with open(os.path.join(skill, "references", "dup.md"), "w") as fh:
        fh.write("LOCAL-DUP-BODY\n")
    with open(os.path.join(skill, "SKILL.md"), "w") as fh:
        fh.write(
            "---\ntitle: Ref Skill\ndescription: fixture\nreferences:\n"
            "  - ../references/output-diff.md\n"
            "  - ../references/titled.md\n"
            "  - ../references/dup.md\n"
            "  - ./references/dup.md\n"
            "  - ../references/gone.md\n"
            "---\nSKILL-BODY-MARKER\n"
        )
    return root


refroot = reference_fixture()
s = Server(["mcp", "skills", "--skill-dir",
            json.dumps({"dir": refroot, "ignore": []})])
s.initialize()

read = tool_result(s.call("tools/call", {
    "name": "read_skill", "arguments": {"slug": "refskill"}}))
sc = read.get("structuredContent", {})
manifest = sc.get("references", [])
text = read.get("content", [{}])[0].get("text", "")

check("read_skill does NOT bundle by default",
      "SHARED-OUTPUT-DIFF-BODY" not in text)
check("read_skill text carries the manifest footer",
      "skill_references:" in text and "uri: hyprpilot://references/refskill/output-diff" in text)
check("manifest names every declaration",
      [e.get("name") for e in manifest] == ["output-diff", "renamed", "dup", "dup", "gone"],
      str([e.get("name") for e in manifest]))
check("a reference's own frontmatter renames it and survives",
      manifest[1].get("metadata", {}).get("disableModelInvocation") is False)
check("a reference defaults to disableModelInvocation",
      manifest[0].get("metadata", {}).get("disableModelInvocation") is True)
check("the shadowed duplicate has no uri of its own",
      manifest[3].get("shadowed") is True and "uri" not in manifest[3])
check("a declared-but-missing file is marked, not dropped",
      manifest[4].get("status") == "not-found")
check("timestamps are RFC 3339 UTC",
      manifest[0].get("modified", "").endswith("Z") and len(manifest[0].get("modified", "")) == 20,
      manifest[0].get("modified"))
check("no declared path reaches the wire",
      "../references/" not in json.dumps(sc))
check("metadata drops the raw references array",
      "references" not in sc.get("metadata", {}))

bundled = tool_result(s.call("tools/call", {
    "name": "read_skill", "arguments": {"slug": "refskill", "bundle": True}}))
btext = bundled.get("content", [{}])[0].get("text", "")
check("bundle:true carries the bodies", "SHARED-OUTPUT-DIFF-BODY" in btext)

one = tool_result(s.call("resources/read", {
    "uri": "hyprpilot://references/refskill/output-diff"}))
otext = one.get("contents", [{}])[0].get("text", "")
check("single-reference URI returns just that reference",
      "SHARED-OUTPUT-DIFF-BODY" in otext and "TITLED-BODY" not in otext)

missing = s.call("resources/read", {"uri": "hyprpilot://references/refskill/nope"})
check("an unknown reference name errors and lists what exists",
      "nope" in json.dumps(missing.get("error", {}))
      and "output-diff" in json.dumps(missing.get("error", {})))

picked = tool_result(s.call("tools/call", {
    "name": "load_skill_references",
    "arguments": {"slug": "refskill", "references": ["output-diff", "renamed"]}}))
ptext = picked.get("structuredContent", {}).get("body", "")
check("an array fetches exactly the named references",
      "SHARED-OUTPUT-DIFF-BODY" in ptext and "TITLED-BODY" in ptext
      and "SHARED-DUP-BODY" not in ptext)

empty = tool_result(s.call("tools/call", {
    "name": "load_skill_references",
    "arguments": {"slug": "refskill", "references": []}}))
check("an EMPTY array fetches nothing, not everything",
      empty.get("structuredContent", {}).get("body") == "")

every = tool_result(s.call("tools/call", {
    "name": "load_skill_references", "arguments": {"slug": "refskill"}}))
etext = every.get("structuredContent", {}).get("body", "")
check("an omitted array fetches everything",
      all(m in etext for m in
          ("SHARED-OUTPUT-DIFF-BODY", "TITLED-BODY", "SHARED-DUP-BODY", "LOCAL-DUP-BODY")))

res = tool_result(s.call("resources/read", {"uri": "hyprpilot://skills/refskill"}))
rtext = res.get("contents", [{}])[0].get("text", "")
check("the skill RESOURCE stops bundling but keeps the footer",
      "SHARED-OUTPUT-DIFF-BODY" not in rtext and "skill_references:" in rtext)

templates = [t["uriTemplate"] for t in
             tool_result(s.call("resources/templates/list")).get("resourceTemplates", [])]
check("the per-reference template is advertised",
      "hyprpilot://references/{slug}/{reference}" in templates, str(templates))
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

# ── 5. mcp harness — the per-launch delegate scope ────────────────────
#
# Both halves have to hold. Filtering only `list_profiles` would leave a
# scoped-out profile reachable by anyone holding its id — the same
# gate-only-the-listing bug the server split exists to prevent — so each
# case below checks the listing AND a launch.
print("=== hyprpilot mcp harness — delegate scope ===")
SCOPE_CFG = stub_config(("personal/one", "personal/two", "work/one"))


def listed(server):
    got = tool_result(server.call("tools/call", {"name": "list_profiles", "arguments": {}}))
    rows = (got.get("structuredContent") or {}).get("profiles") or []
    return sorted(row.get("id") for row in rows), got


def refusal(server, profile):
    got = tool_result(server.call("tools/call", {
        "name": "spawn", "arguments": {"profile": profile, "prompt": "go", "cwd": "/tmp"}}))
    text = " ".join(c.get("text", "") for c in (got.get("content") or []))
    return got.get("isError") is True, text


s = Server(["mcp", "harness", "--include-profile", "personal/*",
            "--exclude-profile", "personal/two"], config=SCOPE_CFG)
s.initialize()
ids, _ = listed(s)
check("scope narrows the listing", ids == ["personal/one"], str(ids))

refused, text = refusal(s, "work/one")
check("a launch outside the scope is refused, not just hidden", refused, text[:80])
check("the refusal names the knob that widens it", "includeProfiles" in text, text[:120])

refused, _ = refusal(s, "personal/two")
check("exclude beats include on a launch too", refused)

# `launch` is the shared body of spawn AND session_send, and the scope
# check sits at the top of it — so a resume must still clear the gate
# without the gate breaking resumes. There is no way to hold a session
# for an out-of-scope profile (spawn refuses first), which is the point:
# the only reachable case is the in-scope one, and it has to keep working.
started = tool_result(s.call("tools/call", {
    "name": "spawn",
    "arguments": {"profile": "personal/one", "prompt": "go", "cwd": "/tmp"}}))
in_scope = (started.get("structuredContent") or {}).get("session", "")
check("an in-scope profile still launches", bool(in_scope), str(started.get("isError")))
# The vendor's own id is harvested lazily, from the transcript, so a
# resume needs turn 1 to have finished — otherwise the refusal is
# "the vendor never emitted a session id" and says nothing about scope.
for _ in range(40):
    st = tool_result(s.call("tools/call",
                            {"name": "session_status", "arguments": {"session": in_scope}}))
    if (st.get("structuredContent") or {}).get("status") == "exited":
        break
    time.sleep(0.5)
resumed = tool_result(s.call("tools/call", {
    "name": "session_send", "arguments": {"session": in_scope, "prompt": "again"}}))
check("the scope does not break resuming an in-scope session",
      resumed.get("isError") is not True,
      " ".join(c.get("text", "") for c in (resumed.get("content") or []))[:80])
s.stop()

# `--no-delegates` is the empty include list. The server is still there —
# it just has no candidates.
s = Server(["mcp", "harness", "--no-delegates"], config=SCOPE_CFG)
info = s.initialize()
check("an empty scope still serves the harness",
      info.get("serverInfo", {}).get("name") == "hyprpilot_harness")
ids, got = listed(s)
check("no-delegates lists nothing", ids == [], str(ids))
summary = " ".join(c.get("text", "") for c in (got.get("content") or []))
check("the empty listing names the scope, not only the opt-in",
      "includeProfiles" in summary, summary[:120])
refused, _ = refusal(s, "personal/one")
check("no-delegates refuses every launch", refused)
s.stop()

# The unscoped default has to stay exactly as it was.
s = Server(["mcp", "harness"], config=SCOPE_CFG)
s.initialize()
ids, _ = listed(s)
check("no flags means unrestricted",
      ids == ["personal/one", "personal/two", "work/one"], str(ids))
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

