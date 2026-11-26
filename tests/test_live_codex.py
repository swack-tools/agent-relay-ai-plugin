"""Live test against a running Codex app-server daemon. Skips when none is available.

Opt in with AGENT_RELAY_LIVE_CODEX=1 and only against a throwaway daemon (CI, a
sandbox): the test creates a named thread in that daemon and queues a relay
message into it, which starts a real turn."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from pathlib import Path

sys.dont_write_bytecode = True  # keep __pycache__ out of the plugin package
SCRIPT = Path(__file__).resolve().parents[1] / "plugins" / "agent-relay" / "scripts" / "relay.py"
spec = importlib.util.spec_from_file_location("relay", SCRIPT)
relay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(relay)


def daemon_available():
    return (os.environ.get("AGENT_RELAY_LIVE_CODEX") == "1"
            and relay.which("codex") and relay.codex_socket().exists())


@unittest.skipUnless(daemon_available(), "set AGENT_RELAY_LIVE_CODEX=1 with a throwaway Codex daemon")
class LiveCodex(unittest.TestCase):
    def test_queue_round_trip(self):
        cs = relay.CodexAppServer()
        try:
            tid = cs.call("thread/start", {"cwd": tempfile.gettempdir()})["thread"]["id"]
            name = f"relay-live-{uuid.uuid4().hex[:6]}"
            cs.call("thread/name/set", {"threadId": tid, "name": name})
            self.assertIn(tid, relay.codex_loaded(cs))
        finally:
            cs.close()
        with tempfile.TemporaryDirectory() as home:
            env = dict(os.environ, AGENT_RELAY_HOME=home)
            r = subprocess.run([sys.executable, str(SCRIPT), "send", "--to", f"codex:{name}",
                                "-m", "live relay check", "--from", "claude:test@here", "--json"],
                               capture_output=True, text=True, env=env, timeout=90)
            self.assertEqual(r.returncode, 0, r.stderr)
            result = json.loads(r.stdout)
            self.assertEqual(result["thread"], tid)
            self.assertEqual(result["route"], "codex queue")
            r = subprocess.run([sys.executable, str(SCRIPT), "sessions", "--json"],
                               capture_output=True, text=True, env=env, timeout=60)
            self.assertIn(tid, r.stdout)
        texts = []
        for _ in range(20):                      # the queued turn starts asynchronously
            cs = relay.CodexAppServer()
            try:
                items = cs.call("thread/items/list", {"threadId": tid})["data"]
            finally:
                cs.close()
            texts = [c.get("text", "") for e in items if e["item"]["type"] == "userMessage"
                     for c in e["item"]["content"]]
            if texts:
                break
            time.sleep(0.5)
        cs = relay.CodexAppServer()
        try:
            cs.call("thread/archive", {"threadId": tid})
        finally:
            cs.close()
        self.assertTrue(any("live relay check" in t and "agent-relay-message" in t for t in texts), texts)


if __name__ == "__main__":
    unittest.main()
