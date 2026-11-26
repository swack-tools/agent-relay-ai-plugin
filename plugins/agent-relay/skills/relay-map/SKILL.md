---
name: relay-map
description: Open the interactive Relay Map pew-pew dashboard via MCP (or run the standalone Rust/WASM hub) to inspect live Claude Code, Codex, Antigravity, opencode, and pi sessions, health alerts, token burn, transcripts, and broadcasts.
---

# Relay Map

Use Relay Map to visualize and control live multi-agent traffic across Claude Code, Codex, Antigravity, opencode, and pi.

## Opening the MCP Dashboard

When the `relay` MCP server is connected (Antigravity, Claude Code, Codex), call `relay_map`:

- **Open full dashboard**: `relay_map()`
- **Filter or focus**: `relay_map(filter="status:waiting", tab="attention")` or `relay_map(focus_agent="claude:<id>@<host>")`
- **Inspect a transcript**: `relay_transcript(addr="<kind>:<id>@<host>")`
- **Broadcast**: preview first with `relay_broadcast(scope="all", dry_run=True)`, then confirm with `dry_run=False`.

In Antigravity Desktop App, **Agent Relay** also appears in the left sidebar under **Plugins** and **Relay Map** appears in the side pane `+` menu.

## CLI & Standalone Cluster Hub Fallback

When running in a client without MCP App UI (or on a remote server):

- List sessions via CLI: `python3 scripts/relay.py sessions`
- Run the standalone Rust/WASM hub locally: `cd web-ui && ./build.sh && ./target/release/relay-map serve`
- Deploy to Kubernetes: see `web-ui/deploy/k8s/README.md`.
