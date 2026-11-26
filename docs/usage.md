# Agent Relay usage

## Client usage

- **Antigravity**: Install with `agy plugin install https://github.com/swack-tools/agent-relay-ai-plugin`. Open **Agent Relay** from the left sidebar under **Plugins**, **Relay Map** from the side pane `+` menu, or invoke `/list-sessions`, `/message-session`, `/relay-map`, `/handle-relay-message`, and `/relay-setup`.
- **Claude Code**: Install from `swack-tools/agent-relay-ai-plugin` and invoke `/agent-relay:relay-map`, `/agent-relay:list-sessions`, `/agent-relay:message-session`, or `/agent-relay:handle-relay-message`.
- **Codex**: Install from `swack-tools/agent-relay-ai-plugin`, trust hooks (`t`), and invoke `$relay-map`, `$list-sessions`, `$message-session`, or `$relay-setup`.
- **opencode**: Add with `opencode plugin add 'github:swack-tools/agent-relay-ai-plugin#main::path:plugins/agent-relay'`.
- **pi**: Install with `pi install git:github.com/swack-tools/agent-relay-ai-plugin`.

See the [installation and multi-host SSH guide](https://relay.ai.swacktech.com/install.html) for full setup instructions.

## Kubernetes deployment

To run the standalone Rust/WASM Relay Map hub in a Kubernetes cluster, see `web-ui/deploy/k8s/README.md`.
