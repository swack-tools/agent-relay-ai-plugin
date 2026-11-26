#!/usr/bin/env python3
"""agent-relay: send messages between live Claude Code, Codex and Antigravity sessions.

Standard library only (Python 3.9+), so the same file can run on a remote
computer over `ssh HOST python3 -` without being installed there.

Routes
  codex:<thread>        `codex queue` into the shared Codex app-server daemon
                        (CLI, Codex app, ChatGPT desktop). An idle thread runs it
                        now; a busy one gets it when its current turn ends.
  claude:<session>      A mailbox under ~/.agent-relay/inbox/<session>/. The
                        plugin's Claude Code hooks deliver it: a background
                        watcher wakes an idle session, and UserPromptSubmit
                        delivers anything still pending.
  antigravity:<conv>    Desktop app conversations: `agentapi send-message`
                        through the app's server, which wakes an idle one. CLI
                        and IDE conversations: the same, through their own
                        language server when it is cached, else Antigravity's
                        message store, read at the conversation's next turn.
  <addr>@<host>         Run the same operation on <host> over ssh (BatchMode).
"""
from __future__ import annotations

import argparse
import base64
import datetime as _dt
import json
import os
import re
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import time
import uuid
from pathlib import Path

VERSION = "1.1.0"
AGENTS = ("codex", "claude", "antigravity", "opencode", "pi")
MAILBOX_AGENTS = ("claude", "pi")   # delivered by the agent's own hooks or extension
INLINE_LIMIT = 6000          # characters; longer bodies travel as a file pointer
MAX_BODY = 1_000_000         # characters
WATCH_SECONDS = int(os.environ.get("AGENT_RELAY_WATCH_SECONDS", "3300"))
POLL_SECONDS = 2.0
EXTRA_PATH = ["/opt/homebrew/bin", "/usr/local/bin", "~/.local/bin", "~/.npm-global/bin", "~/.gemini/bin",
              "~/.bun/bin", "~/.volta/bin", "~/.cargo/bin", "~/bin", "/usr/bin", "/bin"]


class RelayError(Exception):
    pass


# --------------------------------------------------------------------------- paths

def relay_home() -> Path:
    return Path(os.environ.get("AGENT_RELAY_HOME", "~/.agent-relay")).expanduser()


def ensure_dir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path, 0o700)
    except OSError:
        pass
    return path


def config() -> dict:
    try:
        return json.loads((relay_home() / "config.json").read_text())
    except (OSError, ValueError):
        return {}


def self_host() -> str:
    """The name other computers use to ssh back here."""
    return (os.environ.get("AGENT_RELAY_SELF") or config().get("self")
            or socket.gethostname().split(".")[0])


def now_ms() -> int:
    return int(time.time() * 1000)


def iso(ms: int | None = None) -> str:
    t = (ms / 1000) if ms else time.time()
    return _dt.datetime.fromtimestamp(t, _dt.timezone.utc).astimezone().isoformat(timespec="seconds")


def which(name: str) -> str | None:
    env = os.environ.get(f"AGENT_RELAY_{name.upper()}_BIN")
    if env:
        return env if Path(env).expanduser().exists() else None
    found = shutil.which(name)
    if found:
        return found
    for d in extra_path():
        p = Path(d) / name
        if p.is_file() and os.access(p, os.X_OK):
            return str(p)
    return None


def extra_path() -> list[str]:
    """Common install folders, plus nvm's (newest Node first): ssh sessions often
    start with a PATH that has none of them."""
    import glob
    dirs = [str(Path(d).expanduser()) for d in EXTRA_PATH]
    nvm = sorted(glob.glob(str(Path("~/.nvm/versions/node/*/bin").expanduser())),
                 key=lambda p: [int(x) if x.isdigit() else 0 for x in re.findall(r"\d+", p)], reverse=True)
    return dirs + nvm


def tool_env(binary: str | None = None) -> dict:
    """PATH for child processes: npm-installed CLIs need node beside them, and ssh
    sessions often start with a minimal PATH."""
    env = dict(os.environ)
    dirs = ([str(Path(binary).parent)] if binary else []) + extra_path()
    env["PATH"] = os.pathsep.join(dirs + [env.get("PATH", "")])
    return env


def atomic_write(path: Path, text: str) -> None:
    ensure_dir(path.parent)
    tmp = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as fh:
        fh.write(text)
    os.replace(tmp, path)


def pid_alive(pid) -> bool:
    try:
        pid = int(pid)
        if pid <= 1:
            return False
        os.kill(pid, 0)
        return True
    except (ValueError, TypeError, ProcessLookupError):
        return False
    except PermissionError:
        return True


# --------------------------------------------------------------------------- addresses

ADDR_RE = re.compile(r"^(codex|claude|antigravity|opencode|pi):([^@]*[^@\s])(?:@([A-Za-z0-9_.\-]+))?$")


def parse_addr(text: str, host: str | None = None):
    m = ADDR_RE.match(text.strip())
    if not m:
        raise RelayError(f"bad address {text!r}; use codex:<thread>, claude:<session>, "
                         "antigravity:<conversation>, opencode:<session> or pi:<session>, optionally @host")
    agent, ref, at_host = m.groups()
    h = host or at_host
    if h in (None, "", "local", "localhost", self_host()):
        h = None
    return agent, ref, h


def fmt_addr(agent: str, ref: str, host: str | None = None) -> str:
    return f"{agent}:{ref}@{host or self_host()}"


def detect_self() -> str | None:
    """This process's own relay address, when it runs inside an agent session."""
    if os.environ.get("AGENT_RELAY_FROM"):
        return os.environ["AGENT_RELAY_FROM"]
    if os.environ.get("CODEX_THREAD_ID"):
        return fmt_addr("codex", os.environ["CODEX_THREAD_ID"])
    if os.environ.get("CLAUDE_CODE_SESSION_ID"):
        return fmt_addr("claude", os.environ["CLAUDE_CODE_SESSION_ID"])
    if os.environ.get("ANTIGRAVITY_CONVERSATION_ID"):
        return fmt_addr("antigravity", os.environ["ANTIGRAVITY_CONVERSATION_ID"])
    if os.environ.get("AGENT_RELAY_PI_SESSION"):
        return fmt_addr("pi", os.environ["AGENT_RELAY_PI_SESSION"])
    return None


# --------------------------------------------------------------------------- registry

def registry_dir() -> Path:
    return relay_home() / "sessions"


def registry_path(agent: str, sid: str) -> Path:
    return registry_dir() / f"{agent}-{sid}.json"


def read_registry(agent: str | None = None) -> list[dict]:
    out = []
    d = registry_dir()
    if not d.is_dir():
        return out
    for p in sorted(d.glob("*.json")):
        try:
            rec = json.loads(p.read_text())
        except (OSError, ValueError):
            continue
        if agent and rec.get("agent") != agent:
            continue
        rec["alive"] = pid_alive(rec.get("pid")) if rec.get("pid") else None
        out.append(rec)
    return out


def update_registry(agent: str, sid: str, **fields) -> dict:
    p = registry_path(agent, sid)
    try:
        rec = json.loads(p.read_text())
    except (OSError, ValueError):
        rec = {"agent": agent, "id": sid, "registered_at": iso()}
    rec.update({k: v for k, v in fields.items() if v is not None})
    rec["host"] = self_host()
    rec["updated_at"] = iso()
    rec.pop("alive", None)
    atomic_write(p, json.dumps(rec, indent=2))
    return rec


def agent_pid(agent: str, data: dict | None = None) -> int:
    """Best-effort pid of the agent process that spawned this hook."""
    if data and str(data.get("pid", "")).isdigit():
        return int(data["pid"])          # the pi extension and opencode plugin pass their own pid
    env = os.environ.get("CLAUDE_PID") if agent == "claude" else None
    if env and env.isdigit():
        return int(env)
    pid = os.getppid()
    for _ in range(8):
        try:
            out = subprocess.run(["ps", "-o", "ppid=", "-o", "comm=", "-p", str(pid)],
                                 capture_output=True, text=True, timeout=3).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            break
        if not out:
            break
        ppid, _, comm = out.partition(" ")
        if agent in Path(comm.strip()).name.lower():
            return pid
        if not ppid.strip().isdigit() or int(ppid) <= 1:
            break
        pid = int(ppid)
    return os.getppid()


def resolve_claude(ref: str) -> dict:
    return resolve_mailbox("claude", ref)


def resolve_mailbox(agent: str, ref: str) -> dict:
    """A Claude Code or pi session, from the records their hooks keep."""
    recs = read_registry(agent)
    exact = [r for r in recs if r["id"] == ref]
    if exact:
        return exact[0]
    live = [r for r in recs if r.get("alive") is not False]
    cands = [r for r in live if len(ref) >= 6 and r["id"].startswith(ref)]
    if not cands:
        cands = [r for r in live if ref in (r.get("label"), Path(r.get("cwd", "/")).name)]
    if len(cands) == 1:
        return cands[0]
    if len(cands) > 1:
        names = ", ".join(f"{r['id']} ({r.get('label') or r.get('cwd')})" for r in cands)
        raise RelayError(f"{agent}:{ref} matches several sessions: {names}. Use the full session id.")
    if re.fullmatch(r"[0-9a-fA-F-]{36}", ref):
        return {"agent": agent, "id": ref, "unregistered": True}
    kind = {"claude": "Claude", "pi": "pi"}.get(agent, agent)
    raise RelayError(f"no live {kind} session matches {ref!r}. Run `relay sessions` to list them.")


# --------------------------------------------------------------------------- Codex app-server

def codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME", "~/.codex")).expanduser()


def codex_socket() -> Path:
    return Path(os.environ.get("AGENT_RELAY_CODEX_SOCKET",
                               codex_home() / "app-server-control" / "app-server-control.sock"))


class CodexAppServer:
    """Minimal JSON-RPC client for the Codex app-server daemon's control socket.

    The daemon speaks WebSocket over a Unix socket (the same endpoint
    `codex --remote unix://...` uses)."""

    def __init__(self, path: Path | None = None, timeout: float = 20):
        self.path = Path(path or codex_socket())
        if not self.path.exists():
            raise RelayError(f"Codex app-server socket not found at {self.path}. Is a Codex app or "
                             "CLI session running? (`codex app-server daemon version` shows the daemon.)")
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(timeout)
        try:
            self.sock.connect(str(self.path))
        except OSError as exc:
            self.sock.close()
            raise RelayError(f"cannot connect to Codex app-server at {self.path}: {exc}") from exc
        self._buf = b""
        self._id = 0
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
                           f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                           f"Sec-WebSocket-Version: 13\r\n\r\n").encode())
        head = self._read_until(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise RelayError("Codex app-server refused the WebSocket upgrade")
        self.call("initialize", {"clientInfo": {"name": "agent-relay", "version": VERSION}})
        self._send({"jsonrpc": "2.0", "method": "initialized"})

    def _read_until(self, marker: bytes) -> bytes:
        while marker not in self._buf:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RelayError("Codex app-server closed the connection")
            self._buf += chunk
        head, _, self._buf = self._buf.partition(marker)
        return head

    def _read(self, n: int) -> bytes:
        while len(self._buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RelayError("Codex app-server closed the connection")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _frame(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            head = struct.pack("!BB", 0x80 | opcode, 0x80 | n)
        elif n < 65536:
            head = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, n)
        else:
            head = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(head + mask + masked)

    def _send(self, obj: dict) -> None:
        self._frame(0x1, json.dumps(obj).encode())

    def _recv(self) -> dict:
        parts = []
        while True:
            b1, b2 = self._read(2)
            opcode, n = b1 & 0x0F, b2 & 0x7F
            if n == 126:
                n = struct.unpack("!H", self._read(2))[0]
            elif n == 127:
                n = struct.unpack("!Q", self._read(8))[0]
            mask = self._read(4) if b2 & 0x80 else None
            data = self._read(n)
            if mask:
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
            if opcode == 0x9:
                self._frame(0xA, data)
                continue
            if opcode == 0x8:
                raise RelayError("Codex app-server closed the connection")
            if opcode in (0x1, 0x2, 0x0):
                parts.append(data)
                if b1 & 0x80:
                    return json.loads(b"".join(parts).decode())

    def call(self, method: str, params: dict | None = None):
        self._id += 1
        rid = self._id
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}})
        while True:
            msg = self._recv()
            if msg.get("id") != rid:
                continue                       # notifications and server requests
            if "error" in msg:
                err = msg["error"]
                raise RelayError(f"Codex {method} failed: {err.get('message', err)}")
            return msg.get("result")

    def close(self) -> None:
        try:
            self._frame(0x8, b"")
            self.sock.close()
        except OSError:
            pass


def codex_thread_summary(t: dict) -> dict:
    status = t.get("status") or {}
    return {"agent": "codex", "id": t.get("id"), "name": t.get("name"), "cwd": t.get("cwd"),
            "status": status.get("type"), "preview": (t.get("preview") or "")[:100],
            "updated": iso(t["updatedAt"] * 1000) if t.get("updatedAt") else None,
            "cli_version": t.get("cliVersion")}


def codex_find_thread(cs: CodexAppServer, ref: str) -> dict:
    try:
        return cs.call("thread/read", {"threadId": ref, "includeTurns": False})["thread"]
    except RelayError:
        pass
    for tid in codex_loaded(cs):
        t = cs.call("thread/read", {"threadId": tid, "includeTurns": False})["thread"]
        if t.get("name") == ref:
            return t
    res = cs.call("thread/list", {"searchTerm": ref, "limit": 20})
    for t in res.get("data", []):
        if t.get("name") == ref or t.get("id") == ref:
            return t
    raise RelayError(f"no Codex thread with id or exact name {ref!r}")


def codex_loaded(cs: CodexAppServer) -> list[str]:
    ids, cursor = [], None
    for _ in range(20):
        res = cs.call("thread/loaded/list", {"cursor": cursor} if cursor else {})
        ids += res.get("data", [])
        cursor = res.get("nextCursor")
        if not cursor:
            break
    return ids


def codex_agent_messages(cs: CodexAppServer, tid: str, since_ms: int = 0, limit: int = 3) -> list[dict]:
    res = cs.call("thread/items/list", {"threadId": tid, "sortDirection": "desc", "limit": 200})
    out = []
    for entry in res.get("data", []):
        item = entry.get("item", {})
        started = entry.get("startedAtMs") or item.get("startedAtMs") or 0
        if since_ms and started and started < since_ms:
            break
        if item.get("type") == "agentMessage" and item.get("text"):
            out.append({"at": iso(started) if started else None, "at_ms": started,
                        "phase": item.get("phase"), "text": item["text"]})
            if len(out) >= limit:
                break
    return list(reversed(out))


# --------------------------------------------------------------------------- Antigravity

# Antigravity keeps one data folder per surface. The 2.0 desktop app runs a
# shared language server ("hub") that writes its port and CSRF token to
# daemon/hub_<port>.json; `agentapi send-message` through it reaches an idle
# conversation right away. The CLI and IDE servers publish no address, so
# messages for their conversations go into Antigravity's own per-conversation
# message store, which the conversation reads at the start of its next turn.
AGY_PRODUCTS = (("antigravity", "app"), ("antigravity-ide", "ide"), ("antigravity-cli", "cli"))
UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


def gemini_home() -> Path:
    return Path(os.environ.get("AGENT_RELAY_GEMINI_HOME", "~/.gemini")).expanduser()


def agy_db_rows(product_dir: str, where: str = "", args: tuple = (), limit: int = 50) -> list[dict]:
    import sqlite3
    db = gemini_home() / product_dir / "conversation_summaries.db"
    if not db.is_file():
        return []
    query = ("select conversation_id, title, status, last_modified_time, workspace_uris, killed, "
             f"parent_conversation_id from conversation_summaries {where} "
             "order by last_modified_time desc limit ?")
    # mode=ro fails on a WAL database whose -shm file does not exist yet; a
    # normal connection (mode=rw never creates the file) only runs this SELECT.
    for mode in ("ro", "rw"):
        try:
            con = sqlite3.connect(f"file:{db}?mode={mode}", uri=True, timeout=5)
        except sqlite3.Error:
            continue
        try:
            con.row_factory = sqlite3.Row
            return [dict(r) for r in con.execute(query, args + (limit,)).fetchall()]
        except sqlite3.Error:
            continue
        finally:
            con.close()
    return []


def agy_presence_open(product_dir: str, cid: str) -> bool:
    """A conversation shown in a window holds a lock on presence/<id>.lock."""
    import fcntl
    lock = gemini_home() / product_dir / "presence" / f"{cid}.lock"
    if not lock.exists():
        return False
    try:
        fd = os.open(lock, os.O_RDONLY)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    except OSError:
        return True
    finally:
        os.close(fd)


def agy_summary(product_dir: str, kind: str, row: dict) -> dict:
    cid = row["conversation_id"]
    status = (row.get("status") or "").replace("CASCADE_RUN_STATUS_", "").lower() or None  # noqa
    try:
        ws = [u.replace("file://", "") for u in json.loads(row.get("workspace_uris") or "[]")]
    except ValueError:
        ws = []
    return {"agent": "antigravity", "id": cid, "surface": kind, "name": row.get("title") or None,
            "status": status, "open": agy_presence_open(product_dir, cid), "cwd": ws[0] if ws else None,
            "updated": row.get("last_modified_time")}


def agy_transcript(product_dir: str, cid: str) -> Path:
    return gemini_home() / product_dir / "brain" / cid / ".system_generated" / "logs" / "transcript.jsonl"


def agy_dedupe(found: list[dict]) -> list[dict]:
    """Surfaces index each other's conversations, so one id can appear in several
    summary databases. Keep the copy whose folder holds the newest transcript."""
    best: dict = {}
    for m in found:
        t = agy_transcript(m["product_dir"], m["id"])
        score = t.stat().st_mtime if t.is_file() else -1.0
        if m["id"] not in best or score > best[m["id"]][0]:
            best[m["id"]] = (score, m)
    return [v[1] for v in best.values()]


def agy_list(all_rows: bool = False) -> list[dict]:
    found = []
    for product_dir, kind in AGY_PRODUCTS:
        for row in agy_db_rows(product_dir, "where killed = 0 and parent_conversation_id = ''",
                               limit=200 if all_rows else 40):
            found.append(agy_summary(product_dir, kind, row) | {"product_dir": product_dir})
    out_rows = []
    for g in sorted(agy_dedupe(found), key=lambda r: str(r.get("updated") or ""), reverse=True):
        if all_rows or g["open"] or g["status"] not in (None, "idle") or len(
                [r for r in out_rows if r["surface"] == g["surface"]]) < 5:
            out_rows.append({k: v for k, v in g.items() if k != "product_dir"})
    return out_rows


def agy_find(ref: str) -> dict:
    """Resolve an id, id prefix (6+ characters) or exact title to one conversation.
    Summary-database matches win; a data folder with a transcript counts only
    when no database knows the conversation yet."""
    matches, folders = [], []
    for product_dir, kind in AGY_PRODUCTS:
        if UUID_RE.match(ref):
            rows = agy_db_rows(product_dir, "where conversation_id = ?", (ref,), 1)
            pattern = ref
        elif len(ref) >= 6 and re.fullmatch(r"[0-9a-f-]+", ref):
            rows = agy_db_rows(product_dir, "where conversation_id like ?", (ref + "%",), 5)
            pattern = ref + "*"
        else:
            rows = agy_db_rows(product_dir, "where title = ? and killed = 0", (ref,), 5)
            pattern = None
        matches += [agy_summary(product_dir, kind, r) | {"product_dir": product_dir} for r in rows]
        if pattern and not rows:
            for b in (gemini_home() / product_dir / "brain").glob(pattern):
                if UUID_RE.match(b.name) and (b / ".system_generated" / "logs" / "transcript.jsonl").is_file():
                    folders.append(agy_summary(product_dir, kind, {"conversation_id": b.name})
                                   | {"product_dir": product_dir})
    matches = agy_dedupe(matches or folders)
    if len(matches) == 1:
        return matches[0]
    if not matches:
        raise RelayError(f"no Antigravity conversation matches {ref!r}. Run `relay sessions` to list them.")
    names = ", ".join(f"{m['id']} ({m['surface']}: {m.get('name')})" for m in matches)
    raise RelayError(f"antigravity:{ref} matches several conversations: {names}. Use the full id.")


def agy_hub(product_dir: str) -> dict | None:
    """The live hub server for a surface, from its daemon/hub_*.json files."""
    best = None
    for f in sorted((gemini_home() / product_dir / "daemon").glob("hub_*.json")):
        try:
            d = json.loads(f.read_text())
        except (OSError, ValueError):
            continue
        if d.get("httpPort") and d.get("csrfToken") and pid_alive(d.get("pid")):
            if best is None or f.stat().st_mtime > best[0]:
                best = (f.stat().st_mtime, d)
    return best[1] if best else None


def agy_agentapi(product_dir: str) -> str | None:
    for d in (product_dir, "antigravity", "antigravity-ide", "antigravity-cli"):
        p = gemini_home() / d / "bin" / "agentapi"
        if p.is_file() and os.access(p, os.X_OK):
            return str(p)
    return None


# The CLI and IDE language servers publish no address, but every tool command
# an Antigravity agent runs gets ANTIGRAVITY_LS_ADDRESS and ANTIGRAVITY_CSRF_TOKEN
# in its environment. When relay.py runs there it caches them (mode 0600, never
# printed) so a later send can wake that idle conversation through agentapi.
LS_ADDR_RE = re.compile(r"^(127\.0\.0\.1|localhost|\[::1\]):(\d{1,5})$")
LS_CACHE_MAX_AGE = 30 * 86400


def ls_cache_dir() -> Path:
    return registry_dir() / "antigravity-ls"


def ls_cache_path(cid: str) -> Path:
    return ls_cache_dir() / f"{cid}.json"


def save_ls_cache(env=None) -> Path | None:
    """Cache this Antigravity agent's language-server address and CSRF token.
    Only loopback addresses are kept. Never raises."""
    env = os.environ if env is None else env
    cid = env.get("ANTIGRAVITY_CONVERSATION_ID", "")
    addr = env.get("ANTIGRAVITY_LS_ADDRESS", "").strip()
    token = env.get("ANTIGRAVITY_CSRF_TOKEN", "")
    if not (UUID_RE.match(cid) and LS_ADDR_RE.match(addr) and token):
        return None
    exe = env.get("ANTIGRAVITY_AGENTAPI_PATH") or env.get("ANTIGRAVITY_AGENTAPI") or None
    if exe and not (os.path.isabs(exe) and os.path.basename(exe) == "agentapi"):
        exe = None
    hint = exe or ""
    surface = "cli" if "antigravity-cli" in hint else "ide" if "antigravity-ide" in hint else None
    path = ls_cache_path(cid)
    try:
        old = json.loads(path.read_text())
        if (old.get("address"), old.get("token"), old.get("agentapi")) == (addr, token, exe) and \
                now_ms() - int(old.get("saved_at_ms") or 0) < 3600_000:
            return path
    except (OSError, ValueError, TypeError):
        pass
    try:
        ensure_dir(registry_dir())
        ensure_dir(ls_cache_dir())
        atomic_write(path, json.dumps({"conversation": cid, "address": addr, "token": token, "agentapi": exe,
                                       "surface": surface, "saved_at": iso(), "saved_at_ms": now_ms()}))
        return path
    except OSError:
        return None


def read_ls_cache() -> list[dict]:
    out_recs = []
    d = ls_cache_dir()
    for p in sorted(d.glob("*.json")) if d.is_dir() else []:
        try:
            rec = json.loads(p.read_text())
        except (OSError, ValueError):
            rec = {}
        rec["_path"] = p
        out_recs.append(rec)
    return out_recs


def ls_entry_stale(rec: dict) -> bool:
    """Unreadable, too old, or nothing listening at the cached address."""
    m = LS_ADDR_RE.match(str(rec.get("address") or ""))
    if not (m and rec.get("token") and UUID_RE.match(str(rec.get("conversation") or ""))):
        return True
    if now_ms() - int(rec.get("saved_at_ms") or 0) > LS_CACHE_MAX_AGE * 1000:
        return True
    host = "::1" if m.group(1) == "[::1]" else m.group(1)
    try:
        socket.create_connection((host, int(m.group(2))), timeout=1).close()
        return False
    except OSError:
        return True


def agentapi_send(exe: str, address: str, token: str, cid: str, text: str, title: str) -> tuple[bool, str]:
    """(ok, error). The token reaches agentapi only through its environment and
    is scrubbed from any error text."""
    env = dict(os.environ, ANTIGRAVITY_LS_ADDRESS=address, ANTIGRAVITY_CSRF_TOKEN=token)
    try:
        proc = subprocess.run([exe, "send-message", f"--title={title}", cid, text],
                              capture_output=True, text=True, env=env, timeout=60)
    except (OSError, subprocess.SubprocessError) as exc:
        return False, str(exc).replace(token, "***")
    try:
        resp = json.loads(proc.stdout or "{}")
    except ValueError:
        resp = {}
    if not isinstance(resp, dict):
        resp = {}
    if proc.returncode == 0 and not resp.get("error"):
        return True, ""
    err = str(resp.get("error") or proc.stderr.strip() or proc.stdout.strip() or f"exit {proc.returncode}")
    return False, err.replace(token, "***")[:300]


def agy_send_cached(conv: dict, text: str, title: str) -> tuple[dict | None, str | None]:
    """Try the conversation's cached language server. Returns (result, note):
    a result when it was sent, else a note on why the cache was not used."""
    path = ls_cache_path(conv["id"])
    if not path.is_file():
        return None, None
    try:
        rec = json.loads(path.read_text())
    except (OSError, ValueError):
        path.unlink(missing_ok=True)
        return None, "dropped an unreadable cached Antigravity server entry"
    exe = rec.get("agentapi")
    if not (exe and os.path.isfile(exe) and os.access(exe, os.X_OK)):
        exe = agy_agentapi(conv["product_dir"])
    if not exe or not LS_ADDR_RE.match(str(rec.get("address") or "")) or not rec.get("token"):
        if not exe:
            return None, "a cached Antigravity server exists but no agentapi binary was found"
        path.unlink(missing_ok=True)
        return None, "dropped an invalid cached Antigravity server entry"
    ok, err = agentapi_send(exe, rec["address"], rec["token"], conv["id"], text, title)
    if ok:
        return ({"route": "agentapi send-message (cached server)",
                 "delivery": (f"sent through the {conv['surface']} conversation's own language server "
                              "(cached from its last relay command); an idle conversation starts a turn now")},
                None)
    path.unlink(missing_ok=True)
    return None, f"the cached Antigravity server did not accept the message ({err}); dropped the stale entry"


def agy_send(conv: dict, text: str, title: str = "agent-relay") -> dict:
    product_dir = conv["product_dir"]
    hub = agy_hub(product_dir)
    exe = agy_agentapi(product_dir)
    if hub and exe:
        ok, err = agentapi_send(exe, f"127.0.0.1:{hub['httpPort']}", hub["csrfToken"], conv["id"], text, title)
        if ok:
            return {"route": "agentapi send-message",
                    "delivery": "sent through the Antigravity app; an idle conversation starts a turn with it now"}
        if "not found" not in err:
            raise RelayError(f"agentapi send-message failed: {err}")
    sent, note = agy_send_cached(conv, text, title)
    if sent:
        return sent
    path = agy_store_message(product_dir, conv["id"], text, title)
    res = {"route": "antigravity message store", "store": str(path),
           "delivery": ("queued in Antigravity's message store; the conversation reads it at the start "
                        "of its next turn (no live language server is known for this "
                        f"{conv['surface']} conversation; it gets one once it runs a relay command)")}
    if note:
        res["note"] = note
    return res


def agy_store_message(product_dir: str, cid: str, text: str, title: str) -> Path:
    if not UUID_RE.match(cid):
        raise RelayError(f"invalid Antigravity conversation id {cid!r}")
    d = gemini_home() / product_dir / "brain" / cid / ".system_generated" / "messages"
    if not (gemini_home() / product_dir / "brain" / cid).is_dir():
        raise RelayError(f"Antigravity conversation {cid} has no data folder under {product_dir}")
    mid = str(uuid.uuid4())
    record = {"id": mid, "recipient": cid, "sender": "system", "priority": "MESSAGE_PRIORITY_HIGH",
              "timestamp": _dt.datetime.now(_dt.timezone.utc).isoformat().replace("+00:00", "Z"),
              "renderDetails": {"messageTitle": title}, "content": text}
    path = d / f"{mid}.json"
    tmp = d / f".{mid}.tmp"
    d.mkdir(parents=True, exist_ok=True)
    (d / "undelivered").mkdir(exist_ok=True)
    tmp.write_text(json.dumps(record))
    os.replace(tmp, path)
    (d / "undelivered" / mid).touch()
    return path


def agy_messages(conv: dict, since_ms: int = 0, limit: int = 3) -> list[dict]:
    t = agy_transcript(conv["product_dir"], conv["id"])
    out_msgs = []
    try:
        lines = t.read_text(errors="replace").splitlines()
    except OSError:
        return out_msgs
    for line in reversed(lines):
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if ev.get("source") != "MODEL" or ev.get("type") != "PLANNER_RESPONSE":
            continue
        text = str(ev.get("content") or "").strip()
        ts = ev.get("created_at")
        at_ms = 0
        if ts:
            try:
                at_ms = int(_dt.datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp() * 1000)
            except ValueError:
                pass
        if since_ms and at_ms and at_ms < since_ms - 1000:
            break
        if text and text != "None":
            out_msgs.append({"at": ts, "at_ms": at_ms, "text": text})
            if len(out_msgs) >= limit:
                break
    return list(reversed(out_msgs))


def agy_status(conv: dict) -> str | None:
    rows = agy_db_rows(conv["product_dir"], "where conversation_id = ?", (conv["id"],), 1)
    return agy_summary(conv["product_dir"], conv["surface"], rows[0])["status"] if rows else conv.get("status")


def wait_agy_reply(conv: dict, since_ms: int, timeout: float) -> dict:
    deadline = time.time() + timeout
    while True:
        msgs = agy_messages(conv, since_ms)
        status = agy_status(conv)
        if msgs and status in (None, "idle"):
            return {"status": status, "messages": msgs, "complete": True}
        if time.time() >= deadline:
            return {"status": status, "messages": msgs, "complete": False}
        time.sleep(3)


# --------------------------------------------------------------------------- opencode

OPENCODE_RECENT_MS = 24 * 3600 * 1000     # `relay sessions` lists opencode sessions active in the last day


def opencode_api(operation: str, params: dict | None = None, data: dict | None = None,
                 timeout: float = 30):
    """Call the opencode background service through `opencode api` (it handles the
    service's address and password)."""
    exe = which("opencode")
    if not exe:
        raise RelayError("the opencode command is not on PATH on this computer")
    cmd = [exe, "api", operation]
    for k, v in (params or {}).items():
        cmd += ["--param", f"{k}={v}"]
    if data is not None:
        cmd += ["--data", json.dumps(data)]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                              env=tool_env(exe), cwd=str(Path.home()))
    except subprocess.TimeoutExpired:
        raise RelayError(f"opencode api {operation} timed out; is the opencode service running?")
    except OSError as exc:
        raise RelayError(f"could not run opencode: {exc}")
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout).strip().splitlines()
        raise RelayError(f"opencode api {operation} failed: {detail[-1] if detail else proc.returncode}")
    text = proc.stdout.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except ValueError:
        raise RelayError(f"opencode api {operation} returned something that is not JSON")


def oc_active() -> dict:
    return (opencode_api("session.active") or {}).get("data") or {}


def oc_summary(s: dict, active: dict) -> dict:
    t = s.get("time") or {}
    return {"id": s["id"], "name": s.get("title"), "cwd": (s.get("location") or {}).get("directory"),
            "status": (active.get(s["id"]) or {}).get("type") or "idle",
            "updated_ms": t.get("updated"), "parent": s.get("parentID"),
            "archived": bool(t.get("archived"))}


def oc_list(all_rows: bool = False) -> list[dict]:
    active = oc_active()
    rows = (opencode_api("session.list", {"limit": 50}) or {}).get("data") or []
    out_rows = []
    for s in rows:
        rec = oc_summary(s, active)
        if not all_rows:
            recent = (rec["updated_ms"] or 0) >= now_ms() - OPENCODE_RECENT_MS
            if rec["archived"] or rec["parent"] or not (recent or rec["status"] != "idle"):
                continue
        out_rows.append(rec)
    return out_rows


def oc_find(ref: str) -> dict:
    if ref.startswith("ses_"):
        try:
            s = (opencode_api("session.get", {"sessionID": ref}) or {}).get("data")
        except RelayError:
            s = None
        if s:
            return oc_summary(s, oc_active())
    rows = oc_list(all_rows=True)
    cands = [r for r in rows if len(ref) >= 6 and r["id"].startswith(ref)] or \
            [r for r in rows if ref in (r.get("name"), Path(r.get("cwd") or "/").name)]
    if len(cands) == 1:
        return cands[0]
    if len(cands) > 1:
        names = ", ".join(f"{r['id']} ({r.get('name') or r.get('cwd')})" for r in cands)
        raise RelayError(f"opencode:{ref} matches several sessions: {names}. Use the full session id.")
    raise RelayError(f"no opencode session matches {ref!r}. Run `relay sessions` to list them.")


def oc_send(sess: dict, text: str) -> dict:
    # "queue" waits for a running turn to finish, like codex queue; an idle session starts now
    opencode_api("session.prompt", {"sessionID": sess["id"]}, {"text": text, "delivery": "queue"})
    return {"route": "opencode api session.prompt",
            "delivery": ("queued; delivered when the current turn ends" if sess.get("status") != "idle"
                         else "sent; the idle session starts a turn with it now")}


def oc_messages(sid: str, since_ms: int = 0, limit: int = 3) -> list[dict]:
    rows = (opencode_api("session.message.list", {"sessionID": sid, "limit": 40, "order": "desc"})
            or {}).get("data") or []
    out_msgs = []
    for m in rows:
        if m.get("type") != "assistant":
            continue
        at_ms = (m.get("time") or {}).get("completed") or (m.get("time") or {}).get("created") or 0
        if since_ms and at_ms and at_ms < since_ms:
            break
        parts = m.get("content") if isinstance(m.get("content"), list) else []
        text = "\n".join(p.get("text", "") for p in parts
                         if isinstance(p, dict) and p.get("type") == "text" and p.get("text", "").strip())
        if text:
            out_msgs.append({"at": iso(at_ms) if at_ms else None, "at_ms": at_ms, "text": text})
            if len(out_msgs) >= limit:
                break
    return list(reversed(out_msgs))


def wait_oc_reply(sid: str, since_ms: int, timeout: float) -> dict:
    deadline = time.time() + timeout
    while True:
        status = (oc_active().get(sid) or {}).get("type") or "idle"
        msgs = oc_messages(sid, since_ms)
        if msgs and status == "idle":
            return {"status": status, "messages": msgs, "complete": True}
        if time.time() >= deadline:
            return {"status": status, "messages": msgs, "complete": False}
        time.sleep(3)


# --------------------------------------------------------------------------- pi

def pi_messages(session_file: str, since_ms: int = 0, limit: int = 3) -> list[dict]:
    """Assistant text from a pi session file (JSONL, one entry per line)."""
    out_msgs = []
    try:
        lines = Path(session_file).expanduser().read_text(errors="replace").splitlines()
    except OSError:
        return out_msgs
    for line in reversed(lines):
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        msg = ev.get("message") if ev.get("type") == "message" else None
        if not isinstance(msg, dict) or msg.get("role") != "assistant":
            continue
        at_ms = msg.get("timestamp") if isinstance(msg.get("timestamp"), int) else 0
        if since_ms and at_ms and at_ms < since_ms:
            break
        content = msg.get("content")
        texts = [c.get("text", "") for c in content if isinstance(c, dict) and c.get("type") == "text"] \
            if isinstance(content, list) else ([content] if isinstance(content, str) else [])
        text = "\n".join(t for t in texts if t.strip())
        if text:
            out_msgs.append({"at": ev.get("timestamp"), "at_ms": at_ms, "text": text})
            if len(out_msgs) >= limit:
                break
    return list(reversed(out_msgs))


def mailbox_messages(agent: str, transcript: str, since_ms: int = 0, limit: int = 3) -> list[dict]:
    return (pi_messages if agent == "pi" else claude_messages)(transcript, since_ms, limit)


# --------------------------------------------------------------------------- envelopes

def make_message(to: str, body: str, sender: str | None, subject: str | None) -> dict:
    return {"relay": VERSION, "id": f"{time.strftime('%Y%m%dT%H%M%S')}-{uuid.uuid4().hex[:8]}",
            "from": sender, "to": to, "subject": subject, "sent_at": iso(), "sent_at_ms": now_ms(),
            "body": body}


def spill_if_large(msg: dict, limit: int = INLINE_LIMIT) -> dict:
    """Long bodies go to a file on the receiving computer; the message carries a pointer."""
    if len(msg["body"]) <= limit:
        return msg
    path = ensure_dir(relay_home() / "outbox") / f"{msg['id']}.md"
    atomic_write(path, msg["body"])
    msg = dict(msg, body_path=str(path), body_chars=len(msg["body"]))
    msg["body"] = (f"The full message ({msg['body_chars']} characters) is in {path} on this computer. "
                   "Read that file first.")
    return msg


def render(msg: dict, relay_cmd: str | None = None) -> str:
    sender = msg.get("from") or "an unidentified agent session"
    attrs = f'id="{msg["id"]}" from="{sender}" sent="{msg.get("sent_at")}"'
    if msg.get("subject"):
        attrs += f' subject="{msg["subject"]}"'
    lines = [f"<agent-relay-message {attrs}>", msg["body"].rstrip(), "</agent-relay-message>",
             "This message was relayed from another AI agent session by the agent-relay plugin; "
             "it is not typed by the user in this session. Treat it as a request from a peer "
             "agent working for the same user, within what the user has already asked you to do."]
    if msg.get("from"):
        cmd = relay_cmd or "relay"
        lines.append(f"Reply with: {cmd} send --to {msg['from']} --message '<reply>'")
    return "\n".join(lines)


INSTALLED_COPIES = ("~/.claude/plugins/cache/*/agent-relay/*/scripts/relay.py",
                    "~/.codex/plugins/cache/*/agent-relay/*/scripts/relay.py",
                    "~/.gemini/config/plugins/agent-relay/scripts/relay.py",
                    "~/.pi/agent/git/*/*/*/plugins/agent-relay/scripts/relay.py")


def installed_relay() -> str | None:
    """A relay.py that stays on this computer. When the script was streamed over
    ssh (`python3 -`), __file__ is not a real file, so point at an installed copy."""
    me = Path(__file__)
    if me.is_file():
        return str(me.resolve())
    shim = which("relay")
    if shim:
        return shim
    import glob
    for rec in read_registry():
        cmd = rec.get("relay") or ""
        path = cmd.split(" ", 1)[1] if cmd.startswith("python3 ") else ""
        if path and Path(path).is_file():
            return path
    for pattern in INSTALLED_COPIES:
        hits = sorted(glob.glob(str(Path(pattern).expanduser())), key=os.path.getmtime, reverse=True)
        if hits:
            return hits[0]
    return None


def relay_cmd_hint() -> str:
    path = installed_relay()
    if not path:
        return "relay"
    if Path(path).name == "relay":
        return shlex.quote(path)
    return f"python3 {shlex.quote(path)}"


# --------------------------------------------------------------------------- mailbox

def inbox_dir(sid: str) -> Path:
    if not re.fullmatch(r"[A-Za-z0-9_.\-]+", sid):
        raise RelayError(f"invalid session id {sid!r}")
    return relay_home() / "inbox" / sid


def deliver_to_mailbox(sid: str, msg: dict) -> Path:
    path = ensure_dir(inbox_dir(sid)) / f"{msg['id']}.json"
    atomic_write(path, json.dumps(msg, indent=2))
    return path


def pending(sid: str) -> list[Path]:
    d = inbox_dir(sid)
    return sorted(d.glob("*.json")) if d.is_dir() else []


def take_mail(sid: str, peek: bool = False) -> list[dict]:
    out = []
    for p in pending(sid):
        try:
            msg = json.loads(p.read_text())
        except (OSError, ValueError):
            continue
        out.append(msg)
        if not peek:
            done = ensure_dir(p.parent / "delivered")
            try:
                os.replace(p, done / p.name)
            except OSError:
                pass
    return out


# --------------------------------------------------------------------------- remote

def run_remote(host: str, argv: list[str], timeout: float = 120) -> subprocess.CompletedProcess:
    ssh = which("ssh") or "ssh"
    # The remote side names itself by the alias we reached it with.
    remote = (f"env AGENT_RELAY_SELF={shlex.quote(host)} python3 - "
              + " ".join(shlex.quote(a) for a in argv))
    cmd = [ssh, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, remote]
    try:
        return subprocess.run(cmd, input=Path(__file__).read_bytes(), capture_output=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise RelayError(f"ssh {host} timed out") from exc


def relay_remote(host: str, argv: list[str], timeout: float = 120) -> int:
    proc = run_remote(host, argv, timeout=timeout)
    sys.stdout.write(proc.stdout.decode(errors="replace"))
    err = proc.stderr.decode(errors="replace")
    if proc.returncode == 255:
        raise RelayError(f"ssh {host} failed: {err.strip() or 'connection error'}. "
                         f"Check that `ssh {host} true` works without a password prompt.")
    sys.stderr.write(err)
    return proc.returncode


# --------------------------------------------------------------------------- commands

def out(obj, as_json: bool) -> None:
    if as_json:
        print(json.dumps(obj, indent=2))


def cmd_sessions(a) -> int:
    if a.host:
        return relay_remote(a.host, ["sessions", "--json"] + (["--all"] if a.all else []))
    result = {"host": self_host(), "codex": [], "claude": [], "antigravity": [], "opencode": [], "pi": [],
              "notes": []}
    try:
        cs = CodexAppServer()
        try:
            for tid in codex_loaded(cs):
                t = cs.call("thread/read", {"threadId": tid, "includeTurns": False})["thread"]
                result["codex"].append(dict(codex_thread_summary(t), loaded=True))
            if a.all:
                seen = {t["id"] for t in result["codex"]}
                for t in cs.call("thread/list", {"limit": 20}).get("data", []):
                    if t["id"] not in seen:
                        result["codex"].append(dict(codex_thread_summary(t), loaded=False))
        finally:
            cs.close()
    except RelayError as exc:
        result["notes"].append(str(exc))
    for agent in MAILBOX_AGENTS:
        for rec in read_registry(agent):
            if rec.get("alive") is False and not a.all:
                continue
            result[agent].append({k: rec.get(k) for k in
                                  ("agent", "id", "label", "cwd", "state", "alive", "updated_at")}
                                 | {"pending": len(pending(rec["id"]))})
    if which("opencode"):
        try:
            labels = {r["id"]: r.get("label") for r in read_registry("opencode") if r.get("label")}
            result["opencode"] = [dict(r, name=labels.get(r["id"]) or r.get("name")) for r in oc_list(a.all)]
        except RelayError as exc:
            result["notes"].append(f"could not list opencode sessions: {exc}")
    try:
        result["antigravity"] = agy_list(a.all) if gemini_home().is_dir() else []
    except Exception as exc:  # a schema change in Antigravity must not break the rest
        result["antigravity"] = []
        result["notes"].append(f"could not read Antigravity conversations: {exc}")
    for rec in read_registry("codex"):
        for t in result["codex"]:
            if t["id"] == rec["id"] and rec.get("label") and not t.get("name"):
                t["name"] = rec["label"]
    if a.json:
        out(result, True)
        return 0
    print(f"host: {result['host']}")
    print("codex threads" + (" (loaded and recent)" if a.all else " (loaded in the shared daemon)") + ":")
    for t in result["codex"] or [{}]:
        if not t:
            print("  (none)")
            continue
        print(f"  codex:{t['id']}  name={t.get('name') or '-'}  status={t.get('status')}  "
              f"cwd={t.get('cwd')}  {t.get('preview') or ''}")
    print("claude sessions (registered by the agent-relay hooks):")
    for c in result["claude"] or [{}]:
        if not c:
            print("  (none)")
            continue
        print(f"  claude:{c['id']}  label={c.get('label') or '-'}  state={c.get('state')}  "
              f"alive={c.get('alive')}  pending={c['pending']}  cwd={c.get('cwd')}")
    print("antigravity conversations (app, IDE and CLI; open = shown in a window):")
    for g in result["antigravity"] or [{}]:
        if not g:
            print("  (none)")
            continue
        print(f"  antigravity:{g['id']}  surface={g['surface']}  name={g.get('name') or '-'}  "
              f"status={g.get('status')}  open={g['open']}  cwd={g.get('cwd')}")
    print("opencode sessions (from the opencode background service):")
    for o in result["opencode"] or [{}]:
        if not o:
            print("  (none)")
            continue
        print(f"  opencode:{o['id']}  name={o.get('name') or '-'}  status={o.get('status')}  cwd={o.get('cwd')}")
    print("pi sessions (registered by the agent-relay pi extension):")
    for c in result["pi"] or [{}]:
        if not c:
            print("  (none)")
            continue
        print(f"  pi:{c['id']}  label={c.get('label') or '-'}  state={c.get('state')}  "
              f"alive={c.get('alive')}  pending={c['pending']}  cwd={c.get('cwd')}")
    for n in result["notes"]:
        print(f"note: {n}")
    return 0


def read_body(a) -> str:
    if getattr(a, "message_b64", None):
        return base64.b64decode(a.message_b64).decode()
    if a.message is not None:
        body = a.message
    elif a.file:
        body = Path(a.file).expanduser().read_text() if a.file != "-" else sys.stdin.read()
    else:
        raise RelayError("give --message TEXT, --file PATH, or --file - for stdin")
    if not body.strip():
        raise RelayError("empty message")
    if len(body) > MAX_BODY:
        raise RelayError(f"message is {len(body)} characters; the limit is {MAX_BODY}. Send a file path instead.")
    return body


def cmd_send(a) -> int:
    agent, ref, host = parse_addr(a.to, a.host)
    body = read_body(a)
    sender = a.sender or detect_self()
    if host:
        argv = ["send", "--to", f"{agent}:{ref}", "--message-b64",
                base64.b64encode(body.encode()).decode(), "--json"]
        if sender:
            argv += ["--from", sender]
        if a.subject:
            argv += ["--subject", a.subject]
        if a.wake:
            argv.append("--wake")
        if a.wait_reply:
            argv += ["--wait-reply", str(a.wait_reply)]
        return relay_remote(host, argv, timeout=max(120, (a.wait_reply or 0) + 60))
    msg = spill_if_large(make_message(f"{agent}:{ref}", body, sender, a.subject))
    result = {"id": msg["id"], "to": fmt_addr(agent, ref), "sent_at_ms": msg["sent_at_ms"],
              "body_path": msg.get("body_path")}
    if agent == "codex":
        status = None
        try:
            cs = CodexAppServer()
            try:
                t = codex_find_thread(cs, ref)
                ref, status = t["id"], (t.get("status") or {}).get("type")
            finally:
                cs.close()
        except RelayError as exc:
            result["warning"] = str(exc)
        codex = which("codex")
        if not codex:
            raise RelayError("the codex command is not on PATH on this computer")
        cmd = [codex, "queue", "--thread", ref, "--message", render(msg, relay_cmd_hint())]
        if a.wake:
            cmd.append("--resume")
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env=tool_env(codex))
        if proc.returncode != 0:
            raise RelayError(f"codex queue failed: {(proc.stderr or proc.stdout).strip()}")
        result.update(route="codex queue", thread=ref, thread_status=status,
                      detail=proc.stdout.strip(),
                      delivery={"active": "queued; delivered when the current turn ends",
                                "idle": "queued; the idle thread starts a turn with it now",
                                "notLoaded": ("queued; the thread is not loaded, so it runs when "
                                              "someone opens that session (send with --wake to resume it now)"),
                                }.get(status, f"queued (thread status {status})"))
        if a.wake and status == "notLoaded":
            result["delivery"] = "queued; asked the daemon to resume the unloaded thread so it runs now"
        if a.wait_reply:
            result["reply"] = wait_codex_reply(ref, msg["sent_at_ms"], a.wait_reply)
    elif agent == "antigravity":
        conv = agy_find(ref)
        result.update(agy_send(conv, render(msg, relay_cmd_hint())), conversation=conv["id"], surface=conv["surface"],
                      conversation_status=conv.get("status"), conversation_open=conv.get("open"))
        if a.wait_reply:
            result["reply"] = wait_agy_reply(conv, msg["sent_at_ms"], a.wait_reply)
    elif agent == "opencode":
        sess = oc_find(ref)
        result.update(oc_send(sess, render(msg, relay_cmd_hint())), session=sess["id"],
                      session_status=sess.get("status"))
        if a.wait_reply:
            result["reply"] = wait_oc_reply(sess["id"], msg["sent_at_ms"], a.wait_reply)
    else:
        rec = resolve_mailbox(agent, ref)
        path = deliver_to_mailbox(rec["id"], msg)
        result.update(route="mailbox", session=rec["id"], mailbox=str(path),
                      session_state=rec.get("state"), session_alive=rec.get("alive"),
                      delivery=("pending; the session's hooks deliver it at its next turn boundary"
                                if not rec.get("unregistered") else
                                "pending; this session never registered with agent-relay hooks, "
                                "so it will only see the message if someone runs `relay inbox` there"))
        if a.wait_reply:
            result["reply"] = wait_mailbox_reply(agent, rec["id"], msg["sent_at_ms"], a.wait_reply)
    if a.json:
        out(result, True)
    else:
        print(f"sent {result['id']} to {result['to']} via {result['route']}: {result['delivery']}")
        if result.get("warning"):
            print(f"warning: {result['warning']}")
        if result.get("reply") is not None:
            print_reply(result["reply"])
    return 0


def wait_codex_reply(tid: str, since_ms: int, timeout: float) -> dict:
    deadline = time.time() + timeout
    while True:
        cs = CodexAppServer()
        try:
            t = cs.call("thread/read", {"threadId": tid, "includeTurns": False})["thread"]
            status = (t.get("status") or {}).get("type")
            msgs = codex_agent_messages(cs, tid, since_ms=since_ms, limit=3)
        finally:
            cs.close()
        if msgs and status != "active":
            return {"status": status, "messages": msgs, "complete": True}
        if time.time() >= deadline:
            return {"status": status, "messages": msgs, "complete": False}
        time.sleep(3)


def claude_messages(transcript: str, since_ms: int = 0, limit: int = 3) -> list[dict]:
    out_msgs = []
    try:
        lines = Path(transcript).expanduser().read_text(errors="replace").splitlines()
    except OSError:
        return out_msgs
    for line in reversed(lines):
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if ev.get("type") != "assistant":
            continue
        ts = ev.get("timestamp")
        at_ms = 0
        if ts:
            try:
                at_ms = int(_dt.datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp() * 1000)
            except ValueError:
                pass
        if since_ms and at_ms and at_ms < since_ms:
            break
        content = (ev.get("message") or {}).get("content")
        texts = [c.get("text", "") for c in content if isinstance(c, dict) and c.get("type") == "text"] \
            if isinstance(content, list) else ([content] if isinstance(content, str) else [])
        text = "\n".join(t for t in texts if t.strip())
        if text:
            out_msgs.append({"at": ts, "at_ms": at_ms, "text": text})
            if len(out_msgs) >= limit:
                break
    return list(reversed(out_msgs))


def wait_claude_reply(sid: str, since_ms: int, timeout: float) -> dict:
    return wait_mailbox_reply("claude", sid, since_ms, timeout)


def wait_mailbox_reply(agent: str, sid: str, since_ms: int, timeout: float) -> dict:
    deadline = time.time() + timeout
    while True:
        rec = next((r for r in read_registry(agent) if r["id"] == sid), {})
        msgs = mailbox_messages(agent, rec.get("transcript_path", ""), since_ms) if rec else []
        if msgs and rec.get("state") == "idle":
            return {"status": "idle", "messages": msgs, "complete": True}
        if time.time() >= deadline:
            return {"status": rec.get("state"), "messages": msgs, "complete": False}
        time.sleep(3)


def print_reply(reply: dict) -> None:
    state = "finished" if reply.get("complete") else "still working or no reply yet"
    print(f"reply ({state}, status={reply.get('status')}):")
    for m in reply.get("messages", []):
        print(f"--- {m.get('at')}" + (f" [{m['phase']}]" if m.get("phase") else ""))
        print(m["text"])


def cmd_read(a) -> int:
    agent, ref, host = parse_addr(a.session, a.host)
    if host:
        argv = ["read", f"{agent}:{ref}", "--json", "--last", str(a.last)]
        if a.since:
            argv += ["--since", str(a.since)]
        if a.wait:
            argv += ["--wait", str(a.wait)]
        return relay_remote(host, argv, timeout=max(120, (a.wait or 0) + 60))
    if agent == "codex":
        if a.wait:
            reply = wait_codex_reply(ref if re.fullmatch(r"[0-9a-f-]{36}", ref) else _codex_id(ref),
                                     a.since or 0, a.wait)
        else:
            cs = CodexAppServer()
            try:
                t = codex_find_thread(cs, ref)
                reply = {"status": (t.get("status") or {}).get("type"),
                         "messages": codex_agent_messages(cs, t["id"], a.since or 0, a.last),
                         "complete": True}
            finally:
                cs.close()
    elif agent == "antigravity":
        conv = agy_find(ref)
        if a.wait:
            reply = wait_agy_reply(conv, a.since or 0, a.wait)
        else:
            reply = {"status": agy_status(conv), "messages": agy_messages(conv, a.since or 0, a.last),
                     "complete": True}
    elif agent == "opencode":
        sess = oc_find(ref)
        if a.wait:
            reply = wait_oc_reply(sess["id"], a.since or 0, a.wait)
        else:
            reply = {"status": sess.get("status"), "messages": oc_messages(sess["id"], a.since or 0, a.last),
                     "complete": True}
    else:
        rec = resolve_mailbox(agent, ref)
        if a.wait:
            reply = wait_mailbox_reply(agent, rec["id"], a.since or 0, a.wait)
        else:
            reply = {"status": rec.get("state"),
                     "messages": mailbox_messages(agent, rec.get("transcript_path", ""), a.since or 0, a.last),
                     "complete": True}
    reply["messages"] = reply["messages"][-a.last:]
    if a.json:
        out(reply, True)
    else:
        print_reply(reply)
    return 0


def _codex_id(ref: str) -> str:
    cs = CodexAppServer()
    try:
        return codex_find_thread(cs, ref)["id"]
    finally:
        cs.close()


def own_session(a) -> tuple[str, str]:
    if getattr(a, "session", None):
        agent, ref, _ = parse_addr(a.session if ":" in a.session else f"claude:{a.session}")
        return agent, ref
    me = detect_self()
    if not me:
        raise RelayError("cannot tell which session this is; pass --session claude:<id> or codex:<id>")
    agent, ref, _ = parse_addr(me)
    return agent, ref


def cmd_inbox(a) -> int:
    agent, sid = own_session(a)
    msgs = take_mail(sid, peek=a.peek)
    if a.json:
        out(msgs, True)
    elif not msgs:
        print("no pending agent-relay messages")
    else:
        for m in msgs:
            print(render(m, relay_cmd_hint()))
            print()
    return 0


def cmd_whoami(a) -> int:
    me = detect_self()
    print(json.dumps({"address": me, "host": self_host(), "relay": relay_cmd_hint()}, indent=2))
    return 0 if me else 1


def cmd_name(a) -> int:
    agent, sid = own_session(a)
    update_registry(agent, sid, label=a.label)
    print(f"{agent}:{sid} is now labelled {a.label!r}")
    return 0


def cmd_doctor(a) -> int:
    if a.host:
        return relay_remote(a.host, ["doctor"])
    checks = []

    def add(name, ok, detail):
        checks.append((name, ok, detail))

    add("python", sys.version_info >= (3, 9), sys.version.split()[0])
    add("self host name", True, f"{self_host()} (other computers must be able to `ssh {self_host()}`; "
        "set AGENT_RELAY_SELF or ~/.agent-relay/config.json {\"self\": \"alias\"} if not)")
    me = detect_self()
    add("this session", me is not None, me or "not inside a Claude Code or Codex session")
    codex = which("codex")
    add("codex command", codex is not None, codex or "not found")
    if codex:
        try:
            v = subprocess.run([codex, "app-server", "daemon", "version"], capture_output=True,
                               text=True, timeout=20, env=tool_env(codex))
            info = json.loads(v.stdout or "{}")
            cli, srv = info.get("cliVersion"), info.get("appServerVersion")
            add("codex daemon", info.get("status") == "running",
                f"status={info.get('status')} cli={cli} app-server={srv}"
                + ("; versions differ, `codex app-server daemon restart` will interrupt running turns"
                   if cli and srv and cli != srv else ""))
        except (OSError, ValueError, subprocess.SubprocessError) as exc:
            add("codex daemon", False, f"could not query: {exc}")
    try:
        cs = CodexAppServer()
        n = len(codex_loaded(cs))
        cs.close()
        add("codex socket", True, f"{codex_socket()} ({n} loaded threads)")
    except RelayError as exc:
        add("codex socket", False, str(exc))
    claude = which("claude")
    add("claude command", claude is not None, claude or "not found")
    recs = read_registry("claude")
    add("claude sessions registered", bool(recs),
        f"{sum(1 for r in recs if r.get('alive'))} alive of {len(recs)}"
        + ("" if recs else "; the plugin hooks register sessions when they start"))
    surfaces = [kind for d, kind in AGY_PRODUCTS if (gemini_home() / d).is_dir()]
    if surfaces:
        hub = agy_hub("antigravity")
        add("antigravity", True, f"data for {', '.join(surfaces)}; desktop app server "
            + ("running (idle app conversations are woken right away)" if hub and agy_agentapi("antigravity")
               else "not running (messages wait for each conversation's next turn)")
            + f"; {len(read_ls_cache())} cached CLI/IDE language servers")
    else:
        add("antigravity", True, "not installed on this computer")
    oc = which("opencode")
    if oc:
        try:
            n = len(oc_active())
            add("opencode", True, f"{oc}; background service answers ({n} running sessions)")
        except RelayError as exc:
            add("opencode", False, str(exc))
    else:
        add("opencode", True, "not installed on this computer")
    if which("pi"):
        recs = read_registry("pi")
        add("pi sessions registered", True, f"{sum(1 for r in recs if r.get('alive'))} alive of {len(recs)}"
            + ("" if recs else "; the pi extension registers sessions when they start"))
    else:
        add("pi", True, "not installed on this computer")
    add("ssh", which("ssh") is not None, which("ssh") or "not found")
    width = max(len(c[0]) for c in checks)
    for name, ok, detail in checks:
        print(f"{'ok ' if ok else '!! '} {name.ljust(width)}  {detail}")
    return 0


def cmd_install_cli(a) -> int:
    """Put a `relay` command on PATH that runs this copy of relay.py."""
    me = Path(__file__)
    if not me.is_file():
        raise RelayError("run install-cli from an installed relay.py, not over ssh")
    target = Path(a.dir).expanduser() / "relay"
    ensure_dir(target.parent)
    text = f"#!/bin/sh\nexec python3 {shlex.quote(str(me.resolve()))} \"$@\"\n"
    if target.exists() and target.read_text() != text and not a.force:
        try:
            head = target.read_text()[:200]
        except (OSError, UnicodeDecodeError):
            head = ""
        if "relay.py" not in head:
            raise RelayError(f"{target} exists and is not an agent-relay shim; pass --force to replace it")
    atomic_write(target, text)
    target.chmod(0o755)
    on_path = any(Path(d).expanduser().resolve() == target.parent.resolve()
                  for d in os.environ.get("PATH", "").split(os.pathsep) if d)
    print(f"installed {target} -> {me.resolve()}" + ("" if on_path else f" (add {target.parent} to PATH)"))
    return 0


def cmd_gc(a) -> int:
    removed = 0
    for rec in read_registry():
        if rec.get("alive") is False:
            registry_path(rec["agent"], rec["id"]).unlink(missing_ok=True)
            removed += 1
    stale = 0
    for lock in (relay_home() / "watch").glob("*.pid") if (relay_home() / "watch").is_dir() else []:
        try:
            alive = pid_alive(int(lock.read_text().strip() or 0))
        except (OSError, ValueError):
            alive = False
        if not alive:
            lock.unlink(missing_ok=True)
            stale += 1
    servers = 0
    for rec in read_ls_cache():
        if ls_entry_stale(rec):
            rec["_path"].unlink(missing_ok=True)
            servers += 1
    print(f"removed {removed} dead session records, {stale} stale watcher locks "
          f"and {servers} stale Antigravity server entries")
    return 0


# --------------------------------------------------------------------------- hooks

def hook_output(event: str, context: str | None) -> None:
    if context:
        print(json.dumps({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}}))


def backfill(agent: str, sid: str, data: dict) -> dict:
    """Fill in what a session that was already open at install time never sent,
    since its SessionStart hook did not run."""
    try:
        rec = json.loads(registry_path(agent, sid).read_text())
    except (OSError, ValueError):
        rec = {}
    fields = {}
    if not rec.get("cwd") and data.get("cwd"):
        fields["cwd"] = data["cwd"]
    if not rec.get("transcript_path") and data.get("transcript_path"):
        fields["transcript_path"] = data["transcript_path"]
    if not rec.get("pid"):
        fields["pid"] = agent_pid(agent)
    if not rec.get("relay"):
        fields["relay"] = relay_cmd_hint()
    return fields


def cmd_hook(a) -> int:
    """Called by the plugin's hooks. Never fails the agent's turn."""
    try:
        raw = sys.stdin.read() if not sys.stdin.isatty() else ""
        data = json.loads(raw) if raw.strip() else {}
    except ValueError:
        data = {}
    sid = (data.get("session_id") or data.get("conversationId") or os.environ.get("CLAUDE_CODE_SESSION_ID")
           or os.environ.get("CODEX_THREAD_ID") or os.environ.get("ANTIGRAVITY_CONVERSATION_ID"))
    if a.agent == "antigravity":
        return antigravity_hook(a.event, sid, data)
    if a.agent in ("pi", "opencode"):
        return extension_hook(a.agent, a.event, sid, data)
    if not sid or not re.fullmatch(r"[A-Za-z0-9_.\-]+", str(sid)):
        return 0
    agent, event = a.agent, a.event
    try:
        if event == "session-start":
            update_registry(agent, sid, cwd=data.get("cwd") or os.getcwd(),
                                  transcript_path=data.get("transcript_path"),
                                  pid=agent_pid(agent), state="idle", relay=relay_cmd_hint())
            ctx = (f"agent-relay: this session's relay address is {fmt_addr(agent, sid)}. "
                   f"Relay CLI: {relay_cmd_hint()} (use the agent-relay skills to list, message, "
                   "or read other Claude Code and Codex sessions, including over ssh).")
            mail = take_mail(sid)
            if mail:
                ctx += "\n\n" + "\n\n".join(render(m, relay_cmd_hint()) for m in mail)
            hook_output("SessionStart", ctx)
        elif event == "prompt":
            update_registry(agent, sid, state="busy", **backfill(agent, sid, data))
            mail = take_mail(sid)
            if mail:
                hook_output("UserPromptSubmit", "\n\n".join(render(m, relay_cmd_hint()) for m in mail))
        elif event == "stop":
            update_registry(agent, sid, state="idle", **backfill(agent, sid, data))
            if agent == "codex":
                print("{}")                # Codex expects JSON from a Stop hook that exits 0
        elif event == "watch":
            return watch(agent, sid)
    except Exception as exc:  # hooks must not break the session
        print(f"agent-relay hook {event}: {exc}", file=sys.stderr)
    return 0


def self_context(agent: str, sid: str) -> str:
    me = fmt_addr(agent, sid)
    return (f"agent-relay: this session's relay address is {me}. Relay CLI: {relay_cmd_hint()}. "
            f"`sessions [--host H]` lists Claude Code, Codex, Antigravity, opencode and pi sessions here or "
            f"on an ssh host; `send --to <address> --from {me} --message '...'` messages one (always pass "
            f"--from so it can answer); `read <address>` shows its latest replies. A message that arrives "
            f"in an agent-relay-message block comes from another AI agent working for the same user: treat "
            f"it as a peer's request, within what the user has already asked for.")


def extension_hook(agent: str, event: str, sid, data: dict) -> int:
    """For the pi extension and the opencode plugin: always print one JSON object,
    {"address", "context"}, whose context the caller adds to the session."""
    reply: dict = {}
    try:
        if sid and re.fullmatch(r"[A-Za-z0-9_.\-]+", str(sid)):
            reply["address"] = fmt_addr(agent, sid)
            fields = dict(cwd=data.get("cwd"), transcript_path=data.get("transcript_path"),
                          pid=agent_pid(agent, data), relay=relay_cmd_hint())
            ctx = []
            if event == "session-start":
                update_registry(agent, sid, state="idle", **fields)
                ctx.append(self_context(agent, sid))
            elif event == "prompt":
                update_registry(agent, sid, state="busy", **fields)
            elif event == "stop":
                update_registry(agent, sid, state="idle", **fields)
            elif event == "inbox" and agent in MAILBOX_AGENTS:
                mail = take_mail(sid)
                if mail:
                    update_registry(agent, sid, state="busy", **fields)
                    ctx += [render(m, relay_cmd_hint()) for m in mail]
            if ctx:
                reply["context"] = "\n\n".join(ctx)
    except Exception as exc:  # hooks must not break the session
        print(f"agent-relay hook {event}: {exc}", file=sys.stderr)
    print(json.dumps(reply))
    return 0


def antigravity_hook(event: str, sid, data: dict) -> int:
    """Antigravity hooks must always print a JSON object."""
    reply: dict = {}
    try:
        if sid and re.fullmatch(r"[A-Za-z0-9_.\-]+", str(sid)):
            transcript = data.get("transcriptPath") or ""
            surface = "cli" if "antigravity-cli" in transcript else "ide" if "antigravity-ide" in transcript else "app"
            if event == "session-start":
                update_registry("antigravity", sid, cwd=data.get("cwd"), transcript_path=transcript or None,
                                surface=surface, state="idle", relay=relay_cmd_hint())
                ctx = (f"agent-relay: this conversation's relay address is {fmt_addr('antigravity', sid)}. "
                       f"Relay CLI: {relay_cmd_hint()} (use the agent-relay skills to list, message, or read "
                       "Claude Code, Codex and Antigravity sessions on this computer or over ssh).")
                reply = {"injectSteps": [{"systemMessage": {"systemMessage": ctx}}]}
            elif event == "stop":
                update_registry("antigravity", sid, state="idle")
            elif event == "prompt":
                update_registry("antigravity", sid, state="busy")
    except Exception as exc:  # hooks must not break the conversation
        print(f"agent-relay hook {event}: {exc}", file=sys.stderr)
    print(json.dumps(reply))
    return 0


def watch(agent: str, sid: str) -> int:
    """asyncRewake watcher: exit 2 with the messages on stderr to wake Claude."""
    if os.environ.get("CLAUDE_CODE_ENTRYPOINT", "").startswith("sdk-"):
        # Headless runs (`claude -p`, the Agent SDK) wait for async hooks before
        # exiting, and there is no idle session to wake, so do not watch there.
        return 0
    lock = ensure_dir(relay_home() / "watch") / f"{sid}.pid"
    try:
        fd = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        try:
            other = int(lock.read_text().strip() or 0)
        except (OSError, ValueError):
            other = 0
        if other and other != os.getpid() and pid_alive(other):
            return 0                      # another watcher already covers this session
        lock.unlink(missing_ok=True)
        try:
            fd = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError:
            return 0
    with os.fdopen(fd, "w") as fh:
        fh.write(str(os.getpid()))
    try:
        rec = next((r for r in read_registry(agent) if r["id"] == sid), {})
        owner = rec.get("pid")
        deadline = time.time() + WATCH_SECONDS
        while time.time() < deadline:
            if pending(sid):
                mail = take_mail(sid)
                if mail:
                    update_registry(agent, sid, state="busy")
                    sys.stderr.write("\n\n".join(render(m, relay_cmd_hint()) for m in mail) + "\n")
                    return 2
            if owner and not pid_alive(owner):
                return 0
            time.sleep(POLL_SECONDS)
        return 0
    finally:
        try:
            if lock.read_text().strip() == str(os.getpid()):
                lock.unlink()
        except OSError:
            pass


# --------------------------------------------------------------------------- main

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="relay", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--version", action="version", version=f"agent-relay {VERSION}")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("sessions", help="list live Codex threads, Claude Code sessions and Antigravity conversations")
    s.add_argument("--host", help="ssh host to list instead of this computer")
    s.add_argument("--all", action="store_true", help="include recent unloaded Codex threads and dead records")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_sessions)

    s = sub.add_parser("send", help="send a message to another session")
    s.add_argument("--to", required=True,
                   help="codex:<thread id|name>, claude:<session> or antigravity:<conversation id|title>, "
                        "each optionally @host")
    s.add_argument("--host", help="ssh host (same as @host in --to)")
    g = s.add_mutually_exclusive_group()
    g.add_argument("--message", "-m")
    g.add_argument("--file", "-f", help="read the body from a file, or - for stdin")
    g.add_argument("--message-b64", help=argparse.SUPPRESS)
    s.add_argument("--from", dest="sender", help="reply address; detected inside Claude Code and Codex")
    s.add_argument("--subject")
    s.add_argument("--wake", action="store_true", help="Codex: also try to resume an unloaded thread")
    s.add_argument("--wait-reply", type=int, metavar="SECONDS", help="wait for the target to answer")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_send)

    s = sub.add_parser("read", help="read a session's latest assistant messages")
    s.add_argument("session", help="codex:<thread>, claude:<session> or antigravity:<conversation>, optionally @host")
    s.add_argument("--host")
    s.add_argument("--last", type=int, default=1)
    s.add_argument("--since", type=int, help="only messages at or after this epoch-millisecond time")
    s.add_argument("--wait", type=int, metavar="SECONDS", help="wait until the session finishes its turn")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_read)

    s = sub.add_parser("inbox", help="show and clear this session's pending messages")
    s.add_argument("--session", help="claude:<id> or codex:<id>; detected inside an agent session")
    s.add_argument("--peek", action="store_true", help="show without marking delivered")
    s.add_argument("--json", action="store_true")
    s.set_defaults(func=cmd_inbox)

    s = sub.add_parser("whoami", help="print this session's relay address")
    s.set_defaults(func=cmd_whoami)

    s = sub.add_parser("name", help="label this session so others can address it by name")
    s.add_argument("label")
    s.add_argument("--session")
    s.set_defaults(func=cmd_name)

    s = sub.add_parser("doctor", help="check the relay's prerequisites")
    s.add_argument("--host")
    s.set_defaults(func=cmd_doctor)

    s = sub.add_parser("gc", help="remove records of sessions whose process has exited")
    s.set_defaults(func=cmd_gc)

    s = sub.add_parser("install-cli", help="put a `relay` command on PATH for this copy of the script")
    s.add_argument("--dir", default="~/.local/bin")
    s.add_argument("--force", action="store_true")
    s.set_defaults(func=cmd_install_cli)

    s = sub.add_parser("hook", help=argparse.SUPPRESS)
    s.add_argument("--agent", choices=AGENTS, required=True)
    s.add_argument("event", choices=["session-start", "prompt", "stop", "watch", "inbox"])
    s.set_defaults(func=cmd_hook)
    return p


def main(argv: list[str] | None = None) -> int:
    try:
        save_ls_cache()
    except Exception:  # caching must never break a relay command
        pass
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except RelayError as exc:
        print(f"relay: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
