#!/usr/bin/env python3
"""Build deterministic multi-client release archives and SHA256SUMS."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import shutil
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PLUGIN_DIR = ROOT / "plugins" / "agent-relay"
FIXED_ZIP_DT = (2026, 1, 1, 0, 0, 0)
FORBIDDEN_PARTS = {"__pycache__", ".DS_Store", ".data", ".env", "node_modules"}

CLIENT_EXCLUDES = {
    "antigravity": {".antigravity-plugin", ".claude-plugin", ".codex-plugin", ".mcp.json", "hooks"},
    "claude": {".antigravity-plugin", ".codex-plugin", "mcp_config.json", "hooks.json", "agents", "rules", "opencode", "pi", "package.json"},
    "codex": {".antigravity-plugin", ".claude-plugin", "mcp_config.json", "hooks.json", "agents", "rules", "opencode", "pi", "package.json"},
}


def _iter_files(client: str) -> list[tuple[Path, str]]:
    excludes = CLIENT_EXCLUDES[client]
    items: list[tuple[Path, str]] = []
    for p in sorted(PLUGIN_DIR.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(PLUGIN_DIR)
        if any(part in FORBIDDEN_PARTS or part.endswith(".pyc") for part in rel.parts):
            continue
        if rel.parts[0] in excludes:
            continue
        if client == "claude" and rel.as_posix() == "hooks/codex-hooks.json":
            continue
        if client == "codex" and rel.as_posix() == "hooks/hooks.json":
            continue
        items.append((p, rel.as_posix()))
    if client == "antigravity":
        items.append((PLUGIN_DIR / ".antigravity-plugin" / "plugin.json", "plugin.json"))
    items.sort(key=lambda it: it[1])
    return items


def _file_mode(arcname: str) -> int:
    if arcname.startswith("scripts/") and arcname.endswith(".py"):
        return 0o755
    return 0o644


def _write_zip(dest: Path, items: list[tuple[Path, str]]) -> None:
    with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for src, arcname in items:
            zi = zipfile.ZipInfo(arcname, date_time=FIXED_ZIP_DT)
            zi.external_attr = (0o100000 | _file_mode(arcname)) << 16
            zi.compress_type = zipfile.ZIP_DEFLATED
            zf.writestr(zi, src.read_bytes())


def build_release(out_dir: Path, version_override: str = "") -> dict[str, str]:
    manifest = json.loads((PLUGIN_DIR / ".antigravity-plugin" / "plugin.json").read_text(encoding="utf-8"))
    name = str(manifest["name"])
    version = version_override or str(manifest["version"])
    out_dir.mkdir(parents=True, exist_ok=True)

    agy_zip = out_dir / f"{name}-{version}.zip"
    agy_plugin = out_dir / f"{name}-{version}.plugin"
    agy_tgz = out_dir / f"{name}-{version}.tar.gz"
    claude_zip = out_dir / f"{name}-claude.zip"
    codex_zip = out_dir / f"{name}-codex.zip"
    sums_path = out_dir / "SHA256SUMS"

    agy_items = _iter_files("antigravity")
    _write_zip(agy_zip, agy_items)
    shutil.copyfile(agy_zip, agy_plugin)
    _write_zip(claude_zip, _iter_files("claude"))
    _write_zip(codex_zip, _iter_files("codex"))

    with open(agy_tgz, "wb") as raw_f:
        with gzip.GzipFile(fileobj=raw_f, mode="wb", mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode="w") as tf:
                for src, arcname in agy_items:
                    data = src.read_bytes()
                    ti = tarfile.TarInfo(f"{name}/{arcname}")
                    ti.size = len(data)
                    ti.mode = _file_mode(arcname)
                    ti.mtime = 0
                    ti.uid = 0
                    ti.gid = 0
                    ti.uname = ""
                    ti.gname = ""
                    tf.addfile(ti, io.BytesIO(data))

    checksums: dict[str, str] = {}
    lines: list[str] = []
    for artifact in sorted((agy_zip, agy_plugin, agy_tgz, claude_zip, codex_zip), key=lambda a: a.name):
        digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
        checksums[artifact.name] = digest
        lines.append(f"{digest}  {artifact.name}\n")
    sums_path.write_text("".join(lines), encoding="utf-8")
    return checksums


def main() -> int:
    p = argparse.ArgumentParser(description="Build deterministic agent-relay release archives")
    p.add_argument("out_dir", nargs="?", default=str(ROOT / "dist"))
    p.add_argument("--version", default="")
    args = p.parse_args()
    checksums = build_release(Path(args.out_dir), version_override=args.version)
    print(json.dumps({"out_dir": args.out_dir, "artifacts": checksums}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
