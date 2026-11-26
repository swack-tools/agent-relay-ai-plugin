#!/usr/bin/env python3
"""Agent Relay MCP Server (stdio JSON-RPC 2.0 + MCP Apps UI).

Python 3.9+ standard library only. Serves the interactive `ui://agent-relay/relay-map.html`
MCP App resource and exposes 7 model-visible + 2 app-only tools for live multi-agent
observability, messaging, broadcasts, and transcript inspection across Claude Code,
Codex, Antigravity, opencode, and pi.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

VERSION = "1.1.0"
APP_URI = "ui://agent-relay/relay-map.html"
VALID_KINDS = ("claude", "codex", "antigravity", "opencode", "pi", "operator")
RELAY_TARGET_KINDS = {"claude", "codex", "antigravity", "opencode", "pi"}
HOST_PALETTE = ("#ffd166", "#06d6a0", "#ef476f", "#8ecae6", "#c77dff", "#f4a261", "#90be6d")

MAX_MESSAGES = 600
MAX_FILES = 120
MAX_GIT = 80
MAX_SUBAGENTS = 60
MAX_BODY_PREVIEW = 1800
MAX_STRUCTURED_BYTES = 720 * 1024
LIVE_WINDOW_MS = 2 * 3600 * 1000
STUB_WINDOW_MS = 5 * 60 * 1000

_SEND_LOCK = threading.Lock()
_PREFS_LOCK = threading.Lock()


def _plugin_root() -> Path:
    return Path(__file__).resolve().parent.parent


def _relay_py() -> Path:
    return Path(__file__).resolve().parent / "relay.py"


def _relay_home() -> Path:
    return Path(os.environ.get("AGENT_RELAY_HOME", "~/.agent-relay")).expanduser()


def _home_dir() -> Path:
    return Path(os.environ.get("HOME", "~")).expanduser()


def _gemini_home() -> Path:
    return Path(os.environ.get("AGENT_RELAY_GEMINI_HOME", "~/.gemini")).expanduser()


def _codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME", "~/.codex")).expanduser()


def _claude_projects_dir() -> Path:
    cfg = os.environ.get("CLAUDE_CONFIG_DIR")
    if cfg:
        return Path(cfg).expanduser() / "projects"
    return _home_dir() / ".claude" / "projects"


def _now_ms() -> int:
    return int(time.time() * 1000)


def _self_host() -> str:
    env_self = os.environ.get("AGENT_RELAY_SELF", "").strip()
    if env_self:
        return env_self
    try:
        cfg = json.loads((_relay_home() / "config.json").read_text(encoding="utf-8"))
        if isinstance(cfg, dict) and cfg.get("self"):
            return str(cfg["self"])
    except (OSError, ValueError):
        pass
    return socket.gethostname().split(".")[0] or "localhost"


def _truncate(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    return text[:limit] + "…"


def _host_color(host: str) -> str:
    h = 2166136261
    for b in host.encode("utf-8", errors="replace"):
        h ^= b
        h = (h * 16777619) & 0xFFFFFFFF
    return HOST_PALETTE[h % len(HOST_PALETTE)]


def _project_of(cwd: str) -> str:
    parts = [p for p in cwd.split("/") if p]
    if not parts:
        return "(no folder)"
    for idx, part in enumerate(parts):
        if part in (".claude", ".worktrees"):
            if idx > 0 and (part == ".worktrees" or (idx + 1 < len(parts) and parts[idx + 1] == "worktrees")):
                return parts[idx - 1]
    for idx, part in enumerate(parts):
        if part == ".codex" and idx + 1 < len(parts) and parts[idx + 1] == "worktrees":
            if idx + 3 < len(parts):
                return parts[idx + 3]
    if (len(parts) == 2 and parts[0] in ("Users", "home")) or parts == ["root"]:
        return "home"
    if (len(parts) >= 3 and parts[0] == "private" and parts[1] == "tmp") or parts[0] == "tmp":
        return "tmp"
    return parts[-1]


def _pid_alive(pid: Any) -> bool | None:
    try:
        p = int(pid)
    except (TypeError, ValueError):
        return None
    if p <= 1:
        return False
    try:
        os.kill(p, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        return False


def _parse_ts(val: Any) -> int | None:
    if isinstance(val, (int, float)) and not isinstance(val, bool):
        x = int(val)
        return x * 1000 if 0 < x < 10_000_000_000 else x
    if isinstance(val, str) and val.strip():
        s = val.strip()
        if s.endswith("Z"):
            s = s[:-1] + "+00:00"
        try:
            return int(_dt.datetime.fromisoformat(s).timestamp() * 1000)
        except ValueError:
            return None
    return None


def _content_text(val: Any) -> str:
    if isinstance(val, str):
        return val
    if isinstance(val, list):
        parts = [str(item.get("text", "")) for item in val if isinstance(item, dict) and isinstance(item.get("text"), str)]
        return "\n".join(parts)
    return ""


_ADDR_RE = re.compile(r"^\s*['\"]?(claude|codex|antigravity|opencode|pi|operator):([^@\s][^@]*)(?:@([^\s'\"]+))?['\"]?\s*$")


def _parse_addr(text: str, default_host: str = "") -> tuple[str, str, str] | None:
    m = _ADDR_RE.match(text or "")
    if not m:
        return None
    kind, ref, host = m.group(1), m.group(2), m.group(3) or ""
    if not host or host in ("local", "localhost"):
        host = default_host
    return kind, ref, host


def _fmt_addr(kind: str, ref: str, host: str) -> str:
    return f"{kind}:{ref}@{host}" if host else f"{kind}:{ref}"


# --------------------------------------------------------------------------- minimal TOML config

def _strip_toml_comment(line: str) -> str:
    in_quote = ""
    escaped = False
    for idx, ch in enumerate(line):
        if escaped:
            escaped = False
            continue
        if ch == "\\" and in_quote == '"':
            escaped = True
            continue
        if ch in ('"', "'"):
            if not in_quote:
                in_quote = ch
            elif in_quote == ch:
                in_quote = ""
            continue
        if ch == "#" and not in_quote:
            return line[:idx]
    return line


def _parse_toml_val(raw: str) -> Any:
    s = raw.strip()
    if s in ("true", "false"):
        return s == "true"
    if (s.startswith('"') and s.endswith('"')) or (s.startswith("'") and s.endswith("'")):
        if s.startswith('"'):
            try:
                return json.loads(s)
            except ValueError:
                return s[1:-1]
        return s[1:-1]
    if s.startswith("[") and s.endswith("]"):
        inner = s[1:-1].strip()
        if not inner:
            return []
        items: list[Any] = []
        buf = []
        quote = ""
        esc = False
        for ch in inner:
            if esc:
                buf.append(ch)
                esc = False
                continue
            if ch == "\\" and quote == '"':
                buf.append(ch)
                esc = True
                continue
            if ch in ('"', "'"):
                if not quote:
                    quote = ch
                elif quote == ch:
                    quote = ""
                buf.append(ch)
                continue
            if ch == "," and not quote:
                tok = "".join(buf).strip()
                if tok:
                    items.append(_parse_toml_val(tok))
                buf = []
                continue
            buf.append(ch)
        tail = "".join(buf).strip()
        if tail:
            items.append(_parse_toml_val(tail))
        return items
    try:
        if "." in s:
            return float(s)
        return int(s)
    except ValueError:
        return s


def _parse_simple_toml(text: str) -> dict[str, Any]:
    root: dict[str, Any] = {}
    cur: dict[str, Any] = root
    for raw_line in text.splitlines():
        line = _strip_toml_comment(raw_line).strip()
        if not line:
            continue
        if line.startswith("[[") and line.endswith("]]"):
            sec = line[2:-2].strip()
            lst = root.setdefault(sec, [])
            if isinstance(lst, list):
                item: dict[str, Any] = {}
                lst.append(item)
                cur = item
            continue
        if line.startswith("[") and line.endswith("]"):
            sec = line[1:-1].strip()
            obj = root.setdefault(sec, {})
            if isinstance(obj, dict):
                cur = obj
            continue
        if "=" in line:
            k, v = line.split("=", 1)
            cur[k.strip()] = _parse_toml_val(v)
    return root


def _load_config() -> dict[str, Any]:
    candidates: list[Path] = []
    env_cfg = os.environ.get("RELAY_MAP_CONFIG", "").strip()
    if env_cfg:
        candidates.append(Path(env_cfg).expanduser())
    candidates.append(Path.cwd() / "relay-map.toml")
    candidates.append(_relay_home() / "relay-map.toml")
    for p in candidates:
        if p.is_file():
            try:
                return _parse_simple_toml(p.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                pass
    return {}


# --------------------------------------------------------------------------- transcript & command extractors

_TAG_NOTE_RE = re.compile(r"<note>\s*(.*?)\s*</note>", re.DOTALL)
_TAG_STRIP_RE = re.compile(
    r"<(system-reminder|ADDITIONAL_METADATA|local-command-stdout|command-message|command-args)>.*?</\w[\w-]*>",
    re.DOTALL,
)
_ANY_TAG_RE = re.compile(r"</?[A-Za-z_][\w\-]*(?:\s[^>]*)?>")
_REQ_RE = re.compile(r"## My request(?: for Codex)?:\s*(.*)", re.DOTALL)
_SKILL_PATH_RE = re.compile(r"skills/([A-Za-z0-9_.\-]+)/SKILL\.md")
_RECV_RE = re.compile(
    r'<agent-relay-message id="([^"]+)" from="([^"]*)" sent="([^"]*)"(?: subject="([^"]*)")?>\n?(.*?)</agent-relay-message>',
    re.DOTALL,
)
_GIT_RE = re.compile(
    r"(?:^|[\s;&|(`])(git(?:\s+(?:-C\s+\S+|-c\s+\S+|--no-pager|--git-dir=\S+|--work-tree=\S+))*\s+([a-z][a-z\-]+)(?:\s+([a-z][a-z\-]+))?)"
)
_GH_RE = re.compile(
    r"(?:^|[\s;&|(`])(gh\s+(pr|issue|run|repo|release|api|workflow|search)(?:\s+(?:-R\s+\S+\s+|--repo\s+\S+\s+)?([a-z][a-z\-]+))?)"
)
_RELAY_START_RE = re.compile(r"""relay(?:\.py)?['"]?\s+send\b""")
_RELAY_TO_RE = re.compile(r"""--to[=\s]+['"]?([a-z]+:[^\s'"]+)""")
_SENT_OUT_RE = re.compile(r"sent (\d{8}T\d{6}-[0-9a-f]{8}) to (\S+) via")
_JID_OUT_RE = re.compile(r'"id":\s*"(\d{8}T\d{6}-[0-9a-f]{8})"')
_JTO_OUT_RE = re.compile(r'"(session|thread|conversation)":\s*"([A-Za-z0-9_.\-]{8,})"')
_PATCH_RE = re.compile(r"\*\*\* (Add|Update|Delete) File: ([^\n\\]+)")
_SWARM_MSG_RE = re.compile(
    r"\[Message\] timestamp=(\S+) sender=([0-9a-f\-]{36}) priority=\S+ content=(.*?)(?:</SYSTEM_MESSAGE>|$)",
    re.DOTALL,
)
_SWARM_NAME_RE = re.compile(r"^[^\n]{0,200}?`([A-Za-z0-9_.\-]{3,60})`")
_SUB_SENDER_RE = re.compile(r"sender=[0-9a-f\-]{36}/(task-\d+)")


def _clean_title(text: str) -> str:
    t = text or ""
    m = _TAG_NOTE_RE.search(t)
    if m:
        t = m.group(1)
    m2 = _REQ_RE.search(t)
    if m2:
        t = m2.group(1)
    t = _TAG_STRIP_RE.sub(" ", t)
    t = _ANY_TAG_RE.sub(" ", t)
    return " ".join(t.split()).strip()


def _looks_like_prompt(text: str) -> bool:
    t = (text or "").lstrip()
    if not t:
        return False
    if t.startswith(("<relay ", "<note>", "<USER_REQUEST>")) or "## My request" in t:
        return True
    return not t.startswith(("<", "Caveat:", "[Request interrupted", "This session is being continued"))


def _git_actions(cmd: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    for raw_line in re.split(r"[\n;]|&&|\|\|", cmd):
        seg = raw_line.strip()
        if not seg or seg.startswith("#"):
            continue
        m = _GIT_RE.search(seg)
        if m:
            verb = m.group(2)
            sub = m.group(3)
            if verb in ("worktree", "stash", "remote", "submodule") and sub:
                verb = f"{verb} {sub}"
            out.append((verb, _truncate(seg[m.start(1):].strip(), 200)))
            continue
        m2 = _GH_RE.search(seg)
        if m2:
            verb = f"gh {m2.group(2)} {m2.group(3)}" if m2.group(3) else f"gh {m2.group(2)}"
            out.append((verb, _truncate(seg[m2.start(1):].strip(), 200)))
    return out


def _strip_heredoc(b: str) -> str:
    t = b.strip()
    if t.startswith("$(cat <<"):
        body = t.split("\n", 1)[1] if "\n" in t else ""
        body = body.rstrip()
        if body.endswith(")"):
            body = body[:-1].rstrip()
        if "\n" in body:
            body = body.rsplit("\n", 1)[0]
        return body
    return t


def _relay_sends(cmd: str) -> list[dict[str, Any]]:
    if "send" not in cmd or "relay" not in cmd:
        return []
    out: list[dict[str, Any]] = []
    for m in _RELAY_START_RE.finditer(cmd):
        line_start = cmd.rfind("\n", 0, m.start()) + 1
        head = cmd[line_start:m.start()].lstrip()
        head = re.split(r"[;&|]", head)[-1].lstrip()
        ok = (
            not head
            or head.startswith(("python", "env ", "/", "~", '"', "ssh "))
        )
        if not ok or "#" in head or "`" in head or "<<" in cmd[:m.start()]:
            continue
        rest = cmd[m.end():]
        to = None
        subject = None
        body = ""
        b_len = 0
        try:
            tokens = shlex.split(rest)
        except ValueError:
            tokens = None
        if tokens is not None:
            it = iter(tokens)
            for tok in it:
                if tok in ("&&", "||", ";", "|"):
                    break
                if tok == "--to":
                    to = next(it, None)
                elif tok == "--subject":
                    subject = next(it, None)
                elif tok in ("--message", "-m"):
                    val = next(it, None)
                    if val is not None:
                        body = _strip_heredoc(val)
                        b_len = len(body)
                elif tok == "--message-b64":
                    val = next(it, None)
                    if val is not None:
                        b_len = (len(val) * 3) // 4
                        body = "(base64 body)"
                elif tok == "--file":
                    val = next(it, None)
                    if val is not None:
                        body = f"(body in file {val})"
                elif tok.startswith("--to="):
                    to = tok[5:]
                elif tok.startswith("--message="):
                    body = tok[10:]
                    b_len = len(body)
        if not to:
            m_to = _RELAY_TO_RE.search(rest)
            if m_to:
                to = m_to.group(1)
            if not body:
                body = _truncate(rest, 600)
                b_len = len(rest)
        if to and not to.startswith("$") and "<" not in to and _parse_addr(to):
            out.append({"to": to, "subject": subject, "body": body, "bytes": b_len})
    return out


# --------------------------------------------------------------------------- Incremental Collector + Hub

class _AgentAcc:
    def __init__(self, kind: str, aid: str, host: str) -> None:
        self.agent: dict[str, Any] = {
            "addr": _fmt_addr(kind, aid, host),
            "kind": kind,
            "id": aid,
            "host": host,
            "label": None,
            "cwd": None,
            "title": None,
            "focus": None,
            "last_text": None,
            "state": None,
            "alive": None,
            "registered": False,
            "started_ms": 0,
            "last_active_ms": 0,
            "tokens": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
            "cost_usd": None,
            "model": None,
            "files": [],
            "git": [],
            "skills": {},
            "tools": {},
            "subagents": [],
            "transcript": None,
            "parent": None,
            "pool": None,
            "ghost": False,
        }
        self.title_rank = 0
        self.label_rank = 0
        self.pending: dict[str, dict[str, Any]] = {}
        self.seen_usage: set[str] = set()
        self.seen_msgs: set[str] = set()
        self.file_idx: dict[str, int] = {}
        self.stubs: dict[str, dict[str, Any]] = {}
        self.stub_seq = 0
        self.out_msgs: list[dict[str, Any]] = []

    def touch(self, ts: int | None) -> None:
        if ts is None or ts <= 0:
            return
        if self.agent["started_ms"] == 0 or ts < self.agent["started_ms"]:
            self.agent["started_ms"] = ts
        if ts > self.agent["last_active_ms"]:
            self.agent["last_active_ms"] = ts

    def set_title(self, title: str, rank: int) -> None:
        t = _clean_title(title)
        if not t:
            return
        if rank > self.title_rank:
            self.agent["title"] = _truncate(t, 240)
            self.title_rank = rank

    def set_label(self, label: str, rank: int) -> None:
        lbl = (label or "").strip()
        if not lbl or rank < self.label_rank:
            return
        self.agent["label"] = _truncate(lbl, 80)
        self.label_rank = rank

    def set_cwd(self, cwd: str) -> None:
        if cwd:
            self.agent["cwd"] = cwd

    def set_focus(self, focus: str) -> None:
        f = (focus or "").strip()
        if f:
            self.agent["focus"] = _truncate(f, 200)

    def set_last_text(self, text: str) -> None:
        t = (text or "").strip()
        if t:
            self.agent["last_text"] = _truncate(t, 600)

    def tool(self, name: str) -> None:
        if name:
            tools = self.agent["tools"]
            tools[name] = int(tools.get(name, 0)) + 1

    def skill(self, name: str) -> None:
        n = (name or "").strip().lstrip("/")
        if n and len(n) < 80:
            sk = self.agent["skills"]
            sk[n] = int(sk.get(n, 0)) + 1

    def file_touch(self, path: str, op: str, ts: int) -> None:
        p = (path or "").strip().strip('"')
        if not p:
            return
        files = self.agent["files"]
        if p in self.file_idx:
            f = files[self.file_idx[p]]
            f["count"] += 1
            f["at_ms"] = max(f["at_ms"], ts)
            if f["op"] != "created":
                f["op"] = op
        else:
            if len(files) >= MAX_FILES:
                oldest_i = min(range(len(files)), key=lambda idx: files[idx]["at_ms"])
                files.pop(oldest_i)
                self.file_idx = {item["path"]: idx for idx, item in enumerate(files)}
            self.file_idx[p] = len(files)
            files.append({"path": p, "op": op, "count": 1, "at_ms": ts})

    def file_result(self, path: str, created: bool) -> None:
        if created and path in self.file_idx:
            self.agent["files"][self.file_idx[path]]["op"] = "created"

    def tokens_add(self, inp: int = 0, out: int = 0, cr: int = 0, cw: int = 0) -> None:
        tk = self.agent["tokens"]
        tk["input"] += max(0, int(inp))
        tk["output"] += max(0, int(out))
        tk["cache_read"] += max(0, int(cr))
        tk["cache_write"] += max(0, int(cw))

    def subagent(self, sub: dict[str, Any]) -> None:
        subs = self.agent["subagents"]
        for existing in subs:
            if existing["id"] == sub["id"]:
                existing["last_ms"] = max(existing["last_ms"], sub["last_ms"])
                if sub.get("task") and not existing.get("task"):
                    existing["task"] = sub["task"]
                existing["tokens"] = max(existing["tokens"], sub.get("tokens", 0))
                existing["cache_read"] = max(existing["cache_read"], sub.get("cache_read", 0))
                if sub.get("status") == "done":
                    existing["status"] = "done"
                return
        if len(subs) >= MAX_SUBAGENTS:
            subs.pop(0)
        subs.append(sub)

    def subagent_mut(self, sid: str) -> dict[str, Any] | None:
        for existing in self.agent["subagents"]:
            if existing["id"] == sid:
                return existing
        return None

    def command(self, cmd: str, ts: int, call_id: str | None = None) -> str | None:
        for verb, full_cmd in _git_actions(cmd):
            git = self.agent["git"]
            if len(git) >= MAX_GIT:
                git.pop(0)
            git.append({"verb": verb, "command": full_cmd, "at_ms": ts})
        for sk in _SKILL_PATH_RE.findall(cmd):
            self.skill(sk)
        stub_id = None
        for send in _relay_sends(cmd):
            self.stub_seq += 1
            sid = f"send:{self.agent['id']}:{call_id or 'x'}:{self.stub_seq}"
            parsed = _parse_addr(send["to"], self.agent["host"])
            to_addr = _fmt_addr(*parsed) if parsed else send["to"]
            msg = {
                "id": sid,
                "from": self.agent["addr"],
                "to": to_addr,
                "subject": send["subject"],
                "bytes": int(send["bytes"]),
                "body": _truncate(send["body"], MAX_BODY_PREVIEW),
                "sent_ms": ts,
                "source": "sent",
                "replaces": None,
                "from_agent": None,
                "to_agent": None,
            }
            if len(self.stubs) > 500:
                self.stubs.clear()
            self.stubs[sid] = msg
            self.out_msgs.append(msg)
            stub_id = sid
        return stub_id

    def send_result(self, stub_id: str, output: str) -> None:
        m_sent = _SENT_OUT_RE.search(output)
        m_jid = _JID_OUT_RE.search(output)
        if m_sent:
            real_id, to_hint = m_sent.group(1), m_sent.group(2)
        elif m_jid:
            real_id, to_hint = m_jid.group(1), None
        else:
            return
        stub = self.stubs.pop(stub_id, None)
        if not stub:
            return
        to_addr = stub["to"]
        m_jto = _JTO_OUT_RE.search(output)
        if m_jto:
            parsed = _parse_addr(to_addr)
            if parsed:
                to_addr = _fmt_addr(parsed[0], m_jto.group(2), parsed[2])
        elif to_hint:
            p_hint = _parse_addr(to_hint)
            p_orig = _parse_addr(to_addr)
            if p_hint:
                host = p_hint[2] or (p_orig[2] if p_orig else "")
                to_addr = _fmt_addr(p_hint[0], p_hint[1], host)
        self.out_msgs = [m for m in self.out_msgs if m["id"] != stub_id]
        updated = dict(stub, id=real_id, to=to_addr, replaces=stub_id)
        self.out_msgs.append(updated)

    def scan_received(self, text: str, ts: int) -> None:
        if "<agent-relay-message" not in text:
            return
        for m in _RECV_RE.finditer(text):
            mid = m.group(1)
            if mid in self.seen_msgs:
                continue
            self.seen_msgs.add(mid)
            body = m.group(5).strip()
            sent_ms = _parse_ts(m.group(3)) or ts
            self.out_msgs.append(
                {
                    "id": mid,
                    "from": m.group(2),
                    "to": self.agent["addr"],
                    "subject": m.group(4),
                    "bytes": len(body.encode("utf-8", errors="replace")),
                    "body": _truncate(body, MAX_BODY_PREVIEW),
                    "sent_ms": sent_ms,
                    "source": "received",
                    "replaces": None,
                    "from_agent": None,
                    "to_agent": None,
                }
            )


def _walk_strings(val: Any, out: list[str]) -> None:
    if isinstance(val, str):
        out.append(val)
    elif isinstance(val, list):
        for item in val:
            _walk_strings(item, out)
    elif isinstance(val, dict):
        for item in val.values():
            _walk_strings(item, out)


def _claude_tool_use(acc: _AgentAcc, tid: str, name: str, inp: dict[str, Any], ts: int) -> None:
    acc.tool(name)
    if name == "Bash":
        stub = acc.command(str(inp.get("command") or ""), ts, tid)
        if stub:
            acc.pending[tid] = {"type": "send", "stub_id": stub}
    elif name == "Write":
        p = str(inp.get("file_path") or "")
        acc.file_touch(p, "written", ts)
        acc.pending[tid] = {"type": "file", "path": p}
    elif name in ("Edit", "MultiEdit"):
        acc.file_touch(str(inp.get("file_path") or ""), "edited", ts)
    elif name == "NotebookEdit":
        acc.file_touch(str(inp.get("notebook_path") or ""), "edited", ts)
    elif name == "Read":
        for sk in _SKILL_PATH_RE.findall(str(inp.get("file_path") or "")):
            acc.skill(sk)
    elif name == "Skill":
        acc.skill(str(inp.get("skill") or inp.get("command") or ""))
    elif name in ("Agent", "Task"):
        desc = str(inp.get("description") or "")
        prompt = str(inp.get("prompt") or "")
        acc.subagent(
            {
                "id": tid,
                "name": desc or _truncate(prompt, 60),
                "kind": str(inp.get("subagent_type") or "subagent"),
                "task": _truncate(prompt, 600),
                "status": "running",
                "tokens": 0,
                "cache_read": 0,
                "started_ms": ts,
                "last_ms": ts,
            }
        )
        acc.pending[tid] = {"type": "subagent", "id": tid}
    elif name == "TodoWrite":
        todos = inp.get("todos")
        if isinstance(todos, list):
            for item in todos:
                if isinstance(item, dict) and item.get("status") == "in_progress":
                    acc.set_focus(str(item.get("activeForm") or item.get("content") or ""))
                    break
    elif name.endswith("update_progress_checklist"):
        text = str(inp.get("text") or "")
        for ln in text.splitlines():
            if ln.lstrip().startswith("✱"):
                acc.set_focus(ln.lstrip().lstrip("✱").strip())
                break
        if text.splitlines():
            acc.set_title(text.splitlines()[0], 2)
    elif name.endswith("rename_thread"):
        acc.set_title(str(inp.get("title") or ""), 3)


def _claude_tool_result(acc: _AgentAcc, tid: str, out: str) -> None:
    p = acc.pending.pop(tid, None)
    if not p:
        return
    if p["type"] == "file":
        acc.file_result(p["path"], "created successfully" in out or "File created" in out)
    elif p["type"] == "send":
        acc.send_result(p["stub_id"], out)
    elif p["type"] == "subagent":
        sa = acc.subagent_mut(p["id"])
        if sa:
            sa["status"] = "done"


def _parse_claude_line(acc: _AgentAcc, v: dict[str, Any]) -> None:
    ts = _parse_ts(v.get("timestamp"))
    t = ts or acc.agent["last_active_ms"]
    ty = str(v.get("type") or "")
    if isinstance(v.get("cwd"), str):
        acc.set_cwd(v["cwd"])
    if ty == "custom-title" and isinstance(v.get("customTitle"), str):
        acc.set_title(v["customTitle"], 3)
    elif ty == "ai-title" and isinstance(v.get("aiTitle"), str):
        acc.set_title(v["aiTitle"], 2)
    elif ty == "summary" and isinstance(v.get("summary"), str):
        acc.set_title(v["summary"], 2)
    elif ty == "agent-name" and isinstance(v.get("agentName"), str):
        acc.set_label(v["agentName"], 1)
    elif ty == "relocated" and isinstance(v.get("relocatedCwd"), str):
        acc.set_cwd(v["relocatedCwd"])
    elif ty == "pr-link":
        url = str(v.get("prUrl") or "")
        acc.agent["git"].append({"verb": "pr link", "command": url, "at_ms": t})
    elif ty == "cost-state" and isinstance(v.get("totalCostUSD"), (int, float)):
        acc.agent["cost_usd"] = float(v["totalCostUSD"])
    elif ty == "queue-operation" and isinstance(v.get("content"), str):
        acc.scan_received(v["content"], t)
    elif ty in ("attachment", "system"):
        strs: list[str] = []
        _walk_strings(v, strs)
        for s in strs:
            acc.scan_received(s, t)
    elif ty == "user":
        msg = v.get("message")
        if not isinstance(msg, dict):
            return
        content = msg.get("content")
        meta = bool(v.get("isMeta"))
        texts: list[str] = []
        if isinstance(content, str):
            texts.append(content)
        elif isinstance(content, list):
            for it in content:
                if not isinstance(it, dict):
                    continue
                if it.get("type") == "text" and isinstance(it.get("text"), str):
                    texts.append(it["text"])
                elif it.get("type") == "tool_result":
                    _claude_tool_result(acc, str(it.get("tool_use_id") or ""), _content_text(it.get("content")))
        for txt in texts:
            acc.scan_received(txt, t)
            if "<command-name>" in txt:
                cmd_name = txt.split("<command-name>", 1)[1].split("</command-name>", 1)[0]
                acc.skill(cmd_name)
            if not meta and _looks_like_prompt(txt) and "<agent-relay-message" not in txt:
                acc.touch(ts)
                acc.set_title(txt, 1)
    elif ty == "assistant":
        acc.touch(ts)
        msg = v.get("message")
        if not isinstance(msg, dict):
            return
        model = msg.get("model")
        if isinstance(model, str) and not model.startswith("<"):
            acc.agent["model"] = model
        mid = msg.get("id")
        usage = msg.get("usage")
        if isinstance(mid, str) and isinstance(usage, dict) and mid not in acc.seen_usage:
            acc.seen_usage.add(mid)
            acc.tokens_add(
                inp=usage.get("input_tokens", 0),
                out=usage.get("output_tokens", 0),
                cr=usage.get("cache_read_input_tokens", 0),
                cw=usage.get("cache_creation_input_tokens", 0),
            )
        items = msg.get("content")
        if isinstance(items, list):
            for it in items:
                if not isinstance(it, dict):
                    continue
                if it.get("type") == "text" and isinstance(it.get("text"), str):
                    acc.set_last_text(it["text"])
                elif it.get("type") == "tool_use":
                    inp = it.get("input") if isinstance(it.get("input"), dict) else {}
                    _claude_tool_use(acc, str(it.get("id") or ""), str(it.get("name") or "?"), inp, t)


def _parse_claude_subagent_line(acc: _AgentAcc, sub_id: str, v: dict[str, Any]) -> None:
    ts = _parse_ts(v.get("timestamp"))
    t = ts or acc.agent["last_active_ms"]
    ty = str(v.get("type") or "")
    if not acc.subagent_mut(sub_id):
        task = ""
        if ty == "user":
            task = _content_text((v.get("message") or {}).get("content"))
        acc.subagent(
            {
                "id": sub_id,
                "name": _truncate(_clean_title(task), 60) or sub_id,
                "kind": "subagent",
                "task": _truncate(task, 600),
                "status": "running",
                "tokens": 0,
                "cache_read": 0,
                "started_ms": t,
                "last_ms": t,
            }
        )
    if ty == "assistant":
        msg = v.get("message") or {}
        mid = msg.get("id")
        usage = msg.get("usage")
        add_tot = 0
        add_cr = 0
        if isinstance(mid, str) and isinstance(usage, dict) and mid not in acc.seen_usage:
            acc.seen_usage.add(mid)
            i = int(usage.get("input_tokens", 0) or 0)
            o = int(usage.get("output_tokens", 0) or 0)
            cr = int(usage.get("cache_read_input_tokens", 0) or 0)
            cw = int(usage.get("cache_creation_input_tokens", 0) or 0)
            acc.tokens_add(i, o, cr, cw)
            add_tot = i + o + cr + cw
            add_cr = cr
        sa = acc.subagent_mut(sub_id)
        if sa:
            sa["tokens"] += add_tot
            sa["cache_read"] += add_cr
            sa["last_ms"] = max(sa["last_ms"], t)


def _patch_files(acc: _AgentAcc, text: str, ts: int) -> None:
    for m in _PATCH_RE.finditer(text or ""):
        op_raw = m.group(1)
        op = "created" if op_raw == "Add" else ("deleted" if op_raw == "Delete" else "edited")
        acc.file_touch(m.group(2).strip(), op, ts)


def _parse_codex_line(acc: _AgentAcc, v: dict[str, Any]) -> None:
    ts = _parse_ts(v.get("timestamp"))
    t = ts or acc.agent["last_active_ms"]
    ty = str(v.get("type") or "")
    p = v.get("payload") if isinstance(v.get("payload"), dict) else {}
    pty = str(p.get("type") or "")

    if ty == "session_meta":
        acc.touch(ts)
        if isinstance(p.get("cwd"), str):
            acc.set_cwd(p["cwd"])
    elif ty == "turn_context":
        if isinstance(p.get("cwd"), str):
            acc.set_cwd(p["cwd"])
        if isinstance(p.get("model"), str):
            acc.agent["model"] = p["model"]
    elif ty == "event_msg":
        if pty == "thread_settings_applied":
            m = (p.get("thread_settings") or {}).get("model")
            if isinstance(m, str):
                acc.agent["model"] = m
        elif pty == "token_count":
            u = (p.get("info") or {}).get("total_token_usage")
            if isinstance(u, dict):
                inp = int(u.get("input_tokens", 0) or 0)
                cached = int(u.get("cached_input_tokens", 0) or 0)
                out = int(u.get("output_tokens", 0) or 0)
                cw = int(u.get("cache_write_input_tokens", 0) or 0)
                tk = {"input": max(0, inp - cached), "output": out, "cache_read": cached, "cache_write": cw}
                cur_tot = sum(acc.agent["tokens"].values())
                if sum(tk.values()) >= cur_tot:
                    acc.agent["tokens"] = tk
        elif pty == "thread_goal_updated":
            obj = (p.get("goal") or {}).get("objective")
            if isinstance(obj, str):
                acc.set_title(obj, 3)
        elif pty == "thread_name_updated":
            name = p.get("thread_name") or p.get("name")
            if isinstance(name, str):
                acc.set_label(name, 1)
        elif pty == "task_started":
            acc.touch(ts)
        elif pty == "user_message":
            text = str(p.get("message") or "")
            acc.scan_received(text, t)
            if "<agent-relay-message" not in text and (_looks_like_prompt(text) or "## My request" in text):
                acc.touch(ts)
                acc.set_title(text, 1)
        elif pty == "agent_message":
            acc.touch(ts)
            if isinstance(p.get("message"), str):
                acc.set_last_text(p["message"])
        elif pty == "task_complete":
            acc.touch(ts)
            if isinstance(p.get("last_agent_message"), str):
                acc.set_last_text(p["last_agent_message"])
    elif ty == "response_item":
        if pty == "message":
            role = p.get("role")
            text = _content_text(p.get("content"))
            if role == "user":
                acc.scan_received(text, t)
            elif role == "assistant":
                acc.touch(ts)
                acc.set_last_text(text)
        elif pty == "custom_tool_call":
            acc.touch(ts)
            name = str(p.get("name") or "?")
            inp = str(p.get("input") or "")
            call_id = str(p.get("call_id") or "")
            acc.tool(name)
            if name == "exec":
                for m in re.finditer(r'(?:cmd|command)\s*:\s*"((?:[^"\\]|\\.)*)"', inp):
                    cmd_txt = m.group(1).replace("\\n", "\n").replace("\\t", "\t")
                    stub = acc.command(cmd_txt, t, call_id)
                    if stub:
                        acc.pending[call_id] = {"type": "send", "stub_id": stub}
            _patch_files(acc, inp, t)
        elif pty == "function_call":
            acc.touch(ts)
            name = str(p.get("name") or "?")
            call_id = str(p.get("call_id") or "")
            try:
                args = json.loads(p.get("arguments") or "{}")
            except (TypeError, ValueError):
                args = {}
            if not isinstance(args, dict):
                args = {}
            acc.tool(name)
            if name in ("exec_command", "shell", "local_shell"):
                raw_cmd = args.get("cmd") if "cmd" in args else args.get("command")
                if isinstance(raw_cmd, list):
                    cmd_str = " ".join(str(x) for x in raw_cmd)
                else:
                    cmd_str = str(raw_cmd or "")
                stub = acc.command(cmd_str, t, call_id)
                if stub:
                    acc.pending[call_id] = {"type": "send", "stub_id": stub}
            elif name == "apply_patch":
                _patch_files(acc, str(args.get("input") or ""), t)
            elif name == "spawn_agent":
                tname = str(args.get("task_name") or "")
                msg = str(args.get("message") or "")
                sid = tname or call_id
                acc.subagent(
                    {
                        "id": sid,
                        "name": tname or "agent",
                        "kind": "agent",
                        "task": "(task text is encrypted)" if msg.startswith("gAAAA") else _truncate(msg, 600),
                        "status": "running",
                        "tokens": 0,
                        "cache_read": 0,
                        "started_ms": t,
                        "last_ms": t,
                    }
                )
                acc.pending[call_id] = {"type": "subagent", "id": sid}
            elif name == "update_plan" and isinstance(args.get("plan"), list):
                for item in args["plan"]:
                    if isinstance(item, dict) and item.get("status") == "in_progress":
                        acc.set_focus(str(item.get("step") or ""))
                        break
        elif pty in ("custom_tool_call_output", "function_call_output"):
            call_id = str(p.get("call_id") or "")
            pend = acc.pending.pop(call_id, None)
            if pend:
                out = p.get("output") if isinstance(p.get("output"), str) else _content_text(p.get("output"))
                if pend["type"] == "send":
                    acc.send_result(pend["stub_id"], out)
                elif pend["type"] == "subagent":
                    sa = acc.subagent_mut(pend["id"])
                    if sa:
                        sa["last_ms"] = t


def _agy_arg(args: dict[str, Any], keys: tuple[str, ...]) -> str:
    for k in keys:
        if k in args and args[k] is not None:
            v = args[k]
            if isinstance(v, str):
                try:
                    inner = json.loads(v)
                    if isinstance(inner, str):
                        return inner
                except ValueError:
                    pass
                return v
            return json.dumps(v)
    return ""


def _parse_agy_line(acc: _AgentAcc, v: dict[str, Any]) -> None:
    ts = _parse_ts(v.get("created_at"))
    t = ts or acc.agent["last_active_ms"]
    ty = str(v.get("type") or "")
    content = str(v.get("content") or "")
    if ty == "USER_INPUT":
        acc.touch(ts)
        acc.scan_received(content, t)
        if "<agent-relay-message" not in content:
            req = content
            if "<USER_REQUEST>" in content:
                req = content.split("<USER_REQUEST>", 1)[1].split("</USER_REQUEST>", 1)[0]
            acc.set_title(req, 1)
    elif ty == "SYSTEM_MESSAGE":
        acc.scan_received(content, t)
        m_sub = _SUB_SENDER_RE.search(content)
        if m_sub:
            sid = m_sub.group(1)
            done = "finished with result" in content
            acc.subagent(
                {
                    "id": sid,
                    "name": sid,
                    "kind": "task",
                    "task": _truncate(content.split("content=", 1)[1] if "content=" in content else "", 300),
                    "status": "done" if done else "running",
                    "tokens": 0,
                    "cache_read": 0,
                    "started_ms": t,
                    "last_ms": t,
                }
            )
        m_sw = _SWARM_MSG_RE.search(content)
        step = int(v.get("step_index") or 0)
        if m_sw and m_sw.group(2) != acc.agent["id"]:
            sender = m_sw.group(2)
            body = m_sw.group(3).strip()
            sent = _parse_ts(m_sw.group(1)) or t
            from_addr = _fmt_addr("antigravity", sender, acc.agent["host"])
            if step <= 3 and acc.agent["parent"] is None and acc.title_rank < 3:
                acc.agent["parent"] = from_addr
                m_nm = _SWARM_NAME_RE.search(body)
                if m_nm:
                    acc.set_label(m_nm.group(1), 1)
                acc.set_title(body, 2)
            mid = f"swarm:{acc.agent['id']}:{step}"
            if mid not in acc.seen_msgs:
                acc.seen_msgs.add(mid)
                acc.out_msgs.append(
                    {
                        "id": mid,
                        "from": from_addr,
                        "to": acc.agent["addr"],
                        "subject": None,
                        "bytes": len(body.encode("utf-8", errors="replace")),
                        "body": _truncate(body, MAX_BODY_PREVIEW),
                        "sent_ms": sent,
                        "source": "swarm",
                        "replaces": None,
                        "from_agent": None,
                        "to_agent": None,
                    }
                )
    elif ty == "PLANNER_RESPONSE":
        acc.touch(ts)
        acc.tokens_add(
            inp=v.get("input_tokens", 0) or 0,
            out=v.get("output_tokens", 0) or 0,
            cr=v.get("cache_read_tokens", 0) or 0,
        )
        if content:
            acc.set_last_text(content)
        calls = v.get("tool_calls")
        if isinstance(calls, list):
            step = int(v.get("step_index") or 0)
            for idx, c in enumerate(calls):
                if not isinstance(c, dict):
                    continue
                name = str(c.get("name") or "?")
                args = c.get("args") if isinstance(c.get("args"), dict) else {}
                acc.tool(name)
                if name == "run_command":
                    acc.command(_agy_arg(args, ("CommandLine", "command")), t, f"{step}.{idx}")
                elif name in ("write_file", "write_to_file"):
                    acc.file_touch(_agy_arg(args, ("Path", "TargetFile", "path")), "written", t)
                elif name in ("edit_file", "replace_file_content", "multi_replace_file_content"):
                    acc.file_touch(_agy_arg(args, ("Path", "TargetFile", "path")), "edited", t)
                elif name in ("read_file", "view_file"):
                    for sk in _SKILL_PATH_RE.findall(_agy_arg(args, ("Path", "AbsolutePath", "path"))):
                        acc.skill(sk)
                elif name == "invoke_subagent":
                    prompt = _agy_arg(args, ("Prompt", "prompt"))
                    sname = _agy_arg(args, ("Name", "TaskName", "name")) or _truncate(_clean_title(prompt), 50)
                    acc.subagent(
                        {
                            "id": sname,
                            "name": sname,
                            "kind": "subagent",
                            "task": _truncate(prompt, 600),
                            "status": "running",
                            "tokens": 0,
                            "cache_read": 0,
                            "started_ms": t,
                            "last_ms": t,
                        }
                    )
                elif name == "update_focus":
                    acc.set_focus(_agy_arg(args, ("focus",)))
                elif name == "update_steps":
                    try:
                        steps = json.loads(_agy_arg(args, ("steps",)))
                        if isinstance(steps, list):
                            for st in steps:
                                if isinstance(st, dict) and st.get("status") == "in_progress":
                                    acc.set_focus(str(st.get("title") or ""))
                                    break
                    except ValueError:
                        pass
                elif name == "swarm" and _agy_arg(args, ("action",)) == "title":
                    acc.set_title(_agy_arg(args, ("title",)), 2)
                elif name in ("skill", "use_skill"):
                    acc.skill(_agy_arg(args, ("name", "Skill", "skill")))


def _parse_pi_line(acc: _AgentAcc, v: dict[str, Any]) -> None:
    ts = _parse_ts(v.get("timestamp"))
    t = ts or acc.agent["last_active_ms"]
    ty = str(v.get("type") or "")
    if ty == "session":
        acc.touch(ts)
        if isinstance(v.get("cwd"), str):
            acc.set_cwd(v["cwd"])
    elif ty == "model_change" and isinstance(v.get("modelId"), str):
        acc.agent["model"] = v["modelId"]
    elif ty == "message":
        m = v.get("message")
        if not isinstance(m, dict):
            return
        role = m.get("role")
        if role == "user":
            text = _content_text(m.get("content"))
            acc.touch(ts)
            acc.scan_received(text, t)
            if _looks_like_prompt(text):
                acc.set_title(text, 1)
        elif role == "assistant":
            acc.touch(ts)
            u = m.get("usage")
            if isinstance(u, dict):
                acc.tokens_add(
                    inp=u.get("input", 0) or 0,
                    out=u.get("output", 0) or 0,
                    cr=u.get("cacheRead", 0) or 0,
                    cw=u.get("cacheWrite", 0) or 0,
                )
            items = m.get("content")
            if isinstance(items, list):
                for it in items:
                    if not isinstance(it, dict):
                        continue
                    if it.get("type") == "text" and isinstance(it.get("text"), str):
                        acc.set_last_text(it["text"])
                    elif it.get("type") == "toolCall":
                        name = str(it.get("name") or "?")
                        args = it.get("arguments") if isinstance(it.get("arguments"), dict) else {}
                        acc.tool(name)
                        if name == "bash":
                            acc.command(str(args.get("command") or ""), t, str(it.get("id") or ""))
                        elif name == "write":
                            acc.file_touch(str(args.get("path") or ""), "written", t)
                        elif name == "edit":
                            acc.file_touch(str(args.get("path") or ""), "edited", t)
                        elif name == "read":
                            for sk in _SKILL_PATH_RE.findall(str(args.get("path") or "")):
                                acc.skill(sk)


class RelayMapHub:
    """Thread-safe incremental collector and message/pool resolver."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.cfg = _load_config()
        self.self_host = _self_host()
        self.accs: dict[tuple[str, str], _AgentAcc] = {}
        self.agents: dict[str, dict[str, Any]] = {}
        self.messages: list[dict[str, Any]] = []
        self.msg_by_id: dict[str, dict[str, Any]] = {}
        self.files: dict[Path, dict[str, Any]] = {}
        self.inbox_seen: set[Path] = set()
        self.registry_mtime: dict[Path, int] = {}
        self.auto_pools: dict[str, dict[str, Any]] = {}
        self.last_discover = 0.0
        self.last_scan_ms = 0
        self._init_operator()

    def _init_operator(self) -> None:
        op_addr = _fmt_addr("operator", "relay-map", self.self_host)
        self.agents[op_addr] = {
            "addr": op_addr,
            "kind": "operator",
            "id": "relay-map",
            "host": self.self_host,
            "label": "You (Relay Map)",
            "cwd": None,
            "title": "Broadcasts from this dashboard",
            "focus": None,
            "last_text": None,
            "state": "running",
            "alive": True,
            "registered": False,
            "started_ms": _now_ms(),
            "last_active_ms": _now_ms(),
            "tokens": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
            "cost_usd": None,
            "model": None,
            "files": [],
            "git": [],
            "skills": {},
            "tools": {},
            "subagents": [],
            "transcript": None,
            "parent": None,
            "pool": None,
            "ghost": False,
        }

    def _acc(self, kind: str, aid: str) -> _AgentAcc:
        key = (kind, aid)
        acc = self.accs.get(key)
        if acc is None:
            acc = _AgentAcc(kind, aid, self.self_host)
            self.accs[key] = acc
        return acc

    def _configured_pools(self) -> list[dict[str, Any]]:
        out: list[dict[str, Any]] = []
        for p in self.cfg.get("pools") or []:
            if not isinstance(p, dict) or not p.get("name"):
                continue
            out.append(
                {
                    "name": str(p["name"]),
                    "color": str(p.get("color") or "#9aa7ff"),
                    "description": str(p.get("description") or ""),
                    "isolated": bool(p.get("isolated", True)),
                    "auto": False,
                    "host": None,
                    "project": None,
                }
            )
        return out

    def _pool_for_cfg(self, a: dict[str, Any]) -> str | None:
        hay = f"{(a.get('cwd') or '').lower()} {(a.get('label') or '').lower()}"
        title = (a.get("title") or "").lower()
        kind = a.get("kind") or ""
        for p in self.cfg.get("pools") or []:
            if not isinstance(p, dict) or not p.get("name"):
                continue
            for x in p.get("agents") or []:
                xs = str(x)
                if xs == a["addr"] or (len(xs) >= 6 and str(a["id"]).startswith(xs)):
                    return str(p["name"])
        for p in self.cfg.get("pools") or []:
            if not isinstance(p, dict) or not p.get("name"):
                continue
            kinds = [str(k) for k in (p.get("kinds") or [])]
            hosts = [str(h) for h in (p.get("hosts") or [])]
            if kinds and kind not in kinds:
                continue
            if hosts and a["host"] not in hosts:
                continue
            pats = [str(m).lower() for m in (p.get("match") or [])]
            mt = [str(m).lower() for m in (p.get("match_title") or [])]
            if any(m in hay for m in pats) or any(m in title for m in mt):
                return str(p["name"])
        return None

    def _assign_pool(self, a: dict[str, Any]) -> str | None:
        hit = self._pool_for_cfg(a)
        if hit:
            return hit
        parent_addr = a.get("parent")
        if parent_addr and parent_addr in self.agents:
            pp = self.agents[parent_addr].get("pool")
            if pp:
                return pp
        if not self.cfg.get("auto_pools", True) or a.get("kind") == "operator" or not a.get("host"):
            return None
        proj = "seen in messages" if a.get("ghost") else _project_of(a.get("cwd") or "")
        name = f"{a['host']} · {proj}"
        if name not in self.auto_pools:
            self.auto_pools[name] = {
                "name": name,
                "color": _host_color(a["host"]),
                "description": f"Agents in {proj} on {a['host']}",
                "isolated": False,
                "auto": True,
                "host": a["host"],
                "project": proj,
            }
        return name

    def _consider(self, path: Path, kind: str, aid: str, sub_id: str | None, cutoff_s: float) -> None:
        if path in self.files:
            return
        try:
            st = path.stat()
        except OSError:
            return
        if st.st_mtime >= cutoff_s:
            self.files[path] = {"kind": kind, "id": aid, "sub_id": sub_id, "offset": 0}

    def _discover(self, cutoff_s: float) -> None:
        # Claude Code
        c_dir = _claude_projects_dir()
        if c_dir.is_dir():
            for proj in c_dir.iterdir():
                if not proj.is_dir():
                    continue
                for entry in proj.iterdir():
                    if entry.suffix == ".jsonl" and entry.is_file():
                        self._consider(entry, "claude", entry.stem, None, cutoff_s)
                    elif entry.is_dir():
                        sub_dir = entry / "subagents"
                        if sub_dir.is_dir():
                            for sub in sub_dir.iterdir():
                                if sub.suffix == ".jsonl" and sub.is_file():
                                    aid = sub.stem[6:] if sub.stem.startswith("agent-") else sub.stem
                                    self._consider(sub, "claude_sub", entry.name, aid, cutoff_s)

        # Codex
        codex_dir = _codex_home() / "sessions"
        if codex_dir.is_dir():
            stack = [(codex_dir, 0)]
            while stack:
                d, depth = stack.pop()
                try:
                    entries = list(d.iterdir())
                except OSError:
                    continue
                for e in entries:
                    if e.is_dir() and depth < 4:
                        try:
                            if depth < 3 or e.stat().st_mtime >= cutoff_s:
                                stack.append((e, depth + 1))
                        except OSError:
                            pass
                    elif e.suffix == ".jsonl" and e.is_file() and len(e.stem) >= 36:
                        self._consider(e, "codex", e.stem[-36:], None, cutoff_s)

        # Antigravity
        gem = _gemini_home()
        for prod in ("antigravity", "antigravity-cli", "antigravity-ide"):
            brain = gem / prod / "brain"
            if not brain.is_dir():
                continue
            for conv in brain.iterdir():
                if not conv.is_dir():
                    continue
                logs = conv / ".system_generated" / "logs"
                t = logs / "transcript.jsonl"
                path = t if t.is_file() else (logs / "transcript_full.jsonl")
                if path.is_file():
                    self._consider(path, "antigravity", conv.name, None, cutoff_s)

        # Pi
        pi_dir = _home_dir() / ".pi" / "agent" / "sessions"
        if pi_dir.is_dir():
            for d in pi_dir.iterdir():
                if not d.is_dir():
                    continue
                for f in d.iterdir():
                    if f.suffix == ".jsonl" and f.is_file():
                        pid = f.stem.rsplit("_", 1)[-1]
                        self._consider(f, "pi", pid, None, cutoff_s)

    def _registry(self, cutoff_ms: int) -> None:
        sdir = _relay_home() / "sessions"
        if not sdir.is_dir():
            return
        for p in sdir.iterdir():
            if p.suffix != ".json" or not p.is_file():
                continue
            try:
                st = p.stat()
            except OSError:
                continue
            try:
                rec = json.loads(p.read_text(encoding="utf-8", errors="replace"))
            except (OSError, ValueError):
                continue
            if not isinstance(rec, dict):
                continue
            kind = rec.get("agent")
            aid = rec.get("id")
            if kind not in VALID_KINDS or not isinstance(aid, str) or not aid:
                continue
            alive = _pid_alive(rec.get("pid")) if rec.get("pid") is not None else None
            mtime_ms = int(st.st_mtime * 1000)
            known = (kind, aid) in self.accs
            if not known and mtime_ms < cutoff_ms and alive is not True:
                continue
            tp = rec.get("transcript_path")
            if isinstance(tp, str) and tp:
                tp_path = Path(tp)
                if tp_path.is_file() and tp_path not in self.files and kind in ("claude", "antigravity"):
                    if alive is True or mtime_ms >= cutoff_ms:
                        self.files[tp_path] = {"kind": kind, "id": aid, "sub_id": None, "offset": 0}
            acc = self._acc(kind, aid)
            acc.agent["alive"] = alive
            if tp and not acc.agent["transcript"] and Path(str(tp)).is_file():
                acc.agent["transcript"] = str(tp)
            if self.registry_mtime.get(p) == st.st_mtime_ns:
                continue
            self.registry_mtime[p] = st.st_mtime_ns
            acc.agent["registered"] = True
            if isinstance(rec.get("cwd"), str):
                acc.set_cwd(rec["cwd"])
            if isinstance(rec.get("label"), str):
                acc.set_label(rec["label"], 2)
            if isinstance(rec.get("state"), str):
                acc.agent["state"] = rec["state"]
            if mtime_ms > acc.agent["last_active_ms"]:
                acc.agent["last_active_ms"] = mtime_ms

    def _tail(self) -> None:
        for path in list(self.files.keys()):
            meta = self.files[path]
            try:
                size = path.stat().st_size
            except OSError:
                self.files.pop(path, None)
                continue
            offset = meta["offset"]
            if size == offset:
                continue
            start = 0 if size < offset else offset
            try:
                with open(path, "rb") as fh:
                    fh.seek(start)
                    data = fh.read()
            except OSError:
                continue
            if not data:
                continue
            last_nl = data.rfind(b"\n")
            if last_nl < 0:
                continue
            chunk = data[: last_nl + 1]
            meta["offset"] = start + len(chunk)
            kind = meta["kind"]
            aid = meta["id"]
            acc_kind = "claude" if kind == "claude_sub" else kind
            acc = self._acc(acc_kind, aid)
            if acc.agent["transcript"] is None and kind != "claude_sub":
                acc.agent["transcript"] = str(path)
            for raw_line in chunk.splitlines():
                if not raw_line.strip():
                    continue
                head = raw_line[:200]
                if kind in ("claude", "claude_sub") and (
                    b'"type":"api-request' in head or b'"type":"file-history' in head or b'"type":"data-flow' in head
                ):
                    continue
                if kind == "codex" and (
                    b'"type":"compacted"' in head
                    or b'"type":"world_state"' in head
                    or b'"type":"reasoning"' in head
                    or b'"type":"token_usage_record"' in head
                ):
                    continue
                if kind == "antigravity" and (b'"type":"GENERIC"' in head or b'"type": "GENERIC"' in head):
                    continue
                try:
                    v = json.loads(raw_line.decode("utf-8", errors="replace"))
                except ValueError:
                    continue
                if not isinstance(v, dict):
                    continue
                if kind == "claude":
                    _parse_claude_line(acc, v)
                elif kind == "claude_sub":
                    _parse_claude_subagent_line(acc, meta["sub_id"], v)
                elif kind == "codex":
                    _parse_codex_line(acc, v)
                elif kind == "antigravity":
                    _parse_agy_line(acc, v)
                elif kind == "pi":
                    _parse_pi_line(acc, v)

    def _inbox(self, cutoff_ms: int, cutoff_s: float) -> None:
        idir = _relay_home() / "inbox"
        if not idir.is_dir():
            return
        for sdir in idir.iterdir():
            if not sdir.is_dir():
                continue
            sid = sdir.name
            candidates = list(sdir.iterdir())
            deliv = sdir / "delivered"
            if deliv.is_dir():
                candidates.extend(deliv.iterdir())
            for f in candidates:
                if f.suffix != ".json" or not f.is_file() or f in self.inbox_seen:
                    continue
                self.inbox_seen.add(f)
                try:
                    if f.stat().st_mtime < cutoff_s:
                        continue
                    env = json.loads(f.read_text(encoding="utf-8", errors="replace"))
                except (OSError, ValueError):
                    continue
                if not isinstance(env, dict):
                    continue
                sent_ms = int(env.get("sent_at_ms") or 0)
                if sent_ms < cutoff_ms:
                    continue
                body = str(env.get("body") or "")
                b_len = int(env.get("body_chars") or len(body.encode("utf-8", errors="replace")))
                parsed_to = _parse_addr(str(env.get("to") or ""))
                to_kind = parsed_to[0] if parsed_to else "claude"
                self.add_message(
                    {
                        "id": str(env.get("id") or ""),
                        "from": str(env.get("from") or ""),
                        "to": _fmt_addr(to_kind, sid, self.self_host),
                        "subject": env.get("subject"),
                        "body": _truncate(body, MAX_BODY_PREVIEW),
                        "bytes": b_len,
                        "sent_ms": sent_ms,
                        "source": "inbox",
                        "replaces": None,
                        "from_agent": None,
                        "to_agent": None,
                    }
                )

    def _resolve(self, text: str, default_host: str) -> str | None:
        parsed = _parse_addr(text, default_host)
        if not parsed:
            return None
        kind, ref, host = parsed
        full = _fmt_addr(kind, ref, host)
        if full in self.agents and not self.agents[full].get("ghost"):
            return full
        cands = [
            a
            for a in self.agents.values()
            if not a.get("ghost") and a["host"] == host and a.get("kind") == kind
        ]
        if not cands:
            return None
        for pred in (
            lambda a: a["id"] == ref,
            lambda a: len(ref) >= 6 and str(a["id"]).startswith(ref),
            lambda a: a.get("label") == ref,
            lambda a: bool(a.get("cwd")) and Path(a["cwd"]).name == ref,
        ):
            hits = [a for a in cands if pred(a)]
            if hits:
                best = max(hits, key=lambda a: int(a.get("last_active_ms") or 0))
                return best["addr"]
        return None

    def _ensure_ghost(self, text: str, default_host: str) -> str:
        parsed = _parse_addr(text, default_host)
        if not parsed:
            return text
        kind, ref, host = parsed
        key = _fmt_addr(kind, ref, host)
        if key not in self.agents:
            g = {
                "addr": key,
                "kind": kind if kind in VALID_KINDS else None,
                "id": ref,
                "host": host,
                "label": ref[:8] if len(ref) > 12 else ref,
                "cwd": None,
                "title": "Seen only in relay messages (its computer is not monitored, or the session has ended)",
                "focus": None,
                "last_text": None,
                "state": None,
                "alive": None,
                "registered": False,
                "started_ms": 0,
                "last_active_ms": 0,
                "tokens": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
                "cost_usd": None,
                "model": None,
                "files": [],
                "git": [],
                "skills": {},
                "tools": {},
                "subagents": [],
                "transcript": None,
                "parent": None,
                "pool": None,
                "ghost": True,
            }
            g["pool"] = self._assign_pool(g)
            self.agents[key] = g
        return key

    def _endpoints(self, m: dict[str, Any]) -> None:
        parsed_from = _parse_addr(m.get("from") or "")
        from_host = (parsed_from[2] if parsed_from and parsed_from[2] else self.self_host)
        m["from_agent"] = self._resolve(m.get("from") or "", from_host) or self._ensure_ghost(m.get("from") or "", from_host)
        m["to_agent"] = self._resolve(m.get("to") or "", from_host) or self._ensure_ghost(m.get("to") or "", from_host)

    def add_message(self, m: dict[str, Any]) -> None:
        mid = m.get("id") or ""
        if not mid:
            return
        self._endpoints(m)
        stub = m.get("replaces")
        if stub and stub in self.msg_by_id:
            self.msg_by_id.pop(stub, None)
            self.messages = [x for x in self.messages if x["id"] != stub]
        if m.get("source") != "sent" and not mid.startswith("send:"):
            for idx, x in enumerate(self.messages):
                if (
                    x["id"].startswith("send:")
                    and x["from_agent"] == m["from_agent"]
                    and x["to_agent"] == m["to_agent"]
                    and abs(x["sent_ms"] - m["sent_ms"]) < STUB_WINDOW_MS
                ):
                    old = self.messages.pop(idx)
                    self.msg_by_id.pop(old["id"], None)
                    m["replaces"] = old["id"]
                    break
        existing = self.msg_by_id.get(mid)
        if existing:
            if (len(m["body"]) > len(existing["body"]) and m["source"] != "sent") or (
                existing["source"] == "sent" and m["source"] != "sent"
            ):
                existing["body"] = m["body"]
                existing["bytes"] = max(existing["bytes"], m["bytes"])
            if not existing.get("subject"):
                existing["subject"] = m.get("subject")
            if m["source"] in ("inbox", "received"):
                existing["to_agent"] = m["to_agent"]
                existing["from_agent"] = m["from_agent"]
                existing["source"] = m["source"]
            return
        self.msg_by_id[mid] = m
        self.messages.append(m)
        if len(self.messages) > MAX_MESSAGES:
            self.messages.sort(key=lambda x: x["sent_ms"])
            dropped = self.messages[:-MAX_MESSAGES]
            self.messages = self.messages[-MAX_MESSAGES:]
            for d in dropped:
                self.msg_by_id.pop(d["id"], None)

    def scan(self, lookback_hours: float = 24.0) -> None:
        with self.lock:
            now_s = time.time()
            now_ms = int(now_s * 1000)
            lookback_ms = int(max(1.0, float(lookback_hours)) * 3_600_000)
            cutoff_ms = now_ms - lookback_ms
            cutoff_s = now_s - (lookback_ms / 1000.0)

            if now_s - self.last_discover > 5.0:
                self._discover(cutoff_s)
                self.last_discover = now_s
            self._registry(cutoff_ms)
            self._tail()
            self._inbox(cutoff_ms, cutoff_s)

            for acc in self.accs.values():
                for m in acc.out_msgs:
                    if m["sent_ms"] >= cutoff_ms:
                        self.add_message(m)
                acc.out_msgs.clear()
                a = acc.agent
                if a["last_active_ms"] >= cutoff_ms or a.get("alive") is True:
                    snap_a = dict(a)
                    snap_a["pool"] = self._assign_pool(snap_a)
                    self.agents[snap_a["addr"]] = snap_a

            # Re-resolve any ghost endpoints if real agents just appeared.
            for m in self.messages:
                fa = self.agents.get(m.get("from_agent") or "")
                ta = self.agents.get(m.get("to_agent") or "")
                if (fa is None or fa.get("ghost")) or (ta is None or ta.get("ghost")):
                    parsed_from = _parse_addr(m.get("from") or "")
                    fh = (parsed_from[2] if parsed_from and parsed_from[2] else self.self_host)
                    r_from = self._resolve(m.get("from") or "", fh)
                    r_to = self._resolve(m.get("to") or "", fh)
                    if r_from:
                        m["from_agent"] = r_from
                    if r_to:
                        m["to_agent"] = r_to

            self.messages.sort(key=lambda x: x["sent_ms"])
            self.last_scan_ms = now_ms

    def snapshot(self, lookback_hours: float = 24.0) -> dict[str, Any]:
        self.scan(lookback_hours)
        with self.lock:
            now_ms = _now_ms()
            cutoff_ms = now_ms - int(max(1.0, float(lookback_hours)) * 3_600_000)
            agents = [
                a
                for a in self.agents.values()
                if a["kind"] == "operator" or a.get("alive") is True or a.get("last_active_ms", 0) >= cutoff_ms or a.get("ghost")
            ]
            msgs = [m for m in self.messages if m["sent_ms"] >= cutoff_ms][-MAX_MESSAGES:]
            pools = self._configured_pools() + list(self.auto_pools.values())
            hosts = [
                {
                    "name": self.self_host,
                    "via": "local",
                    "state": "live",
                    "detail": f"live ({sum(1 for a in agents if not a.get('ghost') and a['kind'] != 'operator')} agents)",
                    "agents": sum(1 for a in agents if not a.get("ghost") and a["host"] == self.self_host),
                }
            ]
            snap = {
                "now_ms": now_ms,
                "self_host": self.self_host,
                "agents": agents,
                "messages": msgs,
                "pools": pools,
                "hosts": hosts,
            }
            # Enforce size cap safely under Antigravity's 1 MiB trajectory step cap.
            encoded = json.dumps(snap, separators=(",", ":")).encode("utf-8")
            if len(encoded) > MAX_STRUCTURED_BYTES and len(snap["messages"]) > 150:
                snap["messages"] = snap["messages"][-250:]
                encoded = json.dumps(snap, separators=(",", ":")).encode("utf-8")
            if len(encoded) > MAX_STRUCTURED_BYTES:
                for a in snap["agents"]:
                    a["files"] = a["files"][-30:]
                    a["git"] = a["git"][-20:]
                    a["subagents"] = a["subagents"][-20:]
            return snap


_HUB_LOCK = threading.Lock()
_HUB: RelayMapHub | None = None


def _get_hub() -> RelayMapHub:
    global _HUB
    with _HUB_LOCK:
        if _HUB is None:
            _HUB = RelayMapHub()
        return _HUB


# --------------------------------------------------------------------------- Preferences

DEFAULT_PREFS: dict[str, Any] = {
    "pins": [],
    "levels": ["host", "project", "directory", "task"],
    "color_by": "kind",
    "window_hours": 24,
    "show_unpooled": True,
    "show_swarm": True,
    "expand_swarms": False,
    "notify": False,
}


def _prefs_path(data_dir: Path) -> Path:
    return data_dir / "relay_map_prefs.json"


def _load_prefs(data_dir: Path) -> dict[str, Any]:
    with _PREFS_LOCK:
        p = _prefs_path(data_dir)
        out = dict(DEFAULT_PREFS)
        if p.is_file():
            try:
                raw = json.loads(p.read_text(encoding="utf-8"))
                if isinstance(raw, dict):
                    out.update({k: v for k, v in raw.items() if k in DEFAULT_PREFS})
            except (OSError, ValueError):
                pass
        return out


def _save_prefs(data_dir: Path, patch: dict[str, Any]) -> dict[str, Any]:
    with _PREFS_LOCK:
        data_dir.mkdir(parents=True, exist_ok=True)
        p = _prefs_path(data_dir)
        cur = dict(DEFAULT_PREFS)
        if p.is_file():
            try:
                raw = json.loads(p.read_text(encoding="utf-8"))
                if isinstance(raw, dict):
                    cur.update({k: v for k, v in raw.items() if k in DEFAULT_PREFS})
            except (OSError, ValueError):
                pass
        for k, v in (patch or {}).items():
            if k in DEFAULT_PREFS:
                cur[k] = v
        fd, tmp = tempfile.mkstemp(dir=str(data_dir), prefix=".prefs.", suffix=".tmp")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as fh:
                os.chmod(tmp, 0o600)
                fh.write(json.dumps(cur, indent=2))
            os.replace(tmp, p)
        except Exception:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise
        return cur


# --------------------------------------------------------------------------- CLI bridge & Transcript viewer

def _run_relay_cli(argv: list[str], timeout: float = 120.0) -> Any:
    cmd = [sys.executable, "-B", str(_relay_py()), *argv]
    proc = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=timeout)
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or f"exit {proc.returncode}").strip()
        raise RuntimeError(detail)
    out = proc.stdout.strip()
    if not out:
        return {}
    try:
        return json.loads(out)
    except ValueError:
        return {"raw": out}


def _flatten_transcript(agent: dict[str, Any], raw_bytes: bytes, total_len: int, source: str, notes: list[str], max_entries: int) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    skipped = 0
    lines = raw_bytes.splitlines()
    if len(raw_bytes) < total_len and lines:
        lines = lines[1:]
    kind = agent.get("kind")

    def push(t: int, role: str, name: str | None, text: str) -> None:
        tx = (text or "").strip()
        if not tx and not name:
            return
        if len(tx) > 6000:
            tx = tx[:6000] + f"\n… ({len(tx) - 6000} more characters)"
        entries.append({"t": t, "role": role, "name": name, "text": tx})

    def pretty(val: Any) -> str:
        if isinstance(val, str):
            return val
        if val is None:
            return ""
        return json.dumps(val, indent=2)

    for raw_line in lines:
        if not raw_line.strip():
            continue
        try:
            v = json.loads(raw_line.decode("utf-8", errors="replace"))
        except ValueError:
            skipped += 1
            continue
        if not isinstance(v, dict):
            skipped += 1
            continue
        before = len(entries)
        if kind == "claude":
            t = _parse_ts(v.get("timestamp")) or 0
            ty = v.get("type")
            if ty in ("user", "assistant"):
                c = (v.get("message") or {}).get("content")
                if isinstance(c, str):
                    push(t, ty, None, c)
                elif isinstance(c, list):
                    for it in c:
                        if not isinstance(it, dict):
                            continue
                        it_t = it.get("type")
                        if it_t == "text":
                            push(t, ty, None, str(it.get("text") or ""))
                        elif it_t == "thinking":
                            push(t, "thinking", None, str(it.get("thinking") or ""))
                        elif it_t == "tool_use":
                            push(t, "tool", str(it.get("name") or "tool"), pretty(it.get("input")))
                        elif it_t == "tool_result":
                            push(t, "result", "result", _content_text(it.get("content")) or pretty(it.get("content")))
            elif ty == "system":
                push(t, "system", None, str(v.get("content") or ""))
        elif kind == "codex":
            t = _parse_ts(v.get("timestamp")) or 0
            p = v.get("payload") if isinstance(v.get("payload"), dict) else {}
            ty, pty = v.get("type"), p.get("type")
            if ty == "response_item" and pty == "message":
                role = "user" if p.get("role") == "user" else "assistant"
                push(t, role, None, _content_text(p.get("content")))
            elif ty == "response_item" and pty in ("function_call", "custom_tool_call"):
                push(t, "tool", str(p.get("name") or "tool"), pretty(p.get("arguments") or p.get("input")))
            elif ty == "response_item" and pty in ("function_call_output", "custom_tool_call_output"):
                push(t, "result", "result", pretty(p.get("output")))
            elif ty == "event_msg" and pty in ("user_message", "agent_message"):
                role = "user" if pty == "user_message" else "assistant"
                push(t, role, None, str(p.get("message") or ""))
        elif kind == "antigravity":
            t = _parse_ts(v.get("created_at")) or 0
            ty = v.get("type")
            if ty == "USER_INPUT":
                push(t, "user", None, str(v.get("content") or ""))
            elif ty == "SYSTEM_MESSAGE":
                push(t, "system", None, str(v.get("content") or ""))
            elif ty == "PLANNER_RESPONSE":
                if v.get("thought"):
                    push(t, "thinking", None, str(v["thought"]))
                if v.get("content"):
                    push(t, "assistant", None, str(v["content"]))
                for c in v.get("tool_calls") or []:
                    if isinstance(c, dict):
                        push(t, "tool", str(c.get("name") or "tool"), pretty(c.get("args")))
        elif kind == "pi":
            t = _parse_ts(v.get("timestamp")) or 0
            if v.get("type") == "message" and isinstance(v.get("message"), dict):
                m = v["message"]
                role = "user" if m.get("role") == "user" else "assistant"
                c = m.get("content")
                if isinstance(c, str):
                    push(t, role, None, c)
                elif isinstance(c, list):
                    for it in c:
                        if not isinstance(it, dict):
                            continue
                        if it.get("type") == "text":
                            push(t, role, None, str(it.get("text") or ""))
                        elif it.get("type") == "toolCall":
                            push(t, "tool", str(it.get("name") or "tool"), pretty(it.get("arguments")))
        if len(entries) == before:
            skipped += 1

    if max_entries > 0 and len(entries) > max_entries:
        entries = entries[-max_entries:]
    return {
        "addr": agent["addr"],
        "host": agent["host"],
        "path": agent.get("transcript"),
        "source": source,
        "bytes": total_len,
        "shown_bytes": len(raw_bytes),
        "skipped": skipped,
        "entries": entries,
        "notes": notes,
    }


# --------------------------------------------------------------------------- Tool Implementations

def tool_relay_map(data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    lookback = float(args.get("lookback_hours") or 24.0)
    hub = _get_hub()
    snap = hub.snapshot(lookback)
    snap["prefs"] = _load_prefs(data_dir)
    snap["initial_view"] = {
        "filter": str(args.get("filter") or ""),
        "focus_agent": str(args.get("focus_agent") or ""),
        "tab": str(args.get("tab") or ""),
    }
    n_agents = sum(1 for a in snap["agents"] if not a.get("ghost") and a.get("kind") != "operator")
    n_msgs = len(snap["messages"])
    summary = f"Relay Map — {n_agents} agents | {n_msgs} messages in last {lookback:g}h | host {snap['self_host']}"
    return {"content": [{"type": "text", "text": summary}], "structuredContent": snap}


def tool_relay_map_poll(data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    lookback = float(args.get("lookback_hours") or 24.0)
    hub = _get_hub()
    snap = hub.snapshot(lookback)
    snap["prefs"] = _load_prefs(data_dir)
    return {"content": [{"type": "text", "text": f"Polled {len(snap['agents'])} agents"}], "structuredContent": snap}


def tool_relay_map_prefs(data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    patch = args.get("patch")
    if isinstance(patch, dict) and patch:
        prefs = _save_prefs(data_dir, patch)
    else:
        prefs = _load_prefs(data_dir)
    return {"content": [{"type": "text", "text": json.dumps(prefs)}], "structuredContent": {"prefs": prefs}}


def tool_relay_sessions(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    argv = ["sessions", "--json"]
    if args.get("host"):
        argv.extend(["--host", str(args["host"])])
    if args.get("all"):
        argv.append("--all")
    data = _run_relay_cli(argv, timeout=60.0)
    return {"content": [{"type": "text", "text": json.dumps(data, indent=2)}], "structuredContent": data if isinstance(data, dict) else {"sessions": data}}


def tool_relay_send(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    to = str(args["to"])
    message = str(args["message"])
    argv = ["send", "--to", to, "--message", message, "--json"]
    if args.get("subject"):
        argv.extend(["--subject", str(args["subject"])])
    if args.get("from_addr"):
        argv.extend(["--from", str(args["from_addr"])])
    if args.get("wait"):
        argv.append("--wait")
        if args.get("timeout") is not None:
            argv.extend(["--timeout", str(args["timeout"])])
    data = _run_relay_cli(argv, timeout=float(args.get("timeout") or 180) + 15.0)
    return {"content": [{"type": "text", "text": json.dumps(data, indent=2)}], "structuredContent": data if isinstance(data, dict) else {"result": data}}


def tool_relay_inbox(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    argv = ["inbox", "--json"]
    if args.get("session"):
        argv.extend(["--session", str(args["session"])])
    if args.get("peek", True):
        argv.append("--peek")
    data = _run_relay_cli(argv, timeout=30.0)
    out = {"messages": data} if isinstance(data, list) else (data if isinstance(data, dict) else {"result": data})
    return {"content": [{"type": "text", "text": json.dumps(out, indent=2)}], "structuredContent": out}


def tool_relay_read_reply(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    argv = ["read", str(args["target"]), "--json"]
    if args.get("since_ms"):
        argv.extend(["--since-ms", str(int(args["since_ms"]))])
    if args.get("limit"):
        argv.extend(["--limit", str(int(args["limit"]))])
    data = _run_relay_cli(argv, timeout=60.0)
    return {"content": [{"type": "text", "text": json.dumps(data, indent=2)}], "structuredContent": data if isinstance(data, dict) else {"result": data}}


def tool_relay_broadcast(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    hub = _get_hub()
    hub.scan(24.0)
    scope = str(args.get("scope") or "all")
    addrs_arg = [str(x) for x in (args.get("addrs") or [])]
    subject = str(args.get("subject") or "")
    body = str(args.get("body") or "")
    dry_run = bool(args.get("dry_run", True))
    include_stale = bool(args.get("include_stale", False))

    with hub.lock:
        now_ms = _now_ms()
        if scope == "all":
            consider = list(hub.agents.values())
            scope_text = "every live agent"
            explicit = False
        elif scope.startswith("pool:"):
            pname = scope.split("pool:", 1)[1]
            consider = [a for a in hub.agents.values() if a.get("pool") == pname]
            scope_text = f"every agent in the {pname} pool"
            explicit = False
        elif scope.startswith("agent:"):
            addr = scope.split("agent:", 1)[1]
            consider = [hub.agents[addr]] if addr in hub.agents else []
            scope_text = "1 selected agent"
            explicit = True
        elif addrs_arg:
            consider = [hub.agents[x] for x in addrs_arg if x in hub.agents]
            scope_text = f"{len(consider)} selected agent(s)"
            explicit = True
        else:
            consider = []
            scope_text = scope
            explicit = False

        targets: list[str] = []
        skipped: list[str] = []
        for a in consider:
            kind = a.get("kind") or "operator"
            if kind == "operator":
                continue
            live = a.get("alive") is True or (now_ms - int(a.get("last_active_ms") or 0)) < LIVE_WINDOW_MS
            if a.get("ghost"):
                skipped.append(f"{a['addr']} (seen only in messages)")
            elif kind not in RELAY_TARGET_KINDS:
                skipped.append(f"{a['addr']} ({kind} is not an agent-relay target)")
            elif not explicit and not include_stale and not live:
                skipped.append(f"{a['addr']} (idle more than 2 h)")
            else:
                targets.append(a["addr"])
        targets.sort()
        skipped.sort()
        self_host = hub.self_host

    results: list[dict[str, Any]] = []
    if not dry_run and targets and body.strip():
        from_addr = _fmt_addr("operator", "relay-map", self_host)
        full = (
            f"{body}\n\n"
            f"(Broadcast from the user's Relay Map dashboard to {scope_text}. "
            "It has no reply address; report back through your normal channel.)"
        )

        def _send_one(to_addr: str) -> dict[str, Any]:
            argv = ["send", "--to", to_addr, "--from", from_addr, "--message", full, "--json"]
            if subject:
                argv.extend(["--subject", subject])
            sent_ms = _now_ms()
            try:
                res = _run_relay_cli(argv, timeout=150.0)
                detail = str(res.get("delivery") or "sent") if isinstance(res, dict) else "sent"
                mid = str(res.get("id") or "") if isinstance(res, dict) else ""
                with hub.lock:
                    hub.add_message(
                        {
                            "id": mid or f"broadcast:{sent_ms}:{to_addr}",
                            "from": from_addr,
                            "to": to_addr,
                            "subject": subject or None,
                            "bytes": len(full.encode("utf-8", errors="replace")),
                            "body": _truncate(full, MAX_BODY_PREVIEW),
                            "sent_ms": sent_ms,
                            "source": "broadcast",
                            "replaces": None,
                            "from_agent": None,
                            "to_agent": None,
                        }
                    )
                return {"addr": to_addr, "ok": True, "detail": detail}
            except Exception as exc:
                return {"addr": to_addr, "ok": False, "detail": str(exc)[:240]}

        with ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(_send_one, targets))

    out = {"targets": targets, "skipped": skipped, "dry_run": dry_run, "results": results}
    return {"content": [{"type": "text", "text": json.dumps(out, indent=2)}], "structuredContent": out}


def tool_relay_transcript(_data_dir: Path, args: dict[str, Any]) -> dict[str, Any]:
    addr = str(args["addr"])
    max_entries = int(args.get("max_entries") or 600)
    raw_mode = bool(args.get("raw", False))
    hub = _get_hub()
    hub.scan(72.0)
    with hub.lock:
        agent = hub.agents.get(addr)
        if not agent:
            raise ValueError(f"Unknown agent {addr!r}")
        agent = dict(agent)
        self_host = hub.self_host
        hosts_cfg = {str(h.get("name")): str(h.get("ssh") or h.get("name")) for h in (hub.cfg.get("hosts") or []) if isinstance(h, dict) and h.get("name")}
        vlogs_url = hub.cfg.get("victorialogs_url")

    tpath = agent.get("transcript")
    notes: list[str] = []
    max_bytes = 256 * 1024 if raw_mode else 4 * 1024 * 1024
    raw_bytes = b""
    total_len = 0
    source = ""

    # Security invariant: ONLY read the exact `transcript` path recorded by the collector for `addr`.
    if isinstance(tpath, str) and tpath:
        if agent["host"] == self_host:
            p = Path(tpath)
            if p.is_file():
                total_len = p.stat().st_size
                with open(p, "rb") as fh:
                    if total_len > max_bytes:
                        fh.seek(total_len - max_bytes)
                    raw_bytes = fh.read(max_bytes)
                source = "local"
            else:
                notes.append(f"local file not found: {tpath}")
        elif agent["host"] in hosts_cfg:
            target = hosts_cfg[agent["host"]]
            q = shlex.quote(tpath)
            cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", target, f"wc -c < {q} && tail -c {max_bytes} -- {q}"]
            proc = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, timeout=30)
            if proc.returncode == 0 and b"\n" in proc.stdout:
                first_nl = proc.stdout.index(b"\n")
                total_len = int(proc.stdout[:first_nl].decode("utf-8", errors="replace").strip())
                raw_bytes = proc.stdout[first_nl + 1:]
                source = "ssh"
            else:
                notes.append(f"ssh {target}: {proc.stderr.decode('utf-8', errors='replace').strip()}")
        else:
            notes.append(f"{agent['host']} is not a configured ssh host")
    else:
        notes.append("collector reported no transcript path")

    if not source and isinstance(vlogs_url, str) and vlogs_url:
        try:
            q_id = str(agent["id"]).replace("\\", "\\\\").replace('"', '\\"')
            q_expr = f'_time:30d session_id:="{q_id}" | sort by (_time) | fields _msg'
            endpoint = f"{vlogs_url.rstrip('/')}/select/logsql/query"
            body_data = urllib.parse.urlencode({"query": q_expr}).encode("utf-8")
            req = urllib.request.Request(endpoint, data=body_data, method="POST")
            with urllib.request.urlopen(req, timeout=20) as resp:
                lines_out = []
                for ln in resp.read().splitlines():
                    try:
                        obj = json.loads(ln)
                        if isinstance(obj, dict) and isinstance(obj.get("_msg"), str):
                            lines_out.append(obj["_msg"].encode("utf-8"))
                    except ValueError:
                        pass
                if lines_out:
                    raw_bytes = b"\n".join(lines_out) + b"\n"
                    total_len = len(raw_bytes)
                    source = "victorialogs"
        except Exception as exc:
            notes.append(f"VictoriaLogs: {exc}")

    if not source:
        raise RuntimeError("; ".join(notes) or "transcript unavailable")

    if raw_mode:
        out = {
            "addr": agent["addr"],
            "host": agent["host"],
            "path": tpath,
            "source": source,
            "bytes": total_len,
            "shown_bytes": len(raw_bytes),
            "raw_tail": raw_bytes.decode("utf-8", errors="replace"),
            "notes": notes,
        }
        return {"content": [{"type": "text", "text": f"Raw transcript ({len(raw_bytes)}/{total_len} bytes)"}], "structuredContent": out}

    view = _flatten_transcript(agent, raw_bytes, total_len, source, notes, max_entries)
    return {
        "content": [{"type": "text", "text": f"Transcript for {addr}: {len(view['entries'])} turns from {source}"}],
        "structuredContent": view,
    }


# --------------------------------------------------------------------------- Tool & Resource Registry

TOOLS: list[dict[str, Any]] = [
    {
        "name": "relay_map",
        "description": "Open the interactive Relay Map pew-pew dashboard showing live Claude Code, Codex, Antigravity, opencode, and pi sessions, message arcs, health alerts, token burn, and broadcast controls.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "lookback_hours": {"type": "number", "default": 24},
                "filter": {"type": "string"},
                "focus_agent": {"type": "string"},
                "tab": {"type": "string", "enum": ["attention", "agents", "messages", "spend"]},
            },
        },
        "_meta": {
            "ui": {"resourceUri": APP_URI, "visibility": ["model", "app"]},
            "com.google.antigravity/ui": {
                "entrypoints": [
                    {"type": "aux_pane", "title": "Relay Map"},
                    {"type": "global", "title": "Agent Relay"},
                ]
            },
        },
    },
    {
        "name": "relay_sessions",
        "description": "List live Claude Code, Codex, Antigravity, opencode, and pi sessions on this computer or over ssh.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "host": {"type": "string"},
                "all": {"type": "boolean", "default": False},
            },
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_send",
        "description": "Send a relayed message to a live agent session (claude, codex, antigravity, opencode, or pi) on this computer or @host over ssh.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "to": {"type": "string"},
                "message": {"type": "string"},
                "subject": {"type": "string"},
                "from_addr": {"type": "string"},
                "wait": {"type": "boolean", "default": False},
                "timeout": {"type": "number", "default": 180},
            },
            "required": ["to", "message"],
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_inbox",
        "description": "Inspect pending relayed messages in a local Claude Code or pi session mailbox.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session": {"type": "string"},
                "peek": {"type": "boolean", "default": True},
            },
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_read_reply",
        "description": "Read recent assistant replies from a Claude Code, Codex, Antigravity, opencode, or pi session.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "target": {"type": "string"},
                "since_ms": {"type": "integer", "default": 0},
                "limit": {"type": "integer", "default": 3},
            },
            "required": ["target"],
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_broadcast",
        "description": "Preview or send a broadcast message to all live agents, a pool (pool:<name>), or specific agents (agent:<addr>) through agent-relay.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "scope": {"type": "string", "default": "all"},
                "addrs": {"type": "array", "items": {"type": "string"}},
                "subject": {"type": "string"},
                "body": {"type": "string"},
                "dry_run": {"type": "boolean", "default": True},
                "include_stale": {"type": "boolean", "default": False},
            },
            "required": ["scope"],
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_transcript",
        "description": "Read an agent session's recorded transcript (flattened turns or raw JSONL tail) from the collector's recorded transcript path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "addr": {"type": "string"},
                "max_entries": {"type": "integer", "default": 600},
                "raw": {"type": "boolean", "default": False},
            },
            "required": ["addr"],
        },
        "_meta": {"ui": {"visibility": ["model", "app"]}},
    },
    {
        "name": "relay_map_poll",
        "description": "App-only incremental poll returning the latest Relay Map snapshot.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "lookback_hours": {"type": "number", "default": 24},
                "since_ms": {"type": "integer"},
            },
        },
        "_meta": {"ui": {"visibility": ["app"]}},
    },
    {
        "name": "relay_map_prefs",
        "description": "App-only tool to read or update persisted Relay Map UI preferences.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "patch": {"type": "object"},
            },
        },
        "_meta": {"ui": {"visibility": ["app"]}},
    },
]

TOOL_DISPATCH = {
    "relay_map": tool_relay_map,
    "relay_sessions": tool_relay_sessions,
    "relay_send": tool_relay_send,
    "relay_inbox": tool_relay_inbox,
    "relay_read_reply": tool_relay_read_reply,
    "relay_broadcast": tool_relay_broadcast,
    "relay_transcript": tool_relay_transcript,
    "relay_map_poll": tool_relay_map_poll,
    "relay_map_prefs": tool_relay_map_prefs,
}


def _send(obj: dict[str, Any]) -> None:
    line = json.dumps(obj) + "\n"
    with _SEND_LOCK:
        sys.stdout.write(line)
        sys.stdout.flush()


def _handle_request(req: dict[str, Any], data_dir: Path) -> None:
    req_id = req.get("id")
    method = req.get("method", "")
    params = req.get("params") or {}

    if req_id is None:
        return

    try:
        if method == "initialize":
            _send(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "protocolVersion": params.get("protocolVersion", "2024-11-05"),
                        "capabilities": {
                            "tools": {"listChanged": False},
                            "resources": {"subscribe": False, "listChanged": False},
                        },
                        "serverInfo": {"name": "agent-relay", "version": VERSION},
                    },
                }
            )
            return

        if method == "ping":
            _send({"jsonrpc": "2.0", "id": req_id, "result": {}})
            return

        if method == "resources/list":
            _send(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "resources": [
                            {
                                "uri": APP_URI,
                                "name": "Relay Map",
                                "description": "Live multi-agent pew-pew traffic map, health telemetry, and broadcast controls.",
                                "mimeType": "text/html;profile=mcp-app",
                                "_meta": {"ui": {"prefersBorder": False}},
                            }
                        ]
                    },
                }
            )
            return

        if method == "resources/read":
            uri = params.get("uri", "")
            if uri != APP_URI:
                _send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32602, "message": f"Unknown resource URI: {uri}"}})
                return
            override = os.environ.get("RELAY_MAP_HTML", "").strip()
            html_path = Path(override).expanduser() if override else (_plugin_root() / "assets" / "relay-map.html")
            html_text = html_path.read_text(encoding="utf-8")
            _send(
                {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "contents": [
                            {
                                "uri": APP_URI,
                                "mimeType": "text/html;profile=mcp-app",
                                "text": html_text,
                                "_meta": {"ui": {"prefersBorder": False}},
                            }
                        ]
                    },
                }
            )
            return

        if method == "tools/list":
            _send({"jsonrpc": "2.0", "id": req_id, "result": {"tools": TOOLS}})
            return

        if method == "tools/call":
            name = params.get("name", "")
            arguments = params.get("arguments") or {}
            fn = TOOL_DISPATCH.get(name)
            if not fn:
                _send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"Unknown tool: {name}"}})
                return
            try:
                res = fn(data_dir, arguments)
                _send({"jsonrpc": "2.0", "id": req_id, "result": res})
            except Exception as exc:
                _send(
                    {
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "isError": True,
                            "content": [{"type": "text", "text": f"Error in {name}: {exc}"}],
                        },
                    }
                )
            return

        _send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": f"Method not found: {method}"}})
    except Exception as exc:
        _send({"jsonrpc": "2.0", "id": req_id, "error": {"code": -32603, "message": str(exc)}})


def main() -> int:
    p = argparse.ArgumentParser(description="Agent Relay MCP Server")
    p.add_argument("--data-dir", default=os.environ.get("AGENT_RELAY_DATA_DIR") or str(_relay_home()))
    args = p.parse_args()
    data_dir = Path(args.data_dir).expanduser()
    data_dir.mkdir(parents=True, exist_ok=True)

    executor = ThreadPoolExecutor(max_workers=8)
    try:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except json.JSONDecodeError as exc:
                _send({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": f"Parse error: {exc}"}})
                continue
            if req.get("method") == "tools/call":
                executor.submit(_handle_request, req, data_dir)
            else:
                _handle_request(req, data_dir)
    finally:
        executor.shutdown(wait=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
