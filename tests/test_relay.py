"""Unit tests for scripts/relay.py. Each test uses a temporary AGENT_RELAY_HOME."""
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.dont_write_bytecode = True  # keep __pycache__ out of the plugin package
SCRIPT = Path(__file__).resolve().parents[1] / "plugins" / "agent-relay" / "scripts" / "relay.py"
spec = importlib.util.spec_from_file_location("relay", SCRIPT)
relay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(relay)

SID = "11111111-2222-3333-4444-555555555555"


class RelayTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.env = {"AGENT_RELAY_HOME": self.tmp.name, "AGENT_RELAY_SELF": "here",
                    "AGENT_RELAY_GEMINI_HOME": str(Path(self.tmp.name) / "no-gemini")}
        self.saved = {k: os.environ.get(k) for k in
                      list(self.env) + ["CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID", "AGENT_RELAY_FROM",
                                        "CLAUDE_CODE_ENTRYPOINT", "ANTIGRAVITY_CONVERSATION_ID",
                                        "AGENT_RELAY_GEMINI_HOME", "AGENT_RELAY_PI_SESSION",
                                        "AGENT_RELAY_OPENCODE_BIN"]}
        os.environ.update(self.env)
        for k in ("CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID", "AGENT_RELAY_FROM", "CLAUDE_CODE_ENTRYPOINT",
                  "ANTIGRAVITY_CONVERSATION_ID", "AGENT_RELAY_PI_SESSION"):
            os.environ.pop(k, None)
        # never reach a real opencode service from the tests
        os.environ["AGENT_RELAY_OPENCODE_BIN"] = str(Path(self.tmp.name) / "no-opencode")

    def tearDown(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        self.tmp.cleanup()

    def run_cli(self, *args, stdin=""):
        env = dict(os.environ)
        return subprocess.run([sys.executable, str(SCRIPT), *args], input=stdin, capture_output=True,
                              text=True, env=env, timeout=60)

    def hook(self, event, payload, agent="claude"):
        return self.run_cli("hook", "--agent", agent, event, stdin=json.dumps(payload))


class Addresses(RelayTestCase):
    def test_parse(self):
        self.assertEqual(relay.parse_addr("codex:billing@laptop"), ("codex", "billing", "laptop"))
        self.assertEqual(relay.parse_addr("claude:abc"), ("claude", "abc", None))
        self.assertEqual(relay.parse_addr("claude:abc@here"), ("claude", "abc", None))   # self
        self.assertEqual(relay.parse_addr("claude:abc@localhost"), ("claude", "abc", None))
        self.assertEqual(relay.parse_addr("codex:x", host="pc"), ("codex", "x", "pc"))

    def test_bad_address(self):
        self.assertEqual(relay.parse_addr("antigravity:deal finder@laptop"), ("antigravity", "deal finder", "laptop"))
        for bad in ("billing", "gemini:x", "codex:", "codex: ", "claude:x@bad host"):
            with self.assertRaises(relay.RelayError):
                relay.parse_addr(bad)

    def test_detect_self(self):
        os.environ["CODEX_THREAD_ID"] = "t1"
        self.assertEqual(relay.detect_self(), "codex:t1@here")
        del os.environ["CODEX_THREAD_ID"]
        os.environ["CLAUDE_CODE_SESSION_ID"] = SID
        self.assertEqual(relay.detect_self(), f"claude:{SID}@here")


class Envelope(RelayTestCase):
    def test_render_marks_peer_origin_and_reply(self):
        msg = relay.make_message("claude:x", "run the tests", "codex:t1@pc", "tests")
        text = relay.render(msg, "relay")
        self.assertIn('from="codex:t1@pc"', text)
        self.assertIn("not typed by the user", text)
        self.assertIn("relay send --to codex:t1@pc", text)

    def test_large_body_spills_to_file(self):
        msg = relay.spill_if_large(relay.make_message("codex:t", "y" * 7000, None, None))
        self.assertTrue(Path(msg["body_path"]).read_text() == "y" * 7000)
        self.assertLess(len(msg["body"]), 300)
        small = relay.spill_if_large(relay.make_message("codex:t", "short", None, None))
        self.assertNotIn("body_path", small)


class Hooks(RelayTestCase):
    def test_session_start_registers_and_announces_address(self):
        r = self.hook("session-start", {"session_id": SID, "cwd": "/work/billing-api", "transcript_path": "/t.jsonl"})
        self.assertEqual(r.returncode, 0, r.stderr)
        ctx = json.loads(r.stdout)["hookSpecificOutput"]
        self.assertEqual(ctx["hookEventName"], "SessionStart")
        self.assertIn(f"claude:{SID}@here", ctx["additionalContext"])
        recs = relay.read_registry("claude")
        self.assertEqual([(x["id"], x["cwd"], x["state"]) for x in recs], [(SID, "/work/billing-api", "idle")])

    def test_prompt_delivers_mail_once_and_sets_busy(self):
        self.hook("session-start", {"session_id": SID, "cwd": "/w/a"})
        r = self.run_cli("send", "--to", f"claude:{SID}", "-m", "hello", "--from", "codex:t@pc")
        self.assertEqual(r.returncode, 0, r.stderr)
        first = self.hook("prompt", {"session_id": SID})
        self.assertIn("hello", json.loads(first.stdout)["hookSpecificOutput"]["additionalContext"])
        self.assertEqual(self.hook("prompt", {"session_id": SID}).stdout, "")
        self.assertEqual(relay.read_registry("claude")[0]["state"], "busy")
        self.hook("stop", {"session_id": SID})
        self.assertEqual(relay.read_registry("claude")[0]["state"], "idle")

    def test_session_open_before_install_is_backfilled(self):
        self.hook("prompt", {"session_id": SID, "cwd": "/w/old", "transcript_path": "/t.jsonl"})
        rec = relay.read_registry("claude")[0]
        self.assertEqual((rec["cwd"], rec["transcript_path"], rec["state"]), ("/w/old", "/t.jsonl", "busy"))
        self.assertTrue(rec.get("pid"))

    def test_bad_input_never_fails(self):
        for payload in ({}, {"session_id": "../../etc"}, "not json"):
            r = self.run_cli("hook", "--agent", "claude", "prompt",
                             stdin=payload if isinstance(payload, str) else json.dumps(payload))
            self.assertEqual(r.returncode, 0)

    def test_watch_skips_headless_sessions(self):
        os.environ["CLAUDE_CODE_ENTRYPOINT"] = "sdk-cli"
        try:
            start = time.time()
            r = self.hook("watch", {"session_id": SID})
        finally:
            os.environ.pop("CLAUDE_CODE_ENTRYPOINT", None)
        self.assertEqual(r.returncode, 0)
        self.assertLess(time.time() - start, 5)

    def test_watch_wakes_with_exit_2_and_is_single_instance(self):
        os.environ["AGENT_RELAY_WATCH_SECONDS"] = "15"
        try:
            env = dict(os.environ)
            first = subprocess.Popen([sys.executable, str(SCRIPT), "hook", "--agent", "claude", "watch"],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
            first.stdin.write(json.dumps({"session_id": SID}))
            first.stdin.close()
            time.sleep(1)
            second = self.hook("watch", {"session_id": SID})
            self.assertEqual(second.returncode, 0)             # lock held by the first
            relay.deliver_to_mailbox(SID, relay.make_message(f"claude:{SID}", "wake up", "codex:t@pc", None))
            first.wait(timeout=15)
            err = first.stderr.read()
            first.stdout.close()
            first.stderr.close()
            self.assertEqual(first.returncode, 2)
            self.assertIn("wake up", err)
            self.assertEqual(relay.pending(SID), [])
            self.assertFalse((Path(self.tmp.name) / "watch" / f"{SID}.pid").exists())
        finally:
            os.environ.pop("AGENT_RELAY_WATCH_SECONDS", None)


class ClaudeRouting(RelayTestCase):
    def register(self, sid, cwd, label=None):
        relay.update_registry("claude", sid, cwd=cwd, pid=os.getpid(), state="idle", label=label)

    def test_resolve_by_prefix_folder_and_label(self):
        self.register(SID, "/src/billing-api")
        self.register("99999999-2222-3333-4444-555555555555", "/src/webapp", label="reviewer")
        self.assertEqual(relay.resolve_claude("111111")["id"], SID)
        self.assertEqual(relay.resolve_claude("billing-api")["id"], SID)
        self.assertTrue(relay.resolve_claude("reviewer")["id"].startswith("9999"))

    def test_ambiguous_and_missing(self):
        self.register(SID, "/a/same")
        self.register("99999999-2222-3333-4444-555555555555", "/b/same")
        with self.assertRaises(relay.RelayError):
            relay.resolve_claude("same")
        with self.assertRaises(relay.RelayError):
            relay.resolve_claude("nothing")
        self.assertTrue(relay.resolve_claude("12345678-1234-1234-1234-123456789abc")["unregistered"])

    def test_dead_sessions_hidden_and_collected(self):
        relay.update_registry("claude", SID, cwd="/x", pid=2 ** 22 + 12345, state="idle")
        out = io.StringIO()
        with redirect_stdout(out):
            relay.main(["sessions", "--json"])
        self.assertEqual(json.loads(out.getvalue())["claude"], [])
        with redirect_stdout(io.StringIO()):
            relay.main(["gc"])
        self.assertEqual(relay.read_registry(), [])

    def test_inbox_and_read_transcript(self):
        transcript = Path(self.tmp.name) / "t.jsonl"
        lines = [{"type": "user", "message": {"content": "hi"}},
                 {"type": "assistant", "timestamp": "2026-11-14T20:00:00Z",
                  "message": {"content": [{"type": "text", "text": "tests pass"}]}}]
        transcript.write_text("\n".join(json.dumps(x) for x in lines))
        relay.update_registry("claude", SID, cwd="/x", pid=os.getpid(), state="idle",
                              transcript_path=str(transcript))
        r = self.run_cli("read", f"claude:{SID}", "--json")
        self.assertEqual(json.loads(r.stdout)["messages"][0]["text"], "tests pass")
        self.run_cli("send", "--to", f"claude:{SID}", "-m", "ping", "--from", "codex:t@pc")
        os.environ["CLAUDE_CODE_SESSION_ID"] = SID
        r = self.run_cli("inbox", "--json")
        self.assertEqual(json.loads(r.stdout)[0]["body"], "ping")
        self.assertEqual(json.loads(self.run_cli("inbox", "--json").stdout), [])


CID = "bbbbbbbb-1111-4222-8333-444444444444"
CID2 = "aaaaaaaa-1111-4222-8333-444444444444"


class Antigravity(RelayTestCase):
    """Runs against a fake ~/.gemini tree (AGENT_RELAY_GEMINI_HOME)."""

    def setUp(self):
        super().setUp()
        self.gem = Path(self.tmp.name) / "gemini"
        os.environ["AGENT_RELAY_GEMINI_HOME"] = str(self.gem)
        self.make_conv("antigravity", CID, "Relay test (app)")
        self.make_conv("antigravity-cli", CID2, "Relay test (cli)",
                       transcript=[{"source": "USER_EXPLICIT", "type": "USER_INPUT", "content": "hi",
                                    "created_at": "2026-11-16T15:49:35Z"},
                                   {"source": "MODEL", "type": "PLANNER_RESPONSE", "content": "None",
                                    "created_at": "2026-11-16T15:49:36Z"},
                                   {"source": "MODEL", "type": "PLANNER_RESPONSE", "content": "READY",
                                    "created_at": "2026-11-16T15:49:37Z"}])

    def make_conv(self, product, cid, title, transcript=()):
        import sqlite3
        root = self.gem / product
        (root / "brain" / cid / ".system_generated" / "logs").mkdir(parents=True)
        con = sqlite3.connect(root / "conversation_summaries.db")
        con.execute("create table if not exists conversation_summaries (conversation_id text primary key, "
                    "title text, status text, last_modified_time text, workspace_uris text, killed numeric, "
                    "parent_conversation_id text)")
        con.execute("insert into conversation_summaries values (?,?,?,?,?,?,?)",
                    (cid, title, "CASCADE_RUN_STATUS_IDLE", "2026-11-16 15:50:12+00:00",
                     '["file:///w/proj"]', 0, ""))
        con.commit()
        con.close()
        (root / "brain" / cid / ".system_generated" / "logs" / "transcript.jsonl").write_text(
            "\n".join(json.dumps(t) for t in transcript))

    def fake_hub(self, product="antigravity", response='{"response": {"sendMessage": {}}}'):
        (self.gem / product / "daemon").mkdir(parents=True, exist_ok=True)
        (self.gem / product / "daemon" / "hub_1234.json").write_text(json.dumps(
            {"pid": os.getpid(), "httpPort": 1234, "csrfToken": "secret-token"}))
        exe = self.gem / product / "bin" / "agentapi"
        exe.parent.mkdir(parents=True, exist_ok=True)
        log = Path(self.tmp.name) / "agentapi.log"
        exe.write_text("#!/bin/sh\n"
                       f"printf '%s\\n' \"$ANTIGRAVITY_LS_ADDRESS\" \"${{ANTIGRAVITY_CSRF_TOKEN:+token-set}}\" \"$@\" > {log}\n"
                       f"echo '{response}'\n")
        exe.chmod(0o755)
        return log

    def test_address_and_self(self):
        self.assertEqual(relay.parse_addr(f"antigravity:{CID}@laptop"), ("antigravity", CID, "laptop"))
        os.environ["ANTIGRAVITY_CONVERSATION_ID"] = CID
        try:
            self.assertEqual(relay.detect_self(), f"antigravity:{CID}@here")
        finally:
            del os.environ["ANTIGRAVITY_CONVERSATION_ID"]

    def test_find_by_prefix_and_title(self):
        self.assertEqual(relay.agy_find("bbbbbb")["surface"], "app")
        conv = relay.agy_find("Relay test (cli)")
        self.assertEqual((conv["id"], conv["surface"], conv["status"], conv["cwd"]), (CID2, "cli", "idle", "/w/proj"))
        with self.assertRaises(relay.RelayError):
            relay.agy_find("no such conversation")

    def test_unsummarised_conversation_found_by_folder(self):
        cid = "cccccccc-1111-4222-8333-444444444444"
        logs = self.gem / "antigravity" / "brain" / cid / ".system_generated" / "logs"
        logs.mkdir(parents=True)
        self.assertRaises(relay.RelayError, relay.agy_find, "cccccccc")      # no transcript yet
        (logs / "transcript.jsonl").write_text("")
        self.assertEqual(relay.agy_find("cccccccc")["id"], cid)

    def test_conversation_indexed_by_two_surfaces_resolves_to_its_transcript(self):
        import sqlite3
        con = sqlite3.connect(self.gem / "antigravity" / "conversation_summaries.db")
        con.execute("insert into conversation_summaries values (?,?,?,?,?,?,?)",
                    (CID2, "", "CASCADE_RUN_STATUS_IDLE", "2026-11-16 15:55:00+00:00", "[]", 0, ""))
        con.commit()
        con.close()
        conv = relay.agy_find(CID2)
        self.assertEqual((conv["surface"], conv["name"]), ("cli", "Relay test (cli)"))
        r = self.run_cli("sessions", "--json")
        self.assertEqual(sorted(c["id"] for c in json.loads(r.stdout)["antigravity"]), sorted([CID, CID2]))

    def test_database_match_beats_an_empty_folder_elsewhere(self):
        (self.gem / "antigravity" / "brain" / CID2 / ".system_generated" / "logs").mkdir(parents=True)
        (self.gem / "antigravity" / "brain" / CID2 / ".system_generated" / "logs" / "transcript.jsonl").write_text("")
        self.assertEqual(relay.agy_find(CID2)["surface"], "cli")

    def test_cli_conversation_goes_to_message_store(self):
        r = self.run_cli("send", "--to", f"antigravity:{CID2}", "-m", "please check the build",
                         "--from", "codex:t@pc", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        res = json.loads(r.stdout)
        self.assertEqual(res["route"], "antigravity message store")
        msgs = self.gem / "antigravity-cli" / "brain" / CID2 / ".system_generated" / "messages"
        files = list(msgs.glob("*.json"))
        self.assertEqual(len(files), 1)
        rec = json.loads(files[0].read_text())
        self.assertEqual((rec["recipient"], rec["sender"]), (CID2, "system"))
        self.assertIn("please check the build", rec["content"])
        self.assertIn('from="codex:t@pc"', rec["content"])
        self.assertTrue((msgs / "undelivered" / rec["id"]).exists())

    def test_app_conversation_uses_agentapi_through_the_hub(self):
        log = self.fake_hub()
        r = self.run_cli("send", "--to", f"antigravity:{CID}", "-m", "hello app", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["route"], "agentapi send-message")
        args = log.read_text().splitlines()
        self.assertEqual(args[:5], ["127.0.0.1:1234", "token-set", "send-message", "--title=agent-relay", CID])
        self.assertIn("hello app", "\n".join(args[5:]))
        self.assertNotIn("secret-token", r.stdout + r.stderr)

    def test_hub_not_found_falls_back_to_store(self):
        self.fake_hub(response='{"response": {}, "error": "trajectory not found"}')
        r = self.run_cli("send", "--to", f"antigravity:{CID}", "-m", "x", "--json")
        self.assertEqual(json.loads(r.stdout)["route"], "antigravity message store")

    def test_dead_hub_is_ignored(self):
        self.fake_hub()
        hub = self.gem / "antigravity" / "daemon" / "hub_1234.json"
        hub.write_text(json.dumps({"pid": 2 ** 22 + 12345, "httpPort": 1, "csrfToken": "t"}))
        self.assertIsNone(relay.agy_hub("antigravity"))

    def cached_env(self, cid=CID2, addr="127.0.0.1:4321", token="fake-csrf-0000"):
        return dict(os.environ, ANTIGRAVITY_CONVERSATION_ID=cid, ANTIGRAVITY_LS_ADDRESS=addr,
                    ANTIGRAVITY_CSRF_TOKEN=token)

    def fake_agentapi(self, product="antigravity-cli", response='{"response": {"sendMessage": {}}}', code=0):
        exe = self.gem / product / "bin" / "agentapi"
        exe.parent.mkdir(parents=True, exist_ok=True)
        log = Path(self.tmp.name) / "agentapi.log"
        exe.write_text("#!/bin/sh\n"
                       f"printf '%s\\n' \"$ANTIGRAVITY_LS_ADDRESS\" \"$ANTIGRAVITY_CSRF_TOKEN\" \"$@\" > {log}\n"
                       f"echo '{response}'\nexit {code}\n")
        exe.chmod(0o755)
        return log

    def test_agent_tool_command_caches_language_server(self):
        r = subprocess.run([sys.executable, str(SCRIPT), "whoami"], capture_output=True, text=True,
                           env=self.cached_env(), timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("fake-csrf-0000", r.stdout + r.stderr)
        path = Path(self.tmp.name) / "sessions" / "antigravity-ls" / f"{CID2}.json"
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual(path.parent.stat().st_mode & 0o777, 0o700)
        rec = json.loads(path.read_text())
        self.assertEqual((rec["conversation"], rec["address"], rec["token"]),
                         (CID2, "127.0.0.1:4321", "fake-csrf-0000"))
        self.assertEqual(relay.read_registry(), [])          # not mistaken for a session record
        r = self.run_cli("doctor")
        self.assertIn("1 cached CLI/IDE language servers", r.stdout)
        self.assertNotIn("fake-csrf-0000", r.stdout + r.stderr)

    def test_non_loopback_address_is_not_cached(self):
        self.assertIsNone(relay.save_ls_cache(self.cached_env(addr="10.0.0.5:4321")))
        self.assertIsNone(relay.save_ls_cache(self.cached_env(cid="not-a-uuid")))
        self.assertFalse((Path(self.tmp.name) / "sessions" / "antigravity-ls").exists())

    def test_cached_server_wakes_cli_conversation(self):
        relay.save_ls_cache(self.cached_env())
        log = self.fake_agentapi()
        r = self.run_cli("send", "--to", f"antigravity:{CID2}", "-m", "wake up", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["route"], "agentapi send-message (cached server)")
        args = log.read_text().splitlines()
        self.assertEqual(args[:5], ["127.0.0.1:4321", "fake-csrf-0000", "send-message", "--title=agent-relay", CID2])
        self.assertNotIn("fake-csrf-0000", r.stdout + r.stderr)
        self.assertFalse((self.gem / "antigravity-cli" / "brain" / CID2 / ".system_generated" / "messages").exists())

    def test_stale_cached_server_falls_back_and_is_dropped(self):
        path = relay.save_ls_cache(self.cached_env())
        self.fake_agentapi(response='{"error": "unauthorized for token fake-csrf-0000"}', code=1)
        r = self.run_cli("send", "--to", f"antigravity:{CID2}", "-m", "hello", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        res = json.loads(r.stdout)
        self.assertEqual(res["route"], "antigravity message store")
        self.assertIn("dropped the stale entry", res["note"])
        self.assertNotIn("fake-csrf-0000", r.stdout + r.stderr)
        self.assertFalse(path.exists())
        msgs = self.gem / "antigravity-cli" / "brain" / CID2 / ".system_generated" / "messages"
        self.assertEqual(len(list(msgs.glob("*.json"))), 1)

    def test_gc_drops_stale_server_entries(self):
        import socket
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        live = relay.save_ls_cache(self.cached_env(cid=CID, addr=f"127.0.0.1:{srv.getsockname()[1]}"))
        gone = socket.socket()
        gone.bind(("127.0.0.1", 0))
        gone_port = gone.getsockname()[1]
        gone.close()
        dead = relay.save_ls_cache(self.cached_env(addr=f"127.0.0.1:{gone_port}"))
        try:
            r = self.run_cli("gc")
        finally:
            srv.close()
        self.assertIn("1 stale Antigravity server entries", r.stdout)
        self.assertTrue(live.exists())
        self.assertFalse(dead.exists())

    def test_read_skips_empty_planner_steps(self):
        r = self.run_cli("read", f"antigravity:{CID2}", "--last", "3", "--json")
        self.assertEqual([m["text"] for m in json.loads(r.stdout)["messages"]], ["READY"])

    def test_sessions_lists_antigravity(self):
        r = self.run_cli("sessions", "--json")
        got = {(c["id"], c["surface"]) for c in json.loads(r.stdout)["antigravity"]}
        self.assertEqual(got, {(CID, "app"), (CID2, "cli")})

    def test_session_start_hook_injects_address(self):
        r = self.hook("session-start", {"conversationId": CID2, "cwd": "/w/proj", "hookEventName": "SessionStart",
                                        "transcriptPath": "/x/antigravity-cli/brain/t.jsonl"}, agent="antigravity")
        self.assertEqual(r.returncode, 0, r.stderr)
        step = json.loads(r.stdout)["injectSteps"][0]["systemMessage"]["systemMessage"]
        self.assertIn(f"antigravity:{CID2}@here", step)
        self.assertEqual(relay.read_registry("antigravity")[0]["surface"], "cli")
        self.assertEqual(json.loads(self.hook("stop", {"conversationId": CID2}, agent="antigravity").stdout), {})


class Remote(RelayTestCase):
    def test_remote_send_streams_script_and_b64_body(self):
        calls = {}

        def fake_run(cmd, input=None, capture_output=None, timeout=None):
            calls["cmd"], calls["input"] = cmd, input
            return subprocess.CompletedProcess(cmd, 0, b'{"ok": true}\n', b"")

        orig = relay.subprocess.run
        relay.subprocess.run = fake_run
        try:
            with redirect_stdout(io.StringIO()):
                rc = relay.main(["send", "--to", "codex:billing@laptop", "-m", "it's \"quoted\"; $(no)",
                                 "--from", "claude:me@here"])
        finally:
            relay.subprocess.run = orig
        self.assertEqual(rc, 0)
        self.assertIn("laptop", calls["cmd"])
        self.assertIn("BatchMode=yes", calls["cmd"])
        self.assertEqual(calls["input"], SCRIPT.read_bytes())
        remote = calls["cmd"][-1]
        self.assertTrue(remote.startswith("env AGENT_RELAY_SELF=laptop python3 - send --to codex:billing --message-b64 "))
        self.assertNotIn("$(no)", remote)


FAKE_OPENCODE = r"""#!/usr/bin/env python3
import json, os, sys
log = os.path.join(os.environ["AGENT_RELAY_HOME"], "opencode-calls.jsonl")
args = sys.argv[1:]
op = args[1]
params = dict(args[i + 1].split("=", 1) for i, a in enumerate(args) if a == "--param")
data = json.loads(args[args.index("--data") + 1]) if "--data" in args else None
with open(log, "a") as fh:
    fh.write(json.dumps({"op": op, "params": params, "data": data}) + "\n")
now = int(os.environ["FAKE_NOW_MS"])
sess = {"id": "ses_abc123def456", "title": "billing", "time": {"updated": now},
        "location": {"directory": "/work/billing"}}
old = {"id": "ses_old000000000", "title": "old", "time": {"updated": now - 10 * 86400000},
       "location": {"directory": "/work/old"}}
later = int(__import__("time").time() * 1000) + 5   # the reply always lands after the send
replies = {"session.active": {"data": {}},
           "session.list": {"data": [sess, old]},
           "session.get": {"data": sess},
           "session.prompt": {"data": {"id": "inbox_1"}},
           "session.message.list": {"data": [
               {"type": "idle", "time": {"created": later + 1}},
               {"type": "assistant", "time": {"created": later, "completed": later},
                "content": [{"type": "reasoning", "text": "hmm"}, {"type": "text", "text": "PONG"}]}]}}
print(json.dumps(replies[op]))
"""


class OpenCode(RelayTestCase):
    def setUp(self):
        super().setUp()
        fake = Path(self.tmp.name) / "opencode"
        fake.write_text(FAKE_OPENCODE)
        fake.chmod(0o755)
        os.environ["AGENT_RELAY_OPENCODE_BIN"] = str(fake)
        os.environ["FAKE_NOW_MS"] = str(relay.now_ms())

    def calls(self):
        return [json.loads(l) for l in (Path(self.tmp.name) / "opencode-calls.jsonl").read_text().splitlines()]

    def test_address(self):
        self.assertEqual(relay.parse_addr("opencode:ses_abc@laptop"), ("opencode", "ses_abc", "laptop"))
        self.assertEqual(relay.parse_addr("pi:abc"), ("pi", "abc", None))

    def test_list_hides_old_sessions_and_find_by_title(self):
        self.assertEqual([r["id"] for r in relay.oc_list()], ["ses_abc123def456"])
        self.assertEqual(len(relay.oc_list(all_rows=True)), 2)
        self.assertEqual(relay.oc_find("billing")["cwd"], "/work/billing")

    def test_send_queues_prompt_and_reads_reply(self):
        r = self.run_cli("send", "--to", "opencode:ses_abc123def456", "--from", "claude:me@here",
                         "-m", "ping", "--wait-reply", "5", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        out = json.loads(r.stdout)
        self.assertEqual(out["route"], "opencode api session.prompt")
        self.assertEqual(out["reply"]["messages"][-1]["text"], "PONG")
        prompt = next(c for c in self.calls() if c["op"] == "session.prompt")
        self.assertEqual(prompt["params"], {"sessionID": "ses_abc123def456"})
        self.assertEqual(prompt["data"]["delivery"], "queue")
        self.assertIn("<agent-relay-message", prompt["data"]["text"])
        self.assertIn("claude:me@here", prompt["data"]["text"])

    def test_plugin_hook_returns_address(self):
        r = self.hook("session-start", {"session_id": "ses_abc123def456", "cwd": "/work", "pid": os.getpid()},
                      agent="opencode")
        out = json.loads(r.stdout)
        self.assertEqual(out["address"], "opencode:ses_abc123def456@here")
        self.assertIn("--from opencode:ses_abc123def456@here", out["context"])


class Pi(RelayTestCase):
    def test_extension_hooks_register_deliver_and_track_state(self):
        base = {"session_id": SID, "cwd": "/work", "pid": os.getpid(), "transcript_path": "/s.jsonl"}
        out = json.loads(self.hook("session-start", base, agent="pi").stdout)
        self.assertEqual(out["address"], f"pi:{SID}@here")
        self.assertNotIn("<agent-relay-message", out["context"])
        self.assertEqual(json.loads(self.hook("inbox", base, agent="pi").stdout).get("context"), None)
        r = self.run_cli("send", "--to", f"pi:{SID[:8]}", "--from", "codex:t@laptop", "-m", "hello pi")
        self.assertEqual(r.returncode, 0, r.stderr)
        out = json.loads(self.hook("inbox", base, agent="pi").stdout)
        self.assertIn("hello pi", out["context"])
        self.assertEqual(relay.read_registry("pi")[0]["state"], "busy")
        self.assertEqual(json.loads(self.hook("inbox", base, agent="pi").stdout).get("context"), None)
        self.hook("stop", base, agent="pi")
        self.assertEqual(relay.read_registry("pi")[0]["state"], "idle")

    def test_detect_self_and_session_file_reply(self):
        os.environ["AGENT_RELAY_PI_SESSION"] = SID
        self.assertEqual(relay.detect_self(), f"pi:{SID}@here")
        f = Path(self.tmp.name) / "pi.jsonl"
        f.write_text("\n".join(json.dumps(e) for e in [
            {"type": "session", "version": 3, "id": SID, "cwd": "/work"},
            {"type": "message", "timestamp": "t1", "message": {"role": "user", "content": "hi", "timestamp": 1}},
            {"type": "message", "timestamp": "t2", "message": {
                "role": "assistant", "timestamp": 2,
                "content": [{"type": "thinking", "thinking": "x"}, {"type": "text", "text": "PI-OK"}]}}]))
        msgs = relay.pi_messages(str(f))
        self.assertEqual([m["text"] for m in msgs], ["PI-OK"])
        self.assertEqual(relay.pi_messages(str(f), since_ms=3), [])


class RemoteHint(RelayTestCase):
    def test_streamed_script_points_replies_at_an_installed_copy(self):
        installed = Path(self.tmp.name) / "plugin" / "scripts" / "relay.py"
        installed.parent.mkdir(parents=True)
        installed.write_text("# installed copy\n")
        relay.update_registry("claude", SID, relay=f"python3 {installed}")
        saved = relay.__file__
        relay.__file__ = "<stdin>"
        try:
            os.environ["AGENT_RELAY_RELAY_BIN"] = str(Path(self.tmp.name) / "no-shim")
            self.assertEqual(relay.relay_cmd_hint(), f"python3 {installed}")
        finally:
            relay.__file__ = saved
            os.environ.pop("AGENT_RELAY_RELAY_BIN", None)

    def test_install_cli_writes_a_shim(self):
        d = Path(self.tmp.name) / "bin"
        r = self.run_cli("install-cli", "--dir", str(d))
        self.assertEqual(r.returncode, 0, r.stderr)
        shim = d / "relay"
        self.assertIn(str(SCRIPT), shim.read_text())
        self.assertTrue(os.access(shim, os.X_OK))
        (d / "other").write_text("x")
        shim.write_text("#!/bin/sh\necho something else\n")
        self.assertNotEqual(self.run_cli("install-cli", "--dir", str(d)).returncode, 0)

if __name__ == "__main__":
    unittest.main()
