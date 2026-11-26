# Agent Relay plugin

Agent Relay lets Claude Code, Codex, Antigravity, opencode and pi sessions talk to each other, in the desktop apps or the terminal, on one computer or across computers over `ssh`. A Claude session can queue a prompt into a running Codex window on another Mac, an opencode session can ask a pi session on a server a question, and any of them can read the others' replies. The canonical plugin package is [`plugins/agent-relay`](plugins/agent-relay/README.md).

This is an independent project by SwackTech LLC. It is not endorsed by Anthropic, OpenAI, Google or the opencode and pi projects.

## Contents

- [Install](#install)
  - [Claude Code](#claude-code)
  - [Antigravity (app, IDE and agy CLI)](#antigravity-app-ide-and-agy-cli)
  - [Codex (app and CLI)](#codex-app-and-cli)
  - [pi](#pi)
  - [opencode](#opencode)
  - [Check the install](#check-the-install)
- [Use it](#use-it)
- [Develop](#develop)
- [License](#license)

## Install

Install on every computer whose sessions should receive messages. To only send to another computer, ssh access and `python3` there are enough. Every agent needs `python3` 3.9 or newer.

### Claude Code

```sh
claude plugin marketplace add swack-tools/agent-relay-ai-plugin
claude plugin install agent-relay@agent-relay
```

Restart open sessions afterwards: the hooks register a session when it starts.

### Antigravity (app, IDE and agy CLI)

```sh
agy plugin install https://github.com/swack-tools/agent-relay-ai-plugin
```

The plugin gives Antigravity agents the skills and a SessionStart hook that tells each conversation its relay address. Idle desktop-app conversations are woken right away; IDE and CLI conversations see messages at their next turn unless they have run a relay command before.

### Codex (app and CLI)

```sh
codex plugin marketplace add swack-tools/agent-relay-ai-plugin
codex plugin add agent-relay@agent-relay
```

Codex asks you to review and trust plugin hooks before running them (open `codex` and press `t` to trust them). They only record the session's relay address; queued messages reach Codex without them.

### pi

```sh
pi install git:github.com/swack-tools/agent-relay-ai-plugin
```

From a local clone, `pi install <clone>/plugins/agent-relay` does the same. The package adds the skills and an extension that registers each session, tells the model its relay address, and delivers relayed messages, starting a turn when the session is idle. Restart open pi sessions (or run `/reload`) to load it.

### opencode

```sh
opencode plugin add 'github:swack-tools/agent-relay-ai-plugin#main::path:plugins/agent-relay'
```

From a local clone, add the folder to `plugins` in `~/.config/opencode/opencode.json` instead:

```json
{
  "plugins": ["<clone>/plugins/agent-relay"]
}
```

Messages reach opencode sessions through the opencode background service, so any session the service knows about can receive them; the plugin adds each session's relay address to its instructions so it can reply. The service reloads plugins on its own; if the plugin does not show up, run `opencode service restart` while no session is working.

### Check the install

```sh
python3 <plugin folder>/scripts/relay.py doctor
python3 <plugin folder>/scripts/relay.py sessions
```

`doctor` reports each agent it finds on the computer, and `sessions` lists the live sessions of all five. Inside an agent, just ask it to list agent sessions.

To put a `relay` command on your PATH (handy in a shell, and used in reply hints when a message was sent from another computer), run:

```sh
python3 <plugin folder>/scripts/relay.py install-cli
```

It writes a small `~/.local/bin/relay` script that runs that copy of `relay.py`.

## Use it

Ask in plain language, for example `List the agent sessions running on laptop` or `Open the Relay Map dashboard`. In Antigravity Desktop App, **Agent Relay** appears in the left sidebar under **Plugins** and **Relay Map** appears in the side pane `+` menu. In Claude Code the skills also appear as `/agent-relay:relay-map`, `/agent-relay:message-session` and friends. See the [plugin README](plugins/agent-relay/README.md) for routes, MCP tools, commands, and limits.

## Relay Map & Kubernetes

- **Embedded MCP App**: The bundled `relay` MCP server (`plugins/agent-relay/scripts/relay_mcp_server.py`) serves the self-contained `ui://agent-relay/relay-map.html` pew-pew dashboard inside Antigravity, Claude Code, and Codex.
- **Standalone Rust/WASM hub & Kubernetes**: [`web-ui/`](web-ui/README.md) provides the standalone `relay-map` collector/hub and [`web-ui/deploy/k8s/`](web-ui/deploy/k8s/README.md) runs it in Kubernetes behind Envoy Gateway.

## Develop

```sh
python3 scripts/check_plugin.py
python3 -m unittest discover -s tests -v
npx --yes @anthropic-ai/claude-code plugin validate --strict plugins/agent-relay
```

`tests/test_live_codex.py` starts a real Codex turn, so it runs only with `AGENT_RELAY_LIVE_CODEX=1` against a throwaway daemon (CI does this); never point it at the daemon your own Codex windows use.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
