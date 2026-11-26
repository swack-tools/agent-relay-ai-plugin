#!/usr/bin/env python3
"""Stand-in for agent-relay's relay.py inside the container.

The hub broadcasts with `python3 <relay_py> send ...`. The container has no
agent-relay install and no Claude or Codex sessions of its own, so this runs the
same command with the relay.py installed on another computer, over ssh. That
computer then delivers exactly as it would for a local `relay send` (and
reaches other computers over its own ssh, as agent-relay always does).

RELAY_MAP_BROADCAST_VIA names the ssh host (an alias from ~/.ssh/config).
"""
import os
import shlex
import sys

host = os.environ.get("RELAY_MAP_BROADCAST_VIA", "").strip()
if not host:
    sys.stderr.write("broadcast is off: RELAY_MAP_BROADCAST_VIA is not set\n")
    sys.exit(2)

# Newest installed plugin version, as the hub itself picks it on a desktop.
find = ('r=$(ls -d "$HOME"/.claude/plugins/cache/agent-relay/agent-relay/*/scripts/relay.py '
        '2>/dev/null | sort -V | tail -n 1); '
        '[ -n "$r" ] || { echo "agent-relay is not installed on $(hostname)" >&2; exit 2; }; ')
remote = find + 'exec python3 "$r" ' + " ".join(shlex.quote(a) for a in sys.argv[1:])
os.execvp("ssh", ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, remote])
