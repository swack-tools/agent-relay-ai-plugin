import { buildGroupTree, findSpot, groupKey, packGroupTree } from './group.js';
import { computeAlerts, computeInsights, Health, matchesQuery, parseQuery, recordSample } from './insight.js';
import { displayName, heat, kindColor, nodeWeight, parseHex, truncate } from './util.js';

export const UNPOOLED = 'Unpooled';

export class AppState {
  constructor() {
    this.selfHost = '';
    this.agents = new Map();
    this.messages = [];
    this.msgIds = new Set();
    this.pools = [];
    this.hosts = [];
    this.nodes = new Map();
    this.regions = [];
    this.pulses = [];
    this.ripples = [];
    this.channels = [];
    this.drawnAs = new Map();
    this.selection = { type: 'none' };
    this.hover = null;
    this.hoverChannel = null;
    this.cam = { x: 0, y: 0, zoom: 0.55, tx: 0, ty: 0, tzoom: 0.55 };
    this.width = 1200;
    this.height = 800;
    this.dpr = 1;
    this.windowMs = 24 * 3600000;
    this.showUnpooled = true;
    this.showSwarm = true;
    this.expandSwarms = false;
    this.search = '';
    this.query = parseQuery('');
    this.replay = null;
    this.connected = false;
    this.layoutDirty = true;
    this.panelDirty = true;
    this.hudDirty = true;
    this.drawerDirty = true;
    this.feed = [];
    this.feedPaused = false;
    this.drawerHover = false;
    this.drag = null;
    this.firstSnapshot = true;
    this.lastLayout = 0;
    this.lastInsight = 0;
    this.names = new Map();
    this.levels = ['host', 'project', 'directory', 'task'];
    this.insights = new Map();
    this.alerts = [];
    this.tokHist = new Map();
    this.colorBy = 'kind';
    this.pins = new Set();
    this.drawer = null;
    this.sort = ['active', true];
    this.alertCursor = 0;
    this.openMsgs = new Set();
    this.broadcastPlan = null;
    this.broadcastResults = [];
    this.broadcastSending = false;
  }

  operator() {
    return `operator:relay-map@${this.selfHost}`;
  }

  poolColor(name) {
    const p = name ? this.pools.find((x) => x.name === name) : null;
    return p ? parseHex(p.color) : parseHex('#8892b0');
  }

  poolIsolated(name) {
    const p = name ? this.pools.find((x) => x.name === name) : null;
    return Boolean(p && p.isolated);
  }

  isCross(m) {
    if (m.source === 'swarm') return false;
    const pa = m.from_agent && this.agents.get(m.from_agent)?.pool;
    const pb = m.to_agent && this.agents.get(m.to_agent)?.pool;
    return Boolean(pa && pb && pa !== pb && (this.poolIsolated(pa) || this.poolIsolated(pb)));
  }

  name(addr) {
    if (this.names.has(addr)) return this.names.get(addr);
    const a = this.agents.get(addr);
    return a ? displayName(a) : addr;
  }

  insight(addr) {
    return this.insights.get(addr) || null;
  }

  ingestSnapshot(snap, isInitial = false) {
    if (!snap) return;
    this.connected = true;
    this.selfHost = snap.self_host || this.selfHost;
    this.hosts = Array.isArray(snap.hosts) ? snap.hosts : [];
    this.pools = Array.isArray(snap.pools) ? snap.pools : [];

    const incomingAgents = Array.isArray(snap.agents) ? snap.agents : [];
    const nextAgents = new Map();
    for (const a of incomingAgents) {
      nextAgents.set(a.addr, a);
      recordSample(this.tokHist, a);
    }
    this.agents = nextAgents;

    const incomingMsgs = Array.isArray(snap.messages) ? snap.messages : [];
    if (isInitial || this.firstSnapshot) {
      this.messages = [...incomingMsgs];
      this.msgIds = new Set(incomingMsgs.map((m) => m.id));
    } else {
      const now = Date.now();
      for (const m of incomingMsgs) {
        if (m.replaces && this.msgIds.has(m.replaces)) {
          this.msgIds.delete(m.replaces);
          this.messages = this.messages.filter((x) => x.id !== m.replaces);
        }
        if (!this.msgIds.has(m.id)) {
          this.msgIds.add(m.id);
          this.messages.push(m);
          if (now - m.sent_ms < 10 * 60000) {
            this.launch(m);
          }
        } else {
          const idx = this.messages.findIndex((x) => x.id === m.id);
          if (idx >= 0) this.messages[idx] = m;
        }
      }
      this.messages.sort((a, b) => a.sent_ms - b.sent_ms);
    }

    this.feed = this.messages.filter((m) => m.source !== 'swarm').slice(-14).reverse();
    this.layoutDirty = true;
    this.panelDirty = true;
    this.hudDirty = true;
    this.drawerDirty = true;

    if (this.firstSnapshot) {
      this.firstSnapshot = false;
      this.startReplay(36000);
    }
  }

  launch(m) {
    if (m.source === 'swarm' && !this.showSwarm) return;
    if (!m.from_agent || !m.to_agent) return;
    const f = this.drawnAs.get(m.from_agent);
    const t = this.drawnAs.get(m.to_agent);
    if (!f || !t) return;
    const nf = this.nodes.get(f);
    const nt = this.nodes.get(t);
    if (!nf || !nt) return;
    const dist = Math.hypot(nf.x - nt.x, nf.y - nt.y);
    const size = Math.max(2.5, Math.min(16.0, 2.0 + 1.7 * Math.log2(1 + (m.bytes || 0) / 150)));
    const cross = this.isCross(m);
    let color;
    if (cross) color = parseHex('#ff3b5c');
    else if (m.source === 'broadcast') color = parseHex('#ffffff');
    else if (m.source === 'swarm') color = parseHex('#7fd4ff');
    else {
      const pa = this.agents.get(m.from_agent)?.pool;
      color = pa ? this.poolColor(pa) : kindColor(this.agents.get(m.from_agent)?.kind);
    }
    const internal = f === t;
    this.pulses.push({
      from: f,
      to: t,
      t0: Date.now(),
      dur: internal ? 900 : 900 + dist * 1.4,
      size,
      color,
      cross,
      internal,
      swarm: m.source === 'swarm',
    });
    if (this.pulses.length > 400) this.pulses.shift();
  }

  startReplay(spanReal = 36000) {
    this.startReplayFrom(Date.now() - this.windowMs, spanReal);
  }

  startReplayFrom(from, spanReal = 36000) {
    const now = Date.now();
    const queue = this.messages.filter((m) => m.sent_ms >= from && (this.showSwarm || m.source !== 'swarm'));
    if (!queue.length) {
      this.replay = null;
      return;
    }
    const first = queue[0].sent_ms ?? from;
    this.replay = { startReal: now, spanReal, fromMs: first, toMs: now, next: 0, queue };
  }

  tickReplay() {
    const r = this.replay;
    if (!r) return null;
    const frac = Math.max(0, Math.min(1, (Date.now() - r.startReal) / r.spanReal));
    const sim = Math.round(r.fromMs + (r.toMs - r.fromMs) * frac);
    const due = [];
    while (r.next < r.queue.length && r.queue[r.next].sent_ms <= sim) {
      due.push(r.queue[r.next++]);
    }
    for (const m of due.slice(-60)) this.launch(m);
    const lastNonSwarm = [...due].reverse().find((m) => m.source !== 'swarm');
    if (lastNonSwarm && !this.feed.some((f) => f.id === lastNonSwarm.id)) {
      this.feed.unshift(lastNonSwarm);
      if (this.feed.length > 14) this.feed.length = 14;
      this.hudDirty = true;
    }
    if (frac >= 1) this.replay = null;
    return sim;
  }

  layout() {
    this.layoutDirty = false;
    this.lastLayout = Date.now();
    const now = this.lastLayout;
    const op = this.operator();

    const talked = new Set();
    for (const m of this.messages) {
      if (now - m.sent_ms >= this.windowMs) continue;
      if (m.source === 'swarm' && !this.showSwarm) continue;
      if (m.from_agent) talked.add(m.from_agent);
      if (m.to_agent) talked.add(m.to_agent);
    }

    const visible = new Set();
    for (const a of this.agents.values()) {
      if (a.addr === op) {
        visible.add(a.addr);
        continue;
      }
      const active = (!a.ghost && (a.alive === true || now - (a.last_active_ms || 0) < this.windowMs)) || talked.has(a.addr);
      const configured = Boolean(a.pool && this.pools.some((p) => p.name === a.pool && !p.auto));
      const poolOk = this.showUnpooled || configured;
      if (active && poolOk) visible.add(a.addr);
    }

    this.drawnAs.clear();
    const children = new Map();
    for (const addr of visible) {
      let cur = addr;
      if (!this.expandSwarms) {
        let hops = 0;
        while (hops <= 8) {
          const parent = this.agents.get(cur)?.parent;
          if (!parent || !this.agents.has(parent)) break;
          cur = parent;
          hops++;
        }
      }
      if (cur !== addr) {
        if (!children.has(cur)) children.set(cur, []);
        children.get(cur).push(addr);
      }
      this.drawnAs.set(addr, cur);
    }

    const top = [...new Set(this.drawnAs.values())].sort();
    for (const t of top) this.drawnAs.set(t, t);

    const members = top.filter((a) => a !== op);
    const keyf = (lvl, addr) => {
      const ins = this.insights.get(addr);
      if (lvl === 'status' && ins) {
        const meta = ['looping', 'stuck', 'waiting', 'working', 'idle', 'ended', 'messages only'][ins.health] || 'idle';
        const col = ['#ff4fd8', '#ff5a36', '#ffd166', '#46f5a0', '#6f86c4', '#4f5a7a', '#4f5a7a'][ins.health];
        return [String(ins.health), meta, parseHex(col)];
      }
      return groupKey(lvl, this.agents.get(addr), this.pools, now);
    };
    const agentR = (addr) => {
      const a = this.agents.get(addr);
      return Math.max(7, Math.min(22, 7 + 2.4 * Math.log10(1 + nodeWeight(a?.tokens) / 20000)));
    };

    const root = buildGroupTree(members, this.levels, keyf);
    packGroupTree(root, agentR);

    if (!root.children.length) {
      root.x = 0;
      root.y = root.r + 90;
    } else {
      const order = root.children.map((_, i) => i).sort((a, b) => root.children[b].r - root.children[a].r);
      const placed = [[0, 0, 70]];
      for (const i of order) {
        const r = root.children[i].r;
        const [x, y] = findSpot(placed, r, 40);
        root.children[i].x = x;
        root.children[i].y = y;
        placed.push([x, y, r]);
      }
    }

    const golden = Math.PI * (3 - Math.sqrt(5));
    const outRegions = [];
    const spots = [];
    const walk = (n, ox, oy, depth, path) => {
      const x = ox + n.x;
      const y = oy + n.y;
      const key = `${path}›${n.label}`;
      let count = n.agents.length;
      const idx = outRegions.length;
      const lone = depth > 1 && !n.children.length && n.agents.length === 1;
      if (depth > 0 && !lone) {
        const isolated = n.level === 'pool' && this.pools.some((p) => p.name === n.label && p.isolated);
        outRegions.push({ key, label: n.label, color: n.color, depth, x, y, r: n.r, members: 0, isolated });
      }
      const k = n.agents.length;
      let h = 2166136261 >>> 0;
      for (let i = 0; i < key.length; i++) h = Math.imul(h ^ key.charCodeAt(i), 16777619) >>> 0;
      for (let j = 0; j < k; j++) {
        const rr = k <= 1 ? 0 : (n.r - 20) * Math.sqrt((j + 0.5) / k);
        const th = j * golden + (h % 360);
        spots.push([n.agents[j], x + rr * Math.cos(th), y + rr * Math.sin(th)]);
      }
      for (const c of n.children) count += walk(c, x, y, depth + 1, key);
      if (depth > 0 && !lone) outRegions[idx].members = count;
      return count;
    };
    walk(root, 0, 0, 0, '');

    for (const [a, x, y] of spots) this.place(a, x, y);
    const neutral = [parseHex('#7c8db5'), parseHex('#9aa7d6'), parseHex('#b6c1e6'), parseHex('#cdd6f4')];
    this.regions = outRegions.map((reg) => {
      const old = this.regions.find((o) => o.name === reg.key);
      return {
        name: reg.key,
        color: reg.color || neutral[Math.min(3, reg.depth - 1)],
        x: old ? old.x : reg.x,
        y: old ? old.y : reg.y,
        r: old ? old.r : reg.r,
        tr: reg.r,
        tx: reg.x,
        ty: reg.y,
        members: reg.members,
        isolated: reg.isolated,
        label: reg.label,
        depth: reg.depth,
      };
    });

    if (this.agents.has(op)) this.place(op, 0, 0);
    const keep = new Set(top);
    for (const k of [...this.nodes.keys()]) {
      if (!keep.has(k)) this.nodes.delete(k);
    }
    for (const [k, n] of this.nodes.entries()) {
      n.children = children.get(k) || [];
      const a = this.agents.get(k);
      const tok = nodeWeight(a?.tokens);
      n.r = a?.kind === 'operator' ? 16 : Math.max(7, Math.min(22, 7 + 2.4 * Math.log10(1 + tok / 20000)));
      n.r += Math.min(6, Math.sqrt(n.children.length));
    }
    this.computeNames();
    this.computeChannels();
    this.refreshInsights();
  }

  place(addr, x, y) {
    let n = this.nodes.get(addr);
    if (!n) {
      n = { x, y, tx: x, ty: y, r: 8, children: [], born: Date.now() };
      this.nodes.set(addr, n);
    } else {
      n.tx = x;
      n.ty = y;
    }
  }

  computeNames() {
    const base = new Map();
    for (const addr of this.drawnAs.keys()) {
      const a = this.agents.get(addr);
      if (!a) continue;
      const dn = displayName(a);
      if (!base.has(dn)) base.set(dn, []);
      base.get(dn).push(addr);
    }
    this.names.clear();
    for (const [dn, addrs] of base.entries()) {
      const dup = addrs.length > 1;
      for (const addr of addrs) {
        const a = this.agents.get(addr);
        let n = dn;
        if (dup && !a.label) {
          if (a.title) {
            const words = a.title.trim().split(/\s+/).slice(0, 3).join(' ');
            n = `${dn} · ${truncate(words, 22)}`;
          } else {
            n = `${dn} ${String(a.id || '').slice(0, 6)}`;
          }
        }
        this.names.set(addr, n);
      }
    }
  }

  computeChannels() {
    const now = Date.now();
    const map = new Map();
    for (const m of this.messages) {
      if (now - m.sent_ms >= this.windowMs) continue;
      if (m.source === 'swarm' && !this.showSwarm) continue;
      if (!m.from_agent || !m.to_agent) continue;
      const f = this.drawnAs.get(m.from_agent);
      const t = this.drawnAs.get(m.to_agent);
      if (!f || !t || f === t) continue;
      const fwd = f < t;
      const a = fwd ? f : t;
      const b = fwd ? t : f;
      const key = `${a}\0${b}`;
      let e = map.get(key);
      if (!e) {
        e = { a, b, count: 0, bytes: 0, aToB: 0, lastMs: 0, cross: false, swarmOnly: true, color: parseHex('#9fb4ff') };
        map.set(key, e);
      }
      e.count++;
      e.bytes += m.bytes || 0;
      e.lastMs = Math.max(e.lastMs, m.sent_ms);
      if (this.isCross(m)) e.cross = true;
      if (fwd) e.aToB++;
      if (m.source !== 'swarm') e.swarmOnly = false;
    }
    const chans = [...map.values()];
    for (const c of chans) {
      const pa = this.agents.get(c.a)?.pool;
      const pb = this.agents.get(c.b)?.pool;
      if (c.cross) c.color = parseHex('#ff3b5c');
      else if (pa && pa === pb) c.color = this.poolColor(pa);
      else if (c.swarmOnly) c.color = parseHex('#7fd4ff');
      else c.color = parseHex('#c8d3ff');
    }
    chans.sort((a, b) => a.count - b.count);
    this.channels = chans;
  }

  refreshInsights() {
    this.lastInsight = Date.now();
    this.insights = computeInsights(this);
    this.alerts = computeAlerts(this);
    this.drawerDirty = true;
  }

  insets() {
    const left = this.drawer ? Math.min(this.width, 860) : 0;
    const right = this.selection.type !== 'none' ? Math.min(this.width, 500) : 0;
    return [left, right];
  }

  orderedMatches() {
    return [...this.nodes.keys()]
      .filter((k) => {
        const a = this.agents.get(k);
        return a && a.kind !== 'operator' && this.matchesSearch(a);
      })
      .sort((x, y) => (this.agents.get(y)?.last_active_ms || 0) - (this.agents.get(x)?.last_active_ms || 0));
  }

  matchesSearch(a) {
    if (this.query.isEmpty) return true;
    return matchesQuery(this.query, a, this.insights.get(a.addr), this.name(a.addr));
  }

  setSearch(s) {
    this.search = s || '';
    this.query = parseQuery(this.search);
    this.drawerDirty = true;
    this.hudDirty = true;
  }

  nodeColor(addr) {
    const a = this.agents.get(addr);
    if (!a || a.kind === 'operator') return kindColor('operator');
    const ins = this.insights.get(addr);
    if (this.colorBy === 'health' && ins) {
      return ['#ff4fd8', '#ff5a36', '#ffd166', '#46f5a0', '#6f86c4', '#4f5a7a', '#4f5a7a'][ins.health];
    }
    if (this.colorBy === 'spend' && ins) {
      return ins.cost !== null ? heat(ins.cost, 0.5, 200.0) : heat(tokensFresh(a.tokens) / 1e6, 1.0, 400.0);
    }
    return kindColor(a.kind);
  }

  messagesBetween(a, b) {
    return this.messages.filter((m) => {
      const f = m.from_agent && (this.drawnAs.get(m.from_agent) || m.from_agent);
      const t = m.to_agent && (this.drawnAs.get(m.to_agent) || m.to_agent);
      return (f === a && t === b) || (f === b && t === a);
    });
  }

  messagesOf(addr) {
    return this.messages.filter((m) => m.from_agent === addr || m.to_agent === addr);
  }

  worldToScreen(x, y) {
    return [(x - this.cam.x) * this.cam.zoom + this.width / 2, (y - this.cam.y) * this.cam.zoom + this.height / 2];
  }

  screenToWorld(x, y) {
    return [(x - this.width / 2) / this.cam.zoom + this.cam.x, (y - this.height / 2) / this.cam.zoom + this.cam.y];
  }

  fitAll() {
    let minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
    for (const r of this.regions) {
      minx = Math.min(minx, r.tx - r.tr);
      maxx = Math.max(maxx, r.tx + r.tr);
      miny = Math.min(miny, r.ty - r.tr - 30);
      maxy = Math.max(maxy, r.ty + r.tr);
    }
    if (minx === Infinity) {
      minx = -200; maxx = 200; miny = -200; maxy = 200;
    }
    const w = Math.max(200, maxx - minx) + 160;
    const h = Math.max(200, maxy - miny) + 220;
    const [li] = this.insets();
    const avail = Math.max(200, this.width - li);
    this.cam.tzoom = Math.max(0.15, Math.min(1.6, Math.min(avail / w, this.height / h)));
    this.cam.tx = (minx + maxx) / 2 - (li / 2) / this.cam.tzoom;
    this.cam.ty = (miny + maxy) / 2 - 30 / this.cam.tzoom;
  }

  focus(addr) {
    const key = this.drawnAs.get(addr) || addr;
    const n = this.nodes.get(key);
    if (n) {
      this.cam.tzoom = 2.1;
      const li = this.drawer ? Math.min(this.width, 860) : 0;
      const ri = Math.min(this.width, 500);
      this.cam.tx = n.tx + ((ri - li) / 2) / 2.1;
      this.cam.ty = n.ty;
    }
    this.selection = { type: 'agent', addr };
    this.panelDirty = true;
  }

  focusChannel(a, b) {
    const na = this.nodes.get(a);
    const nb = this.nodes.get(b);
    if (na && nb) {
      const d = Math.hypot(na.tx - nb.tx, na.ty - nb.ty);
      this.cam.tzoom = Math.max(0.3, Math.min(2.0, (this.width * 0.5) / (d + 200)));
      this.cam.tx = (na.tx + nb.tx) / 2 + (this.width * 0.17) / this.cam.tzoom;
      this.cam.ty = (na.ty + nb.ty) / 2;
    }
    this.selection = { type: 'channel', a, b };
    this.panelDirty = true;
  }
}
