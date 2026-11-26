# Relay Map

A live, attack-map style view of what your agents are saying to each other over
agent-relay. Rust end to end: a collector and hub (`relay-map`) and a
WebAssembly front end (`relay-map-web`) that draws on a canvas.

## What it shows

- **Agents** from Claude Code, Codex, Antigravity and pi, on this computer and on
  any computer you reach with `ssh`. Node size follows token use, the rim dot is
  the computer, a spinner means it is working now.
- **Pulses** for every relay message: a bolt of light from sender to receiver,
  bigger and longer for bigger messages, with a ripple when it lands.
- **Channels** between agents that keep talking: a glowing arc whose dashes flow
  in the main direction. Click one for the whole conversation and what both
  sides are working on.
- **Pools**: groups of agents that work together (for example backend and frontend sessions). Pools marked `isolated` flag any message that leaves them in red.
- **Agent detail**: click an agent and the map zooms in. Its sub-agents, threads
  and Antigravity swarm members orbit it, and the panel lists its task, what it
  is doing right now, token counts and cost, files it created or edited, git and
  GitHub actions, skills, tools and relay messages.
- **Broadcast**: send one message to every live agent, a pool, or one agent.
  It goes through agent-relay's own `relay send`, so delivery is exactly what
  the plugin does (Claude mailboxes and hooks, `codex queue`, Antigravity's
  servers). Preview shows the recipient list first, and sending needs a second
  click.
- **Replay** compresses the selected window (1 h to 72 h) into 36 seconds of
  traffic, so a quiet map still tells you what happened.

## Where the data comes from

Nothing is sent anywhere except between your own computers over ssh. The
collector reads, read-only:

| Source | Gives |
| --- | --- |
| `~/.agent-relay/sessions/*.json` | registered sessions, labels, busy/idle, whether the process is alive |
| `~/.agent-relay/inbox/**` | every delivered relay envelope (sender, recipient, subject, body) |
| `~/.claude/projects/**.jsonl` (+ `subagents/`) | Claude tasks, tool calls, files, git, skills, tokens, cost, sub-agents |
| `~/.codex/sessions/**/rollout-*.jsonl` | Codex goals, commands, patches, tokens, spawned agents |
| `~/.gemini/antigravity*/brain/*/…/transcript.jsonl` | Antigravity tasks, tools, tokens, swarm messages between conversations |
| `~/.pi/agent/sessions/**.jsonl` | pi sessions (observed only; pi is not an agent-relay target) |

Messages are matched from three sides: the inbox envelope, the
`<agent-relay-message>` block in the receiver's transcript, and the
`relay send` command in the sender's transcript, so traffic to computers you do
not monitor still shows up (as a dashed "ghost" node).

## Run it

```sh
./build.sh                      # wasm front end, native hub, Linux collectors
cp relay-map.example.toml relay-map.toml   # add your ssh hosts and pools
./target/release/relay-map serve
# open http://127.0.0.1:7878/
```

`build.sh` needs `rustup target add wasm32-unknown-unknown` and
`cargo install wasm-bindgen-cli --version 0.2.100`. Linux collectors for remote
servers are cross-compiled with `cargo zigbuild` when it is installed
(`SKIP_CROSS=1` skips them).

For each `[[hosts]]` entry the hub runs `uname` over ssh, copies the matching
collector from `bin/` to `~/.cache/relay-map/` there (once per build), and
streams `relay-map collect` from it. A host whose platform has no binary shows
the reason in its header chip and is retried.

`relay-map collect --once` prints one computer's events as JSON lines, which is
handy for checking what the parsers see.

Keys: `Esc` zoom out, `b` broadcast, `r` replay, `f` fit.

## Layout

```
crates/relay-map-core   wire types shared by hub and front end
crates/relay-map        collector (per-agent parsers), hub, ssh remotes, WebSocket, broadcast
crates/relay-map-web    WebAssembly front end (canvas drawing, panels)
web/                    index.html and style.css
```

The plugin itself is unchanged; Relay Map only reads its files and calls
`relay.py send`.

## Operator views

- **Health** per agent: working (active in the last 2 min), waiting (process alive but quiet, likely waiting for you), stuck (marked busy, silent 15 min), looping (relay ping-pong of 8+ messages in 15 min, or heavy edits to one file), idle, ended. Stuck and looping agents get a pulsing badge; waiting ones a dashed amber ring. The header counts each state; click a count to filter.
- **Attention** (`a`, ⚠ button): stuck, looping, long-waiting and high-spend agents, cross-pool traffic in the last hour, unreachable computers. `n` jumps through them on the map; the tab title shows how many need you now.
- **Fleet table** (`t`): every agent with status, last active, tokens, tokens/min, cost and counts; sortable, star to watch.
- **Message log** (`m`): every relay message in the window, searchable, expandable to the full body; scoped to the selected agent.
- **Cost and burn**: the agent's recorded cost, or an estimate from Claude list prices (cache reads 0.1x, writes 1.25x input); tokens/min and $/hour measured since the page opened.
- **Search** (`/`): words or filters `kind:` `host:` `status:` `project:` `pool:` `model:` `has:`; Enter zooms to the best match, `j`/`k` step through matches.
- **Colour by** (`c`): agent type, health, or spend.
- **Watch list** (`w` or ☆): starred agents stay pinned top left with live status.
- **Traffic timeline**: message volume over the window (cross-pool in red); click a bar to replay from there. The agent panel adds an activity strip (messages, files, git, sub-agents) and a 30-minute burn sparkline.
- **Spend** (`s`): cost split by project, computer, model and agent type, with the rate per hour right now; click a row to filter the map.
- **Desktop alerts**: opt in from the attention tab; a notification fires when an agent turns stuck or looping or starts waiting, and clicking it focuses the agent.
- **Agent panel actions**: open the agent's folder in VS Code (over ssh for other computers), copy a `claude --resume` / `codex resume` command, copy the folder or the relay address.
- **Links**: the address bar keeps `q`, `tab`, `color` and `agent`, so a reload or a shared link opens the same view.
- `?` lists every shortcut.
