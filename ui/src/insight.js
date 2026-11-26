import { ago, big, dur, parseHex, projectOf, shortPath, tokensFresh, truncate } from './util.js';

export const Health = {
  Looping: 0,
  Stuck: 1,
  Waiting: 2,
  Working: 3,
  Idle: 4,
  Ended: 5,
  Ghost: 6,
};

export const HEALTH_META = [
  { id: 'looping', label: 'looping', color: parseHex('#ff4fd8') },
  { id: 'stuck', label: 'stuck', color: parseHex('#ff5a36') },
  { id: 'waiting', label: 'waiting', color: parseHex('#ffd166') },
  { id: 'working', label: 'working', color: parseHex('#46f5a0') },
  { id: 'idle', label: 'idle', color: parseHex('#6f86c4') },
  { id: 'ended', label: 'ended', color: parseHex('#4f5a7a') },
  { id: 'ghost', label: 'messages only', color: parseHex('#4f5a7a') },
];

export function healthFromId(s) {
  if (s === 'active' || s === 'busy') return Health.Working;
  const idx = HEALTH_META.findIndex((h) => h.id === s);
  return idx >= 0 ? idx : null;
}

export const HIST_MS = 30 * 60000;
export const HOT_USD_H = 30.0;

export function claudePrice(model) {
  const m = String(model || '').toLowerCase();
  if (!(m.includes('claude') || m.includes('opus') || m.includes('sonnet') || m.includes('haiku') || m.includes('fable') || m.includes('mythos'))) {
    return null;
  }
  if (m.includes('fable-5-2') || m.includes('mythos-5-2')) return [8.0, 40.0];
  if (m.includes('fable') || m.includes('mythos')) return [10.0, 50.0];
  if (m.includes('opus-5-5')) return [4.0, 20.0];
  if (m.includes('opus-4-1') || m.includes('opus-4-0') || m.includes('opus-4-2025') || m.includes('claude-3-opus')) return [15.0, 75.0];
  if (m.includes('opus')) return [5.0, 25.0];
  if (m.includes('sonnet-5')) return [2.0, 10.0];
  if (m.includes('sonnet')) return [3.0, 15.0];
  if (m.includes('haiku-5-5')) return [0.1, 0.5];
  if (m.includes('haiku-4-5')) return [1.0, 5.0];
  if (m.includes('haiku')) return [0.8, 4.0];
  return null;
}

export function costParts(a) {
  const p = claudePrice(a.model);
  if (!p) return null;
  const [pi, po] = p;
  const t = a.tokens || {};
  const input = ((t.input || 0) * pi) / 1e6;
  const output = ((t.output || 0) * po) / 1e6;
  const cacheWrite = ((t.cache_write || 0) * pi * 1.25) / 1e6;
  const cacheRead = ((t.cache_read || 0) * pi * 0.1) / 1e6;
  return { input, output, cacheWrite, cacheRead, total: input + output + cacheWrite + cacheRead };
}

export function costOf(a) {
  if (typeof a.cost_usd === 'number' && a.cost_usd > 0) return [a.cost_usd, false];
  const p = costParts(a);
  if (p) return [p.total, true];
  return [null, false];
}

export function editsOf(a) {
  return (a.files || []).reduce((acc, f) => acc + (f.count || 0), 0);
}

export function recordSample(hist, a) {
  const t = Date.now();
  const s = {
    t,
    tokens: tokensFresh(a.tokens),
    cacheWrite: (a.tokens && a.tokens.cache_write) || 0,
    cacheRead: (a.tokens && a.tokens.cache_read) || 0,
    cost: costOf(a)[0] || 0,
    edits: editsOf(a),
  };
  let q = hist.get(a.addr);
  if (!q) {
    q = [];
    hist.set(a.addr, q);
  }
  const last = q[q.length - 1];
  if (
    last &&
    last.tokens === s.tokens &&
    last.cacheWrite === s.cacheWrite &&
    last.cacheRead === s.cacheRead &&
    last.edits === s.edits &&
    t - last.t < 20000
  ) {
    return;
  }
  q.push(s);
  while (q.length > 2 && t - q[0].t > HIST_MS) q.shift();
}

function calcRates(q, lastActive) {
  if (!q || !q.length) return { tpm: 0, cwTpm: 0, crTpm: 0, usdH: 0, edits: 0 };
  const now = Date.now();
  const first = q.find((s) => now - s.t <= 10 * 60000);
  const last = q[q.length - 1];
  if (!first || !last) return { tpm: 0, cwTpm: 0, crTpm: 0, usdH: 0, edits: 0 };
  const edits = Math.max(0, last.edits - first.edits);
  const span = Math.max(1, Math.max(now, last.t) - first.t);
  if (span < 30000 || now - lastActive > 10 * 60000) {
    return { tpm: 0, cwTpm: 0, crTpm: 0, usdH: 0, edits };
  }
  const perMin = (a, b) => Math.max(0, b - a) / (span / 60000);
  return {
    tpm: perMin(first.tokens, last.tokens),
    cwTpm: perMin(first.cacheWrite, last.cacheWrite),
    crTpm: perMin(first.cacheRead, last.cacheRead),
    usdH: Math.max(0, last.cost - first.cost) / (span / 3600000),
    edits,
  };
}

function series(q, bins, getter) {
  const out = new Array(bins).fill(0);
  if (!q || q.length < 2) return out;
  const now = Date.now();
  for (let i = 1; i < q.length; i++) {
    const a = q[i - 1], b = q[i];
    const idx = Math.floor((now - b.t) / 60000);
    if (idx >= 0 && idx < bins) out[bins - 1 - idx] += Math.max(0, getter(b) - getter(a));
  }
  return out;
}

export function burnSeries(q, bins) {
  return series(q, bins, (s) => s.tokens);
}

export function cacheReadSeries(q, bins) {
  return series(q, bins, (s) => s.cacheRead);
}

export function computeInsights(app) {
  const now = Date.now();
  const pairs = new Map();
  const msgs = new Map();
  for (const m of app.messages) {
    if (now - m.sent_ms >= app.windowMs || m.source === 'swarm') continue;
    for (const x of [m.from_agent, m.to_agent]) {
      if (x) msgs.set(x, (msgs.get(x) || 0) + 1);
    }
    if (now - m.sent_ms > 15 * 60000 || m.source === 'broadcast') continue;
    if (!m.from_agent || !m.to_agent || m.from_agent === m.to_agent) continue;
    const fwd = m.from_agent < m.to_agent;
    const key = fwd ? `${m.from_agent}\0${m.to_agent}` : `${m.to_agent}\0${m.from_agent}`;
    const e = pairs.get(key) || [0, 0];
    if (fwd) e[0]++; else e[1]++;
    pairs.set(key, e);
  }
  const pingpong = new Map();
  for (const [key, [ab, ba]] of pairs.entries()) {
    if (ab + ba >= 8 && Math.min(ab, ba) >= 3) {
      const [a, b] = key.split('\0');
      const n = ab + ba;
      if (!pingpong.has(a)) pingpong.set(a, []);
      if (!pingpong.has(b)) pingpong.set(b, []);
      pingpong.get(a).push(`ping-pong with ${app.name(b)}: ${n} messages in 15m`);
      pingpong.get(b).push(`ping-pong with ${app.name(a)}: ${n} messages in 15m`);
    }
  }
  const out = new Map();
  for (const [addr, a] of app.agents.entries()) {
    const idle = now - Math.max(0, a.last_active_ms || 0);
    const [cost, est] = costOf(a);
    const r = calcRates(app.tokHist.get(addr), a.last_active_ms || 0);
    const reasons = [];
    const loopHit = pingpong.get(addr) || [];
    const hotFile = (a.files || [])
      .filter((f) => f.count >= 40 && now - f.at_ms < 20 * 60000)
      .sort((x, y) => y.count - x.count)[0];
    if (hotFile) loopHit.push(`edited ${shortPath(hotFile.path, 2)} ×${hotFile.count} (last ${ago(hotFile.at_ms)})`);
    const recentFiles = (a.files || []).filter((f) => now - f.at_ms < 10 * 60000).length;
    if (r.edits >= 15 && recentFiles <= 2) loopHit.push(`${r.edits} edits to the same file or two in 10m`);

    let health;
    if (a.ghost) health = Health.Ghost;
    else if (loopHit.length && idle < 20 * 60000) {
      reasons.push(...loopHit);
      health = Health.Looping;
    } else if (a.state === 'busy' && idle > 15 * 60000 && idle < 6 * 3600000 && a.alive !== false) {
      reasons.push(`marked busy but silent for ${dur(idle)}`);
      health = Health.Stuck;
    } else if (idle < 2 * 60000) health = Health.Working;
    else if (a.alive === true) {
      reasons.push(`process alive, quiet for ${dur(idle)}`);
      health = Health.Waiting;
    } else if (a.alive === false) health = Health.Ended;
    else health = Health.Idle;

    out.set(addr, {
      health,
      reasons,
      cost,
      costEstimated: est,
      burnTpm: r.tpm,
      burnCwTpm: r.cwTpm,
      burnCrTpm: r.crTpm,
      burnUsdH: r.usdH,
      idleMs: idle,
      msgs: msgs.get(addr) || 0,
    });
  }
  return out;
}

export function computeAlerts(app) {
  const now = Date.now();
  const v = [];
  for (const [addr, i] of app.insights.entries()) {
    if (!app.drawnAs.has(addr)) continue;
    const a = app.agents.get(addr);
    if (!a) continue;
    if (i.health === Health.Looping) v.push({ sev: 0, kind: 'looping', addr, text: i.reasons.join('; '), at_ms: a.last_active_ms });
    else if (i.health === Health.Stuck) v.push({ sev: 0, kind: 'stuck', addr, text: i.reasons.join('; '), at_ms: a.last_active_ms });
    else if (i.health === Health.Waiting && i.idleMs > 10 * 60000 && i.idleMs < 3 * 3600000) {
      v.push({
        sev: 1,
        kind: 'waiting',
        addr,
        text: `quiet for ${dur(i.idleMs)}; may be waiting for you${a.focus ? ` · last: ${truncate(a.focus, 80)}` : ''}`,
        at_ms: a.last_active_ms,
      });
    }
    if (i.burnUsdH >= HOT_USD_H) {
      v.push({ sev: 1, kind: 'spend', addr, text: `spending about $${i.burnUsdH.toFixed(0)}/h (${big(i.burnTpm)} tokens/min)`, at_ms: now });
    }
  }
  for (let idx = app.messages.length - 1; idx >= 0; idx--) {
    const m = app.messages[idx];
    if (now - m.sent_ms >= 3600000) break;
    if (app.isCross(m)) {
      v.push({
        sev: 0,
        kind: 'cross-pool',
        addr: m.from_agent || null,
        text: `${m.from_agent ? app.name(m.from_agent) : m.from} → ${m.to_agent ? app.name(m.to_agent) : m.to} crossed an isolated pool: ${truncate(m.subject || m.body, 60)}`,
        at_ms: m.sent_ms,
      });
    }
  }
  for (const h of app.hosts) {
    if (h.state === 'error') v.push({ sev: 0, kind: 'computer', addr: null, text: `${h.name} unreachable: ${truncate(h.detail, 90)}`, at_ms: now });
  }
  v.sort((a, b) => a.sev - b.sev || b.at_ms - a.at_ms);
  return v;
}

const FILTER_KEYS = new Set(['kind', 'type', 'host', 'on', 'status', 'is', 'project', 'pool', 'model', 'has']);

export function parseQuery(s) {
  const words = [];
  const filters = [];
  for (const tok of String(s || '').trim().split(/\s+/)) {
    if (!tok) continue;
    const t = tok.toLowerCase();
    const ci = t.indexOf(':');
    if (ci > 0) {
      const k = t.slice(0, ci);
      const val = t.slice(ci + 1);
      if (val && FILTER_KEYS.has(k)) {
        filters.push([k, val]);
        continue;
      }
    }
    words.push(t);
  }
  return { words, filters, isEmpty: !words.length && !filters.length };
}

export function matchesQuery(q, a, ins, name) {
  for (const [k, v] of q.filters) {
    let ok = true;
    if (k === 'kind' || k === 'type') ok = (a.kind || 'other').startsWith(v) || (v === 'openai' && a.kind === 'codex');
    else if (k === 'host' || k === 'on') ok = String(a.host || '').toLowerCase().includes(v);
    else if (k === 'status' || k === 'is') {
      ok = Boolean(ins) && (ins.health === healthFromId(v) || (v === 'alert' && (ins.health === Health.Stuck || ins.health === Health.Looping)) || (v === 'hot' && ins.burnUsdH >= HOT_USD_H));
    } else if (k === 'project') ok = projectOf(a.cwd || '').toLowerCase().includes(v);
    else if (k === 'pool') ok = String(a.pool || '').toLowerCase().includes(v);
    else if (k === 'model') ok = String(a.model || '').toLowerCase().includes(v);
    else if (k === 'has') {
      if (v === 'git') ok = Boolean((a.git || []).length);
      else if (v === 'files') ok = Boolean((a.files || []).length);
      else if (v === 'subagents' || v === 'subs') ok = Boolean((a.subagents || []).length);
      else if (v === 'msgs' || v === 'messages') ok = Boolean(ins && ins.msgs > 0);
      else if (v === 'cost') ok = Boolean(ins && ins.cost !== null);
    }
    if (!ok) return false;
  }
  if (!q.words.length) return true;
  const hay = [name, a.title || '', a.focus || '', a.cwd || '', a.addr || '', a.model || ''].join(' ').toLowerCase();
  return q.words.every((w) => hay.includes(w));
}
