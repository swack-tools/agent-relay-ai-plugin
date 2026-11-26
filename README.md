# Agent Relay plugin

[Documentation](https://relay.ai.swacktech.com/) · [Relay Map UI](https://relay.ai.swacktech.com/relay-map.html) · [Install guide](https://relay.ai.swacktech.com/install.html) · [Workflows](https://relay.ai.swacktech.com/workflows.html) · [Technical reference](https://relay.ai.swacktech.com/reference.html) · [Plugin marketplace](https://ai.swacktech.com/plugins/agent-relay/)

Agent Relay lets Claude Code, Codex, Antigravity, opencode and pi sessions talk to each other, in the desktop apps or the terminal, on one computer or across computers over `ssh`. A Claude session can queue a prompt into a running Codex window on another Mac, an opencode session can ask a pi session on a server a question, and any of them can read the others' replies. The canonical plugin package is [`plugins/agent-relay`](plugins/agent-relay/README.md).

This is an independent project by SwackTech LLC. It is not endorsed by Anthropic, OpenAI, Google or the opencode and pi projects.

![Relay Map orbital fleet view showing live hosts, 14 agents across five runtimes, glowing message arcs, and traffic timeline](site/assets/screenshots/01-orbital-fleet-map.png)

## Contents

- [Install](#install)
  - [Claude Code](#claude-code)
  - [Antigravity (app, IDE and agy CLI)](#antigravity-app-ide-and-agy-cli)
  - [Codex (app and CLI)](#codex-app-and-cli)
  - [pi](#pi)
  - [opencode](#opencode)
  - [Check the install](#check-the-install)
- [Relay Map MCP App & Visual Tour](#relay-map-mcp-app--visual-tour)
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

## Relay Map MCP App & Visual Tour

The bundled `relay` MCP server (`plugins/agent-relay/scripts/relay_mcp_server.py`) serves the self-contained `ui://agent-relay/relay-map.html` pew-pew dashboard directly inside Antigravity, Claude Code, and Codex alongside 7 model-visible tools (`relay_map`, `relay_sessions`, `relay_send`, `relay_inbox`, `relay_read_reply`, `relay_broadcast`, `relay_transcript`).

<table>
  <tr>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/02-agent-inspector-panel.png"><img src="site/assets/screenshots/02-agent-inspector-panel.png" alt="Agent Inspector Panel"></a>
      <br><strong>Agent Inspector Panel</strong><br>
      <sub>Click any node to zoom in and inspect current focus, token split bar, 10-minute burn rate, 24h 4-lane activity matrix, subagents, files, skills, and one-click <em>Session log</em> / <em>Ask in chat</em> actions.</sub>
    </td>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/03-channel-conversation.png"><img src="site/assets/screenshots/03-channel-conversation.png" alt="Cross-Machine Channel Conversation"></a>
      <br><strong>Cross-Machine Channel Conversation</strong><br>
      <sub>Click any Bezier message arc or feed row to zoom between hosts, compare peer status cards side-by-side, and read the full relayed conversation thread.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/04-fleet-table-drawer.png"><img src="site/assets/screenshots/04-fleet-table-drawer.png" alt="Sortable Fleet Table"></a>
      <br><strong>Sortable Fleet Table (<code>t</code>)</strong><br>
      <sub>Compare every live session across Claude, Codex, Antigravity, opencode, and pi by status, fresh/cache tokens, <code>tok/min</code>, 15-minute sparklines, and USD cost with structured search filters.</sub>
    </td>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/05-attention-health-alerts.png"><img src="site/assets/screenshots/05-attention-health-alerts.png" alt="Health & Attention Alerts"></a>
      <br><strong>Health &amp; Attention Alerts (<code>a</code> / <code>n</code>)</strong><br>
      <sub>Surfaces looping ping-pong or hot-file edits, stuck busy sessions, long-waiting prompts, high-burn alerts, and cross-pool leaks between isolated pools.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/06-spend-cost-analytics.png"><img src="site/assets/screenshots/06-spend-cost-analytics.png" alt="Spend & Cost Analytics"></a>
      <br><strong>Spend &amp; Cost Analytics (<code>s</code>)</strong><br>
      <sub>Track total USD spend and live <code>$/hour</code> burn broken down by project, computer, model, and agent runtime, with click-to-filter thermal node coloring.</sub>
    </td>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/07-session-transcript-viewer.png"><img src="site/assets/screenshots/07-session-transcript-viewer.png" alt="Session Transcript Viewer"></a>
      <br><strong>Session Transcript Viewer</strong><br>
      <sub>Read any local or remote agent's JSONL transcript inline via <code>relay_transcript</code> with collapsible <code>thinking</code> and <code>tool</code> rows, filter toggles, and raw tail copy.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/08-broadcast-preview-modal.png"><img src="site/assets/screenshots/08-broadcast-preview-modal.png" alt="Broadcast Preview Modal"></a>
      <br><strong>Two-Step Broadcast Modal (<code>b</code>)</strong><br>
      <sub>Target every live agent, a pool, or a single session with mandatory dry-run recipient preview (<code>relay_broadcast</code>) and armed confirmation before sending.</sub>
    </td>
    <td width="50%" valign="top">
      <a href="site/assets/screenshots/09-hierarchy-grouping-swarms.png"><img src="site/assets/screenshots/09-hierarchy-grouping-swarms.png" alt="Hierarchy Grouping & Expanded Swarms"></a>
      <br><strong>Hierarchy Grouping &amp; Swarms (<code>g</code>)</strong><br>
      <sub>Reorder concentric rings (<code>Pool</code>, <code>Computer</code>, <code>Project</code>, <code>Folder</code>, <code>Task</code>, <code>Agent type</code>, <code>Status</code>) and expand Antigravity swarm workers individually.</sub>
    </td>
  </tr>
</table>

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
