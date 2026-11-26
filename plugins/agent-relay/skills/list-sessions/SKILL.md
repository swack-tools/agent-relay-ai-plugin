---
name: list-sessions
description: Use when the user asks which Claude Code, Codex, Antigravity, opencode or pi sessions are running, on this computer or another one reachable over ssh (for example "what's open on laptop", "find the billing Codex window"), or before messaging a session whose address is not known yet.
---

# List agent sessions

## The relay command

In Claude Code run `python3 "${CLAUDE_PLUGIN_ROOT}/scripts/relay.py"`. In Codex, Antigravity, opencode or pi run `python3 <plugin root>/scripts/relay.py`, where the plugin root is two directories above this skill's folder. If the session-start context printed an absolute `Relay CLI:` command, use that. Below, `relay` means that command.

## Steps

1. Pick the computer. No computer named means this one. A computer name the user gives is an ssh alias (`pc`, `laptop`): pass `--host <alias>`. Do not guess aliases; ask, or read `~/.ssh/config` if the user allows it.
2. Run `relay sessions --json` (add `--host <alias>` for another computer; add `--all` to include recent Codex threads that are not loaded and exited Claude sessions).
3. Read the result:
   - `codex` lists threads in the shared Codex app-server daemon, which serves the Codex desktop app and every CLI window on that computer. `status` is `active` (mid-turn), `idle`, or `notLoaded`. `name` is the thread title, and `codex queue` accepts the exact name or the id.
   - `claude` lists Claude Code sessions registered by this plugin's hooks, with `state` (`busy` or `idle`), `alive` (whether its process still runs) and `pending` (undelivered relay messages). Sessions started before the plugin was installed are missing until they restart.
   - `opencode` lists opencode sessions the opencode background service worked on in the last day, with their title, folder and `status` (`idle` or `running`). Address them by id, an id prefix, the exact title or the folder name.
   - `pi` lists pi sessions registered by the agent-relay pi extension, like `claude`.
   - `antigravity` lists recent Antigravity conversations from the desktop app (`surface=app`), the IDE (`ide`) and the `agy` CLI (`cli`), with their title, `status` (`idle` or `running`) and `open` (shown in a window right now). Address them by id, an id prefix, or the exact title.
   - `notes` explains a missing part, such as no Codex daemon running.
4. In Claude Code, also call the native `ListAgents` tool when it is available: it shows other local Claude sessions and, with Remote Control connected, Claude sessions on other machines. Those are messaged with `SendMessage`, not the relay.
5. Present each session as an address the user can reuse: `codex:<id or name>@<host>`, `claude:<id>@<host>`, `antigravity:<id>@<host>`, `opencode:<id>@<host>` or `pi:<id>@<host>`, plus its name or folder, status, and what it is working on (the Codex `preview`, the Antigravity title). Say which computer each list came from.

To see what a session most recently said, run `relay read <address> --last 2`.

## Failures

- `ssh <alias> failed`: the computer is off, asleep, or ssh asks for a password. `relay` uses `BatchMode`, so key-based ssh must work (`ssh <alias> true`). Use the relay-setup skill.
- `Codex app-server socket not found`: no Codex daemon runs on that computer, so there are no live Codex threads to message.
- No Claude sessions: the plugin is not installed there, or its sessions began before installation.

Session lists can include project paths and task summaries. Show what the user asked about, and do not copy them anywhere else.
