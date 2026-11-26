---
name: message-session
description: Use when the user asks to send a message, prompt, task, review result, or file to another running Claude Code, Codex, Antigravity, opencode or pi session (desktop app or terminal), on this computer or another one over ssh, or to ask another session a question and bring back its answer.
---

# Message another agent session

## The relay command

In Claude Code run `python3 "${CLAUDE_PLUGIN_ROOT}/scripts/relay.py"`. In Codex, Antigravity, opencode or pi run `python3 <plugin root>/scripts/relay.py`, where the plugin root is two directories above this skill's folder. If the session-start context printed an absolute `Relay CLI:` command, use that. Below, `relay` means that command.

## Addresses

`codex:<thread id or exact name>`, `claude:<session id, id prefix, label, or folder name>` `antigravity:<conversation id, id prefix, or exact title>`, `opencode:<session id, id prefix, title, or folder name>` or `pi:<session id, id prefix, label, or folder name>`, optionally followed by `@<ssh alias>` for another computer. The same address works whichever app the sender runs in, and for desktop and terminal sessions alike: Codex app and ChatGPT desktop threads are Codex threads, Claude desktop Code sessions are Claude Code sessions, and Antigravity app, IDE and `agy` conversations are Antigravity conversations. If the user names a session loosely ("the billing window on laptop"), run the list-sessions skill first and confirm the match when more than one fits.

## How each route delivers

- **To Codex:** `codex queue` into the shared Codex daemon that serves the desktop app and CLI windows. An idle thread starts a turn with the message right away. A thread mid-turn gets it as soon as the current turn ends. A thread that is not loaded keeps it until someone opens that session, unless you add `--wake`, which asks the daemon to resume it. The message becomes a user turn in that thread and runs with that thread's own permissions.
- **To Claude Code:** a mailbox on the target computer. The target session's hooks deliver it: a background watcher wakes an idle session within a few seconds, and anything still pending arrives with the next prompt. The receiving Claude treats it as a request from a peer agent, not as its user's own words, so write it as a request with context and reasons.
- **To opencode:** the message is queued into the session through the opencode background service (`opencode api session.prompt`); an idle session starts a turn with it right away, a busy one when its turn ends.
- **To pi:** the message goes to the session's mailbox; the agent-relay pi extension picks it up within seconds and starts a turn if the session is idle.
- **To Antigravity:** a desktop-app conversation gets the message through the Antigravity app's own `agentapi send-message`; an idle one starts a turn with it right away. An IDE or `agy` CLI conversation is woken the same way through its own language server when the relay has that server cached (route `agentapi send-message (cached server)`); otherwise, or if the cached server is gone, it gets the message in Antigravity's message store and reads it at the start of its next turn. Report the route the result names; never print the cached token. Antigravity shows it as a system message, not as its user's words.
- **Claude Code to Claude Code:** when the native `SendMessage` tool is available and the target appears in `ListAgents` (same computer, or another machine through Remote Control), prefer it. Use the relay when the target does not appear there or the sender is Codex.

## Steps

1. Get the exact message. Send what the user asked to send. Do not add tasks, change scope, or include secrets, tokens, or private files the user did not ask to share.
2. Short messages go inline: `relay send --to <address> --message '<text>'`. For long text, write it to a file and use `--file <path>`. Bodies over about 6,000 characters are written to a file on the receiving computer automatically, and the message tells the receiver where to read it; that keeps queued Codex turns short.
3. The relay detects this session's own address and adds it as the reply address. Pass `--from <address>` only when that detection fails (`relay whoami` shows it).
4. Add `--json` and read the result: `route`, `delivery`, `thread_status` or `session_state`, and any `warning`. Tell the user how the message will arrive, for example "queued; billing is mid-turn running its review, so it will see this when that turn ends".
5. When the user wants the answer back, add `--wait-reply <seconds>` (for example 600), or send first and later run `relay read <address> --since <sent_at_ms> --wait <seconds>`. `complete: false` means the target has not finished; report that plainly and offer to check again instead of claiming an answer. The receiver may also reply with its own `relay send`, which reaches this session as an `<agent-relay-message>`.
6. Report what was sent, where, the route, and the reply or its absence. Quote the target's answer as theirs.

## Replies from a computer that cannot reach this one

Replies sent with `relay send` travel back over ssh, so they need the target computer to reach this one by an ssh alias. When it cannot (for example a server with no ssh entry for the laptops), ask the receiver for an answer in its own conversation and collect it with `--wait-reply` or `relay read <address> --since <sent_at_ms> --wait <seconds>`; that pulls the reply over this computer's ssh connection.

## Examples

`Tell the billing Codex session on laptop to read /tmp/webapp-review.md and fold it into its merge review.`
`relay send --to codex:billing@laptop --message 'Please read /tmp/webapp-review.md on laptop and fold its findings into your webapp merge review. Reply to me when done.' --json`

`Ask my Claude session in ~/src/billing-api on pc whether its tests pass, and wait for the answer.`
`relay send --to claude:billing-api@pc --message 'Do the billing-api tests pass on your branch right now? Reply with the command you ran and the result.' --wait-reply 600 --json`

`Ask the deal finder Antigravity conversation on laptop what it found, and wait.`
`relay send --to 'antigravity:deal finder@laptop' --message 'What deals did you find in your last run? Answer in this conversation.' --wait-reply 600 --json`

## Boundaries

- Send only when the user asked for this message to go to this session. A reply to a relayed message counts as part of that exchange.
- Never send credentials, the contents of `~/.ssh`, tokens, or another session's private output to a third session unless the user explicitly asks.
- Do not loop: do not answer an automated acknowledgement with another message, and stop after the exchange the user asked for.
- A failed `codex queue` or ssh call delivered nothing. Do not retry more than once. Run `relay doctor` (with `--host`) and report.
- If the relay cannot reach a Codex thread, the user can attach a second window to it themselves: `codex resume <thread id> --remote unix://$HOME/.codex/app-server-control/app-server-control.sock`, then paste the message.
