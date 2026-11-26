# Security and privacy

## What the relay does with data

- Messages travel only between the computers involved: through `ssh` to
  another computer, through the local Codex app-server daemon's Unix socket, and
  through files in `~/.agent-relay/` (created with owner-only permissions).
- The relay contacts no web service and collects no telemetry.
- `~/.agent-relay/sessions/` records each registered session's id, working
  folder, transcript path and process id so other sessions can address it.
  `inbox/*/delivered/` and `outbox/` keep message history until you delete them.
- `~/.agent-relay/sessions/antigravity-ls/` holds the loopback address and
  CSRF token of Antigravity IDE/CLI language servers, saved when an Antigravity
  agent runs the relay, so idle conversations can be woken. Files are mode 0600,
  the token is never printed or logged, and `relay gc` removes entries whose
  server is gone.

## Trust model

- Anyone who can run commands as your user on a computer, including over ssh,
  can already message your sessions there; the relay adds no network listener.
- A message queued into Codex becomes a user turn in that thread and runs with
  the thread's existing permissions. It cannot raise them.
- Claude Code receives relayed messages as hook context marked as coming from
  another agent session. The bundled skills tell the receiver to act only on
  requests that fit the work its user gave it and to bring anything risky back
  to the user. Do not rely on that as a security boundary against a hostile
  sender: control who can ssh into your computers.

## Reporting a vulnerability

Open a private security advisory on
[the repository](https://github.com/swack-tools/agent-relay-ai-plugin/security/advisories/new).
Do not include message contents, session ids, or credentials in public issues.
