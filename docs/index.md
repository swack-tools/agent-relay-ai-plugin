# Agent Relay

Cross-client agent messaging and live Relay Map observability for Antigravity, Claude Code, Codex, opencode, and pi, on one computer or across computers over `ssh`. Read the [documentation site](https://relay.ai.swacktech.com/) for the Relay Map visual tour, install walkthrough, and tool reference.

## Key features

- **Five-agent messaging**: Route prompts and replies between Claude Code, Codex, Antigravity, opencode, and pi sessions on the local machine or over `ssh`.
- **Interactive Relay Map MCP App**: Live Canvas2D pew-pew traffic map, health alerts (`working`, `waiting`, `stuck`, `looping`), token burn & spend breakdown, session log viewer, and two-step broadcast modal.
- **Standalone Rust/WASM hub & Kubernetes deployment**: Optional cluster-hosted `relay-map` hub in `web-ui/` for multi-host tailing over SSH behind an Envoy Gateway.
- **Lifecycle hooks & extensions**: Automatic session registration, relay address announcement, and idle wake-up delivery across all five clients.
