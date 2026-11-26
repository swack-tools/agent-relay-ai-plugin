# Agent Relay reference

## MCP tools

The bundled `relay` MCP server exposes 7 model-visible tools and 2 app-only UI tools (`relay_map_poll`, `relay_map_prefs`). See the [complete technical reference](https://relay.ai.swacktech.com/reference.html) for CLI commands, hooks, and architecture details.

| Tool | Title | Description |
| :--- | :--- | :--- |
| `relay_map` | Open Relay Map dashboard | Open the interactive Relay Map pew-pew dashboard showing live Claude Code, Codex, Antigravity, opencode, and pi sessions, message arcs, health alerts, token burn, and broadcast controls. |
| `relay_sessions` | List live agent sessions | List live Claude Code, Codex, Antigravity, opencode, and pi sessions on this computer or over ssh. |
| `relay_send` | Send relayed message | Send a relayed message to a live agent session on this computer or over ssh, optionally waiting for a reply. |
| `relay_inbox` | Inspect session inbox | Inspect pending relayed messages in a local Claude Code or pi session mailbox. |
| `relay_read_reply` | Read recent agent replies | Read recent assistant replies from a Claude Code, Codex, Antigravity, opencode, or pi session. |
| `relay_broadcast` | Broadcast to agents | Preview or send a broadcast message to all live agents, a pool, or selected agents through agent-relay. |
| `relay_transcript` | Read session transcript | Read an agent session's recorded transcript (flattened turns or raw JSONL tail) from the collector's recorded transcript path. |

## Lifecycle hooks

- **SessionStart**: Registers the session, starts the idle wake-up watcher when applicable, and announces the session's relay address.
- **UserPromptSubmit**: Delivers pending mailbox messages and marks the session busy.
- **Stop**: Marks the session idle and refreshes the wake-up watcher.
