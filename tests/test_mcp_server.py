"""Unit and MCP protocol tests for scripts/relay_mcp_server.py."""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
SERVER = ROOT / "plugins" / "agent-relay" / "scripts" / "relay_mcp_server.py"


class McpServerTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.home = self.base / "home"
        self.relay_home = self.base / "relay-home"
        self.gemini_home = self.base / "gemini-home"
        self.codex_home = self.base / "codex-home"
        self.data_dir = self.base / "data"
        for d in (self.home, self.relay_home, self.gemini_home, self.codex_home, self.data_dir):
            d.mkdir(parents=True, exist_ok=True)
        self.env = dict(
            os.environ,
            HOME=str(self.home),
            AGENT_RELAY_HOME=str(self.relay_home),
            AGENT_RELAY_SELF="testhost",
            AGENT_RELAY_GEMINI_HOME=str(self.gemini_home),
            CODEX_HOME=str(self.codex_home),
            AGENT_RELAY_DATA_DIR=str(self.data_dir),
            AGENT_RELAY_OPENCODE_BIN=str(self.base / "no-opencode"),
        )
        self.env.pop("CLAUDE_CONFIG_DIR", None)
        self._seed_synthetic_data()

    def tearDown(self):
        self.tmp.cleanup()

    def _seed_synthetic_data(self):
        now_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        now_ms = int(time.time() * 1000)

        # 1. Claude Code session + transcript + delivered inbox message
        c_sid = "11111111-2222-3333-4444-555555555555"
        c_proj = self.home / ".claude" / "projects" / "demo-app"
        c_proj.mkdir(parents=True, exist_ok=True)
        c_tx = c_proj / f"{c_sid}.jsonl"
        c_tx.write_text(
            "\n".join(
                [
                    json.dumps(
                        {
                            "type": "user",
                            "timestamp": now_iso,
                            "cwd": "/work/demo-app",
                            "message": {"content": "Fix the widget parser"},
                        }
                    ),
                    json.dumps(
                        {
                            "type": "assistant",
                            "timestamp": now_iso,
                            "message": {
                                "id": "msg-1",
                                "model": "claude-sonnet-4-6",
                                "usage": {
                                    "input_tokens": 1200,
                                    "output_tokens": 300,
                                    "cache_read_input_tokens": 5000,
                                    "cache_creation_input_tokens": 400,
                                },
                                "content": [{"type": "text", "text": "Working on widget parser."}],
                            },
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        sess_dir = self.relay_home / "sessions"
        sess_dir.mkdir(parents=True, exist_ok=True)
        (sess_dir / f"claude-{c_sid}.json").write_text(
            json.dumps(
                {
                    "agent": "claude",
                    "id": c_sid,
                    "cwd": "/work/demo-app",
                    "label": "demo-claude",
                    "state": "idle",
                    "pid": os.getpid(),
                    "transcript_path": str(c_tx),
                }
            ),
            encoding="utf-8",
        )

        # 2. Codex rollout
        x_tid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        x_dir = self.codex_home / "sessions" / "2026" / "11" / "25"
        x_dir.mkdir(parents=True, exist_ok=True)
        (x_dir / f"rollout-2026-11-25-{x_tid}.jsonl").write_text(
            json.dumps(
                {
                    "type": "session_meta",
                    "timestamp": now_iso,
                    "payload": {"cwd": "/work/demo-app"},
                }
            )
            + "\n"
            + json.dumps(
                {
                    "type": "event_msg",
                    "timestamp": now_iso,
                    "payload": {"type": "user_message", "message": "Review parser tests"},
                }
            )
            + "\n",
            encoding="utf-8",
        )

        # 3. Antigravity conversation + swarm message
        a_cid = "99999999-8888-7777-6666-555555555555"
        a_logs = self.gemini_home / "antigravity" / "brain" / a_cid / ".system_generated" / "logs"
        a_logs.mkdir(parents=True, exist_ok=True)
        (a_logs / "transcript.jsonl").write_text(
            json.dumps(
                {
                    "type": "USER_INPUT",
                    "created_at": now_iso,
                    "content": "<USER_REQUEST>Coordinate release</USER_REQUEST>",
                }
            )
            + "\n",
            encoding="utf-8",
        )

        # 4. Pi session
        p_dir = self.home / ".pi" / "agent" / "sessions" / "demo"
        p_dir.mkdir(parents=True, exist_ok=True)
        (p_dir / "20261125_pi123456.jsonl").write_text(
            json.dumps({"type": "session", "timestamp": now_iso, "cwd": "/work/demo-app"})
            + "\n"
            + json.dumps(
                {
                    "type": "message",
                    "timestamp": now_iso,
                    "message": {"role": "user", "content": "Run pi check"},
                }
            )
            + "\n",
            encoding="utf-8",
        )

        # 5. Inbox envelope
        inb = self.relay_home / "inbox" / c_sid
        inb.mkdir(parents=True, exist_ok=True)
        (inb / "20261125T120000-deadbeef.json").write_text(
            json.dumps(
                {
                    "id": "20261125T120000-deadbeef",
                    "from": f"codex:{x_tid}@testhost",
                    "to": f"claude:{c_sid}",
                    "subject": "Tests green",
                    "body": "All parser tests passed.",
                    "sent_at_ms": now_ms - 1000,
                }
            ),
            encoding="utf-8",
        )
        self.c_sid = c_sid

    def _rpc(self, requests):
        stdin_data = "\n".join(json.dumps(r) for r in requests) + "\n"
        proc = subprocess.run(
            [sys.executable, "-B", str(SERVER), "--data-dir", str(self.data_dir)],
            input=stdin_data,
            capture_output=True,
            text=True,
            env=self.env,
            timeout=30,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]

    def test_initialize_resources_and_tools_list(self):
        resps = self._rpc(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}},
                {"jsonrpc": "2.0", "id": 2, "method": "ping"},
                {"jsonrpc": "2.0", "id": 3, "method": "resources/list"},
                {"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "ui://agent-relay/relay-map.html"}},
                {"jsonrpc": "2.0", "id": 5, "method": "tools/list"},
            ]
        )
        by_id = {r["id"]: r["result"] for r in resps}
        self.assertEqual(by_id[1]["serverInfo"]["name"], "agent-relay")
        self.assertTrue(by_id[1]["capabilities"]["resources"] is not None)
        self.assertEqual(by_id[2], {})
        self.assertEqual(by_id[3]["resources"][0]["uri"], "ui://agent-relay/relay-map.html")
        content = by_id[4]["contents"][0]
        self.assertEqual(content["mimeType"], "text/html;profile=mcp-app")
        self.assertIn("RELAY<b>MAP</b>", content["text"])
        self.assertNotIn("fonts.googleapis.com", content["text"])

        tools = {t["name"]: t for t in by_id[5]["tools"]}
        self.assertIn("relay_map", tools)
        meta = tools["relay_map"]["_meta"]
        self.assertEqual(meta["ui"]["resourceUri"], "ui://agent-relay/relay-map.html")
        eps = meta["com.google.antigravity/ui"]["entrypoints"]
        self.assertEqual({e["type"] for e in eps}, {"aux_pane", "global"})
        self.assertEqual(tools["relay_map_poll"]["_meta"]["ui"]["visibility"], ["app"])
        self.assertEqual(tools["relay_map_prefs"]["_meta"]["ui"]["visibility"], ["app"])

    def test_relay_map_collects_agents_prefs_broadcast_and_transcript(self):
        c_addr = f"claude:{self.c_sid}@testhost"
        resps = self._rpc(
            [
                {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "relay_map", "arguments": {}}},
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {"name": "relay_map_prefs", "arguments": {"patch": {"color_by": "health", "pins": [c_addr]}}},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {"name": "relay_broadcast", "arguments": {"scope": "all", "dry_run": True}},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/call",
                    "params": {"name": "relay_transcript", "arguments": {"addr": c_addr}},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 5,
                    "method": "tools/call",
                    "params": {"name": "relay_transcript", "arguments": {"addr": "claude:nonexistent@testhost"}},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 6,
                    "method": "tools/call",
                    "params": {"name": "relay_inbox", "arguments": {"session": self.c_sid, "peek": True}},
                },
            ]
        )
        by_id = {r["id"]: r["result"] for r in resps}
        snap = by_id[1]["structuredContent"]
        kinds = {a["kind"] for a in snap["agents"]}
        self.assertTrue({"claude", "codex", "antigravity", "pi", "operator"}.issubset(kinds))
        self.assertGreaterEqual(len(snap["messages"]), 1)

        prefs = by_id[2]["structuredContent"]["prefs"]
        self.assertEqual(prefs["color_by"], "health")
        self.assertEqual(prefs["pins"], [c_addr])

        bc = by_id[3]["structuredContent"]
        self.assertIn(c_addr, bc["targets"])

        tr = by_id[4]["structuredContent"]
        self.assertEqual(tr["source"], "local")
        self.assertGreaterEqual(len(tr["entries"]), 2)
        self.assertEqual(tr["entries"][0]["role"], "user")

        self.assertTrue(by_id[5].get("isError"))
        self.assertEqual(len(by_id[6]["structuredContent"]["messages"]), 1)


if __name__ == "__main__":
    unittest.main()
