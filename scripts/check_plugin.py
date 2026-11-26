#!/usr/bin/env python3
"""Package contract checks for the agent-relay plugin (offline, standard library)."""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / "plugins" / "agent-relay"
SKILLS = {"list-sessions", "message-session", "handle-relay-message", "relay-setup", "relay-map"}
SHARED = ("name", "version", "description", "license", "repository")
FORBIDDEN = {".DS_Store", "Thumbs.db", "__pycache__", ".env", "node_modules"}
errors = []


def check(cond, msg):
    if not cond:
        errors.append(msg)


def load(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        errors.append(f"{path.relative_to(ROOT)}: {exc}")
        return {}


def inside(value, label):
    check(isinstance(value, str) and value.startswith("./"), f"{label}: path must start with ./")
    if isinstance(value, str):
        target = (PLUGIN / value).resolve()
        check(target.is_relative_to(PLUGIN) and target.exists(), f"{label}: {value} is missing or escapes the plugin")


claude = load(PLUGIN / ".claude-plugin" / "plugin.json")
codex = load(PLUGIN / ".codex-plugin" / "plugin.json")
antigravity = load(PLUGIN / ".antigravity-plugin" / "plugin.json")
for key in SHARED:
    vals = {json.dumps(m.get(key)) for m in (claude, codex)}
    check(len(vals) == 1, f"manifests disagree on {key}: {sorted(vals)}")
for key in ("name", "version", "description"):
    check(antigravity.get(key) == claude.get(key), f"Antigravity manifest disagrees on {key}")
check(antigravity.get("displayName") == "Agent Relay", "Antigravity manifest: displayName must be 'Agent Relay'")

# Antigravity reads only the root hooks.json, in its own named-group schema.
agy_hooks = load(PLUGIN / "hooks.json")
check(list(agy_hooks) == ["agent-relay"], "root hooks.json must hold exactly one named hook, agent-relay")
for event, handlers in agy_hooks.get("agent-relay", {}).items():
    check(event in ("SessionStart", "PreInvocation", "PostInvocation", "Stop"), f"root hooks.json: event {event}")
    for h in handlers:
        check(h.get("command", "").startswith("python3 ./scripts/relay.py hook --agent antigravity "),
              f"root hooks.json: {h.get('command')}")
check(re.fullmatch(r"\d+\.\d+\.\d+", claude.get("version", "")) is not None, "version must be semver")
check("hooks" not in claude, "Claude manifest must not list hooks/hooks.json (it is loaded by default)")
inside(claude.get("icon"), "claude icon")
inside(claude.get("mcpServers"), "claude mcpServers")
inside(codex.get("skills"), "codex skills")
inside(codex.get("hooks"), "codex hooks")
inside(codex.get("mcpServers"), "codex mcpServers")
iface = codex.get("interface", {})
check(not (PLUGIN / "plugin.json").exists(),
      "no root plugin.json: Codex 0.169 stops loading a plugin's hooks when one is present")
for key in ("composerIcon", "logo"):
    inside(iface.get(key), f"codex interface {key}")

for mcp_rel in ("mcp_config.json", ".mcp.json"):
    mcp = load(PLUGIN / mcp_rel)
    check("relay" in mcp.get("mcpServers", {}), f"{mcp_rel}: missing 'relay' MCP server")

html_path = PLUGIN / "assets" / "relay-map.html"
check(html_path.is_file() and 20_000 < html_path.stat().st_size < 10 * 1024 * 1024,
      "assets/relay-map.html missing or bad size")
if html_path.is_file():
    html_txt = html_path.read_text(encoding="utf-8", errors="replace")
    check("fonts.googleapis.com" not in html_txt, "assets/relay-map.html must not reference external Google Fonts")

for sc in sorted((PLUGIN / "scripts").glob("*.py")):
    check((sc.stat().st_mode & 0o111) != 0, f"{sc.relative_to(ROOT)}: must be executable (0755)")

tools_manifest = load(ROOT / "api" / "agent-relay" / "tools.json")
manifest_names = [t.get("name") for t in tools_manifest.get("tools", []) if isinstance(t, dict)]
check(len(manifest_names) == 7 and len(set(manifest_names)) == 7, "api/agent-relay/tools.json must list 7 unique tools")
ref_md = (ROOT / "docs" / "reference.md").read_text(encoding="utf-8")
documented_names = re.findall(r"^\|\s*`([a-z0-9_]+)`\s*\|", ref_md, flags=re.MULTILINE)
check(set(manifest_names) == set(documented_names),
      f"api/agent-relay/tools.json {manifest_names} != docs/reference.md {documented_names}")

market = load(ROOT / ".claude-plugin" / "marketplace.json")
check([p.get("source") for p in market.get("plugins", [])] == ["./plugins/agent-relay"], "Claude marketplace source")
amarket = load(ROOT / ".agents" / "plugins" / "marketplace.json")
check([p.get("source", {}).get("path") for p in amarket.get("plugins", [])] == ["./plugins/agent-relay"],
      "Codex marketplace source")

found = {p.name for p in (PLUGIN / "skills").iterdir() if p.is_dir()}
check(found == SKILLS, f"skills on disk {sorted(found)} != expected {sorted(SKILLS)}")
for name in sorted(found):
    text = (PLUGIN / "skills" / name / "SKILL.md").read_text(encoding="utf-8")
    m = re.match(r"---\nname: (.+)\ndescription: (.+)\n---\n", text)
    check(m is not None, f"{name}: frontmatter must hold name and description")
    if m:
        check(m.group(1).strip() == name, f"{name}: frontmatter name mismatch")
        check(20 <= len(m.group(2)) <= 1024, f"{name}: description length")
    check("scripts/relay.py" in text, f"{name}: must say how to run the relay script")
    check((PLUGIN / "skills" / name / "agents" / "openai.yaml").is_file(), f"{name}: agents/openai.yaml missing")

for hooks_file, var, agent in (("hooks.json", "${CLAUDE_PLUGIN_ROOT}", "claude"),
                               ("codex-hooks.json", "${PLUGIN_ROOT}", "codex")):
    data = load(PLUGIN / "hooks" / hooks_file)
    commands = [h.get("command", "") for groups in data.get("hooks", {}).values()
                for g in groups for h in g.get("hooks", [])]
    check(commands, f"{hooks_file}: no hooks")
    for c in commands:
        check(c.startswith(f'python3 "{var}/scripts/relay.py" hook --agent {agent} '), f"{hooks_file}: {c}")

for p in PLUGIN.rglob("*"):
    rel = p.relative_to(PLUGIN)
    check(not p.is_symlink(), f"symlink: {rel}")
    check(not any(part in FORBIDDEN or part.startswith("._") or part.endswith(".pyc") for part in rel.parts),
          f"forbidden file: {rel}")
    if p.suffix == ".png":
        check(p.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n", f"invalid PNG: {rel}")

# Sensitive-data guardrail across tracked repository files.
SECRET_PATTERNS = [
    ("private key", re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")),
    ("GitHub token", re.compile(r"\b(?:ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b")),
    ("Anthropic key", re.compile(r"\bsk-ant-[A-Za-z0-9_-]{16,}\b")),
    ("AWS key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("Slack token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{16,}\b")),
]
IDENT_PATTERNS = [
    ("local user path", re.compile(r"/Users/[A-Za-z0-9._-]{2,}/|/home/(?!relay\b|u\b|me\b|user\b)[A-Za-z0-9._-]{2,}/")),
    ("real host or internal domain", re.compile(r"\b(?:m4air|m5air|0fc6ce)\b", re.IGNORECASE)),
    ("private project name", re.compile(r"\b(?:lg_g5|lg-g5|g5_calibration|colorimeter|rustyget|nzbget|zipfast|prybar|zipview|home-net|par-rs)\b", re.IGNORECASE)),
]
LOCK_FILES = {"web-ui/Cargo.lock", "ui/package-lock.json"}
try:
    import subprocess
    tracked = subprocess.check_output(["git", "-C", str(ROOT), "ls-files", "-z"], text=False).split(b"\0")
    scan_paths = [ROOT / p.decode("utf-8") for p in tracked if p]
except Exception:
    scan_paths = [p for p in ROOT.rglob("*") if p.is_file()]

for p in scan_paths:
    if not p.is_file():
        continue
    rel = p.relative_to(ROOT)
    rel_s = rel.as_posix()
    if any(part in FORBIDDEN or part == ".git" or part == "target" for part in rel.parts):
        continue
    if rel_s in LOCK_FILES or p.suffix in {".png", ".webp", ".svg", ".ico", ".zip", ".plugin", ".gz"}:
        continue
    try:
        txt = p.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        continue
    for label, rx in SECRET_PATTERNS:
        check(not rx.search(txt), f"sensitive {label} found in {rel_s}")
    if rel_s == "scripts/check_plugin.py":
        continue
    for label, rx in IDENT_PATTERNS:
        check(not rx.search(txt), f"sensitive {label} found in {rel_s}")
    for m in re.finditer(r"[A-Za-z0-9.-]*swacktech\.com", txt, re.IGNORECASE):
        dom = m.group(0).lower().lstrip(".")
        check(dom == "ai.swacktech.com" or dom.endswith(".ai.swacktech.com"),
              f"sensitive internal domain {dom} found in {rel_s}")

if errors:
    print("\n".join(f"error: {e}" for e in errors))
    sys.exit(1)
print(f"ok: v{claude.get('version')}, {len(found)} skills, 3 manifests, 2 marketplaces, 3 hook files, mcp + UI")
