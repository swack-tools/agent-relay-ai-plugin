import { hostColorHex, kindColor, parseHex, projectOf, shortPath, truncate } from './util.js';

export const ALL_LEVELS = ['host', 'project', 'directory', 'task', 'pool', 'kind', 'status'];

export function levelTitle(level) {
  switch (level) {
    case 'host': return 'Computer';
    case 'pool': return 'Pool (from relay-map.toml)';
    case 'project': return 'Project (repository)';
    case 'directory': return 'Folder';
    case 'kind': return 'Agent type';
    case 'task': return 'Current task';
    case 'status': return 'Status';
    default: return level;
  }
}

export function groupKey(level, a, pools, now) {
  switch (level) {
    case 'host':
      return [a.host || '', a.host || '', parseHex(hostColorHex(a.host))];
    case 'pool': {
      const p = a.pool ? pools.find((x) => x.name === a.pool && !x.auto) : null;
      return p ? [p.name, p.name, parseHex(p.color)] : ['~none', 'no pool', null];
    }
    case 'project': {
      const p = a.ghost ? 'seen in messages' : projectOf(a.cwd || '');
      return [p, p, null];
    }
    case 'directory': {
      const c = a.cwd || '';
      return [c, c ? shortPath(c, 2) : '(no folder)', null];
    }
    case 'kind': {
      const k = a.kind || 'other';
      const label = k === 'codex' ? 'openai codex' : k;
      return [k, label, kindColor(a.kind)];
    }
    case 'task': {
      const t = a.title || a.focus || a.last_text || '';
      if (!t) return ['~', '(no task recorded)', null];
      const words = t.trim().split(/\s+/).slice(0, 5);
      return [words.map((w) => w.toLowerCase()).join(' '), truncate(words.join(' '), 40), null];
    }
    case 'status': {
      if (a.ghost) return ['3', 'seen in messages', null];
      if (a.alive === true || now - (a.last_active_ms || 0) < 5 * 60 * 1000) return ['0', 'active now', parseHex('#46f5a0')];
      if (a.alive === false) return ['2', 'ended', parseHex('#8892b0')];
      return ['1', 'idle', parseHex('#ffd166')];
    }
    default:
      return ['', '', null];
  }
}

export function buildGroupTree(members, levels, keyf, label = '', color = null, level = null) {
  if (!levels.length) {
    return { label, color, children: [], agents: members, r: 0, x: 0, y: 0, level };
  }
  const [first, ...rest] = levels;
  const groups = new Map();
  for (const m of members) {
    const [k, l, c] = keyf(first, m);
    if (!groups.has(k)) groups.set(k, { label: l, color: c, members: [] });
    groups.get(k).members.push(m);
  }
  const sorted = [...groups.entries()].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  if (sorted.length === 1 && level !== null) {
    const [, g] = sorted[0];
    const l = g.label;
    const merged =
      l === label || l.startsWith('(') || l.endsWith(`/${label}`) || (!label && !l)
        ? label
        : `${label} / ${l}`;
    return buildGroupTree(g.members, rest, keyf, merged, color || g.color, level);
  }
  const children = sorted.map(([, g]) => buildGroupTree(g.members, rest, keyf, g.label, g.color, first));
  return { label, color, children, agents: [], r: 0, x: 0, y: 0, level };
}

export function findSpot(placed, r, gap) {
  let d = placed[0][2] + r + gap;
  while (true) {
    const steps = Math.max(12, Math.ceil((Math.PI * 2 * d) / (r * 0.6 + 8.0)));
    const off = d * 0.37;
    for (let s = 0; s < steps; s++) {
      const th = off + (s / steps) * Math.PI * 2;
      const x = d * Math.cos(th);
      const y = d * Math.sin(th);
      if (placed.every(([px, py, pr]) => Math.hypot(x - px, y - py) >= pr + r + gap)) {
        return [x, y];
      }
    }
    d += Math.max(6.0, r * 0.25);
  }
}

export function packGroupTree(n, agentR) {
  if (!n.children.length) {
    const k = n.agents.length;
    let biggest = 8.0;
    for (const a of n.agents) biggest = Math.max(biggest, agentR(a));
    n.r = k === 1 ? biggest + 12.0 : Math.max(biggest + 26.0, 20.0 + 17.0 * Math.sqrt(k));
    return;
  }
  for (const c of n.children) packGroupTree(c, agentR);
  const gap = 14.0;
  const order = n.children.map((_, i) => i).sort((a, b) => n.children[b].r - n.children[a].r);
  const placed = [];
  for (const i of order) {
    const r = n.children[i].r;
    const [x, y] = placed.length === 0 ? [0.0, 0.0] : findSpot(placed, r, gap);
    n.children[i].x = x;
    n.children[i].y = y;
    placed.push([x, y, r]);
  }
  let minx = Infinity, maxx = -Infinity, miny = Infinity, maxy = -Infinity;
  for (const [x, y, r] of placed) {
    minx = Math.min(minx, x - r);
    maxx = Math.max(maxx, x + r);
    miny = Math.min(miny, y - r);
    maxy = Math.max(maxy, y + r);
  }
  const cx = (minx + maxx) / 2.0;
  const cy = (miny + maxy) / 2.0;
  let rad = 0.0;
  for (const c of n.children) {
    c.x -= cx;
    c.y -= cy;
    rad = Math.max(rad, Math.hypot(c.x, c.y) + c.r);
  }
  n.r = rad + 22.0;
}
