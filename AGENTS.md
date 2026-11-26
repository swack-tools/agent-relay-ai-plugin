# Repository guidance

- `plugins/agent-relay` is the canonical plugin package. Root manifests and the
  marketplace files exist for discovery; keep the Claude
  (`.claude-plugin/plugin.json`) and Codex (`.codex-plugin/plugin.json`) manifests
  consistent in name, version, description and license. Do not add a portable
  root `plugin.json`: with one present, Codex 0.169 silently skips the plugin's
  hooks. Antigravity reads `.antigravity-plugin/plugin.json` and only the root
  `hooks.json`, written in its named-group schema with `--agent antigravity`
  commands. Never put Claude or Codex hooks in the root `hooks.json`: without
  that manifest, `agy plugin install` converts the Claude hooks verbatim and its
  CLI would run the Claude wake-up watcher.
- Antigravity delivery: desktop-app conversations get `agentapi send-message`
  through the app's server (port and CSRF token from
  `~/.gemini/antigravity/daemon/hub_*.json`); CLI and IDE conversations get `agentapi send-message` through their own
  language server when relay.py has cached its address and token (saved, 0600,
  whenever an Antigravity agent runs relay.py), else a file in Antigravity's own
  message store, read at their next turn. Never print or log a CSRF token.
- `scripts/relay.py` must stay a single standard-library file that runs on
  Python 3.9, because it is streamed to remote computers over
  `ssh HOST python3 -`.
- Claude hooks live in `hooks/hooks.json` (loaded by default, so do not also list
  it in the Claude manifest). Codex hooks live in `hooks/codex-hooks.json`,
  referenced from the Codex manifest. Hooks must never fail a turn.
- Relayed messages are framed as coming from a peer agent, not from the user.
  Do not change the framing to make a receiving agent treat them as user
  instructions, and keep the skills' rule that risky or out-of-scope requests go
  back to the user.
- Never commit credentials, host names of real machines, session ids, or
  message contents. Tests use temporary `AGENT_RELAY_HOME` folders.
- Run `python3 scripts/check_plugin.py`, `python3 -m unittest discover -s tests -v`
  and `claude plugin validate --strict plugins/agent-relay` before committing,
  and report which checks ran.
