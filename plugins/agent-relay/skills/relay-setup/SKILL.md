---
name: relay-setup
description: Use when agent-relay cannot reach a session or computer, when setting it up across computers (ssh aliases, reply addresses), when Codex hooks need trusting, or when the Codex CLI and its background daemon report different versions.
---

# Set up and troubleshoot agent-relay

## The relay command

In Claude Code run `python3 "${CLAUDE_PLUGIN_ROOT}/scripts/relay.py"`. In Codex, Antigravity, opencode or pi run `python3 <plugin root>/scripts/relay.py`, where the plugin root is two directories above this skill's folder. If the session-start context printed an absolute `Relay CLI:` command, use that. Below, `relay` means that command.

Start with `relay doctor`, and `relay doctor --host <alias>` for the other computer. Report each `!!` line with its fix. Diagnose before changing anything, and ask before editing the user's ssh config, shell profile, or Codex settings.

## Requirements on each computer

- `python3` 3.9 or newer. The relay is one standard-library script; over ssh it is streamed to the remote `python3`, so nothing has to be installed there just to send.
- To receive as Claude Code: this plugin installed there, with sessions started after installation (the hooks register them).
- To receive as Codex: a Codex app or CLI session there, which runs the shared app-server daemon. Installing the plugin in Codex adds relay addresses to Codex sessions and lets them use these skills.

- To receive as Antigravity: nothing beyond Antigravity itself. Desktop-app conversations are woken while the Antigravity app runs. IDE and `agy` CLI conversations are woken too once they have run any relay command (the relay caches their language server's address and token, owner-only, never shown); otherwise they read messages at their next turn. Installing the plugin in Antigravity (`agy plugin install <plugin folder>`) gives its agents these skills and their relay address.

## Across computers

1. Each direction needs key-based ssh: `ssh <alias> true` must succeed without a prompt, because the relay uses `BatchMode`. The user sets up keys; never ask them to paste a private key or password into chat.
2. Replies travel back over ssh too. The reply address uses this computer's short host name. If the other computer reaches this one by a different alias, set it with `AGENT_RELAY_SELF=<alias>` or `~/.agent-relay/config.json` containing `{"self": "<alias>"}`.
3. Non-interactive ssh sessions often have a short `PATH`. The relay also searches `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `~/.npm-global/bin` and `~/.bun/bin`. If `codex` lives elsewhere, set `AGENT_RELAY_CODEX_BIN=/path/to/codex` in the environment that ssh commands get.

4. A computer that cannot ssh back (a server with no entry for the laptops) can still be messaged; replies are collected from the sending side with `--wait-reply` or `relay read --wait`, which use the sender's own ssh connection.

## Common problems

- **Codex hooks do not run:** Codex skips plugin hooks until the user reviews and trusts them. They only add the session's relay address and deliver mailbox messages; queued messages reach Codex without them.
- **CLI and daemon versions differ** (`cli=0.169 app-server=0.167`): an upgrade replaced the CLI while the old daemon kept running. Queueing usually still works, and the first send is the test. `codex app-server daemon restart` aligns them but interrupts running turns, so only suggest it when the user can afford that.
- **A Claude session is missing from `relay sessions`:** it started before the plugin was installed, or its process exited. Restarting it, or running `/hooks` to confirm the agent-relay hooks are enabled, fixes the first case. `relay gc` clears records of exited sessions.
- **A message reached a Claude mailbox but was not seen:** the background watcher covers about an hour after each turn. After that, the message arrives with the user's next prompt, or with `relay inbox` run inside that session.
- **Codex thread `notLoaded`:** the message waits until someone opens that session. `relay send ... --wake` asks the daemon to resume it.
- **Antigravity message not answered:** IDE and `agy` CLI conversations that never ran a relay command (or whose language server restarted since) only read it when their next turn starts; having that conversation run `relay whoami` once refreshes the cache, and a desktop-app conversation is woken only while the Antigravity app runs (`relay doctor` says whether its server is up).
- **Manual fallback for Codex:** attach another window to the live thread with `codex resume <thread id> --remote unix://$HOME/.codex/app-server-control/app-server-control.sock` and paste the message.

## Where data lives

`~/.agent-relay/` holds `sessions/` (registered sessions: id, folder, transcript path, process id), `inbox/` (pending and `delivered/` messages), `outbox/` (long message bodies) and `watch/` (watcher locks). The folders are created readable only by the user. Message bodies stay on the computers involved; nothing is sent to any service by the relay itself. Delete those folders to clear history.
