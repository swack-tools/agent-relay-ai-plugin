import { ALL_LEVELS, levelTitle } from './group.js';
import { burnSeries, cacheReadSeries, costParts, HEALTH_META } from './insight.js';
import { UNPOOLED } from './state.js';
import {
  ago, big, bytes, clock, dayClock, displayName, dur, el, esc, heat, hostColorHex,
  inputVal, kindColor, projectOf, RELAY_TARGET_KINDS, rgba, setHtml, setInputVal,
  shortPath, show, sq, tokensFresh, tokensTotal, truncate, usd,
} from './util.js';

export function toast(text, ok = false) {
  const t = el('toast');
  if (!t) return;
  t.classList.toggle('ok', ok);
  t.textContent = text;
  show('toast', true);
  window.setTimeout(() => show('toast', false), ok ? 2500 : 5000);
}

function kindBadge(k) {
  const c = kindColor(k);
  return `<span class="badge" style="border-color:${rgba(c, 1)};color:${rgba(c, 1)}">${esc(k || 'agent')}</span>`;
}

function healthPill(h) {
  const m = HEALTH_META[h] || HEALTH_META[4];
  return `<span class="hp" style="color:${rgba(m.color, 1)};border-color:${rgba(m.color, 1)}">${m.label}</span>`;
}

export function renderHud(app) {
  let hs = '';
  for (const h of app.hosts) {
    const cls = h.state === 'live' ? 'ok' : h.state === 'error' ? 'bad' : 'warn';
    hs += `<span class="chip ${cls}" title="${esc(`${h.name} via ${h.via}: ${h.detail}`)}"><i style="background:${hostColorHex(h.name)}"></i>${esc(h.name)} <b>${esc(h.state)}</b></span>`;
  }
  if (!app.connected) hs += '<span class="chip bad">MCP disconnected</span>';
  setHtml('hosts', hs);

  const now = Date.now();
  const inWin = app.messages.filter((m) => now - m.sent_ms < app.windowMs);
  const relay = inWin.filter((m) => m.source !== 'swarm').length;
  const swarm = inWin.length - relay;
  const cross = inWin.filter((m) => app.isCross(m)).length;
  const shown = Math.max(0, app.nodes.size - 1);
  let folded = 0;
  for (const n of app.nodes.values()) folded += n.children.length;
  const counts = new Array(7).fill(0);
  let cost = 0, tpm = 0, usdH = 0;
  for (const k of app.drawnAs.keys()) {
    const i = app.insight(k);
    if (!i) continue;
    counts[i.health]++;
    cost += i.cost || 0;
    tpm += i.burnTpm;
    usdH += i.burnUsdH;
  }
  const pill = (h) => {
    const n = counts[h];
    if (!n) return '';
    const m = HEALTH_META[h];
    return `<span class="hs" data-status="${m.id}" title="Show only ${m.label} agents"><i style="background:${rgba(m.color, 1)}"></i><b>${n}</b> ${m.label}</span>`;
  };
  setHtml(
    'stats',
    `<span><b>${shown}</b> agents${folded > 0 ? ` <em>(+${folded} in swarms)</em>` : ''}</span>` +
      pill(3) + pill(2) + pill(1) + pill(0) +
      `<span title="Spend of agents on map"><b>${usd(cost)}</b> spent</span>` +
      `<span title="Burn rate since opened"><b>${big(tpm)}</b> tok/min${usdH > 0.5 ? ` <em>≈${usd(usdH)}/h</em>` : ''}</span>` +
      `<span><b>${relay}</b> relay msgs</span>` +
      (swarm > 0 && app.showSwarm ? `<span><b>${swarm}</b> swarm</span>` : '') +
      (cross > 0 ? `<span class="alert"><b>${cross}</b> cross-pool</span>` : ''),
  );

  const crit = app.alerts.filter((a) => a.sev === 0).length;
  const btnAttn = el('btn-attn');
  if (btnAttn) {
    btnAttn.innerHTML = `⚠ <b>${app.alerts.length}</b>`;
    btnAttn.classList.toggle('hot', crit > 0);
  }
  setHtml('search-count', app.query.isEmpty ? '' : `${app.orderedMatches().length} match`);

  const top = el('top');
  if (top) {
    document.documentElement.style.setProperty('--top', `${Math.round(top.getBoundingClientRect().bottom)}px`);
  }

  renderWatch(app);
  renderTimeline(app);

  if (!app.feedPaused) {
    let f = '';
    for (const m of app.feed) {
      const from = m.from_agent ? app.name(m.from_agent) : m.from;
      const to = m.to_agent ? app.name(m.to_agent) : m.to;
      const what = m.subject || truncate(String(m.body || '').replace(/\n/g, ' '), 70);
      const isCross = app.isCross(m);
      const pool = m.from_agent && app.agents.get(m.from_agent)?.pool;
      const col = isCross ? parseHex('#ff3b5c') : app.poolColor(pool);
      const fresh = now - m.sent_ms < 4000 || Boolean(app.replay);
      f += `<li data-action="channel" data-a="${esc(m.from_agent || '')}" data-b="${esc(m.to_agent || '')}" class="${isCross ? 'cross' : ''}${fresh && app.feed[0]?.id === m.id ? ' fresh' : ''}"><span class="t">${clock(m.sent_ms)}</span><span class="dot" style="background:${rgba(col, 1)}"></span><b>${esc(truncate(from, 22))}</b> → <b>${esc(truncate(to, 22))}</b> <span class="sz">${bytes(m.bytes)}</span><div class="what">${esc(what)}</div></li>`;
    }
    setHtml('feed-list', f || '<li class="empty">No relay traffic in this window yet.</li>');
  }

  let leg = '';
  for (const p of app.pools.filter((x) => !x.auto)) {
    leg += `<div><i style="background:${esc(p.color)}"></i>${esc(p.name)}${p.isolated ? ' <em>isolated</em>' : ''}</div>`;
  }
  setHtml('legend-pools', leg);
}

export function renderWatch(app) {
  const pins = [...app.pins].filter((p) => app.agents.has(p)).sort((a, b) => (app.agents.get(b)?.last_active_ms || 0) - (app.agents.get(a)?.last_active_ms || 0));
  if (!pins.length) {
    show('watch', false);
    return;
  }
  let h = '<h3>★ WATCHING</h3><ul>';
  for (const p of pins) {
    const a = app.agents.get(p);
    const i = app.insight(p);
    if (!a || !i) continue;
    const m = HEALTH_META[i.health];
    h += `<li data-action="focus" data-addr="${esc(p)}"><span class="dot" style="background:${rgba(m.color, 1)}"></span><b>${esc(truncate(app.name(p), 28))}</b> <span class="muted">${m.label} · ${ago(a.last_active_ms)}${i.burnTpm > 0 ? ` · ${big(i.burnTpm)}/min` : ''}</span><div class="what">${esc(truncate(a.focus || a.title || '', 70))}</div></li>`;
  }
  h += '</ul>';
  setHtml('watch', h);
  show('watch', true);
}

export function renderTimeline(app) {
  const BINS = 72;
  const now = Date.now();
  const from = now - app.windowMs;
  const step = Math.max(1, app.windowMs / BINS);
  const sel =
    app.selection.type === 'agent' ? new Set([app.selection.addr]) :
    app.selection.type === 'channel' ? new Set([app.selection.a, app.selection.b]) : null;
  const bins = Array.from({ length: BINS }, () => [0, 0, 0]);
  for (const m of app.messages) {
    if (m.sent_ms < from) continue;
    if (sel) {
      const f = m.from_agent && (sel.has(m.from_agent) || sel.has(app.drawnAs.get(m.from_agent)));
      const t = m.to_agent && (sel.has(m.to_agent) || sel.has(app.drawnAs.get(m.to_agent)));
      if (!f && !t) continue;
    }
    const i = Math.min(BINS - 1, Math.max(0, Math.floor((m.sent_ms - from) / step)));
    if (m.source === 'swarm') {
      if (app.showSwarm) bins[i][2]++;
    } else if (app.isCross(m)) bins[i][1]++;
    else bins[i][0]++;
  }
  const peak = Math.max(1, ...bins.map((b) => b[0] + b[1] + b[2]));
  const total = bins.reduce((acc, b) => acc + b[0] + b[1], 0);
  const replayAt = app.replay ? Math.round(app.replay.fromMs + (app.replay.toMs - app.replay.fromMs) * Math.max(0, Math.min(1, (now - app.replay.startReal) / app.replay.spanReal))) : null;
  const titleSub =
    app.selection.type === 'agent' ? ` · ${esc(truncate(app.name(app.selection.addr), 24))}` :
    app.selection.type === 'channel' ? ` · ${esc(truncate(app.name(app.selection.a), 14))} ⇄ ${esc(truncate(app.name(app.selection.b), 14))}` : '';
  let h = `<div class="tl-hd"><span>TRAFFIC${titleSub}</span><span class="muted">${total} relay msgs · click a bar to replay from there</span></div><div class="bars">`;
  const pct = (v) => (v === 0 ? 0 : Math.max(6, (v / peak) * 100));
  for (let i = 0; i < BINS; i++) {
    const [n, x, s] = bins[i];
    const t0 = Math.round(from + i * step);
    const past = replayAt !== null && replayAt >= t0;
    h += `<div class="bar${past ? ' past' : ''}" data-action="replay-from" data-ms="${t0}" title="${dayClock(t0)} · ${n + x} relay${x ? `, ${x} cross-pool` : ''}${s ? `, ${s} swarm` : ''}"><i class="s" style="height:${pct(s).toFixed(0)}%"></i><i class="x" style="height:${pct(x).toFixed(0)}%"></i><i class="n" style="height:${pct(n).toFixed(0)}%"></i></div>`;
  }
  h += `</div><div class="tl-ft muted"><span>${dayClock(from)}</span><span>now</span></div>`;
  setHtml('timeline', h);
}

export function renderDrawer(app) {
  const tab = app.drawer;
  if (!tab) {
    show('drawer', false);
    return;
  }
  show('drawer', true);
  const n0 = app.alerts.filter((a) => a.sev === 0).length;
  const shown = app.orderedMatches().length;
  for (const [id, t, lbl] of [
    ['tab-attention', 'attention', `⚠ Attention <em>${app.alerts.length}</em>`],
    ['tab-agents', 'agents', `Agents <em>${shown}</em>`],
    ['tab-messages', 'messages', 'Messages'],
    ['tab-spend', 'spend', '$ Spend'],
  ]) {
    const b = el(id);
    if (!b) continue;
    b.innerHTML = lbl;
    b.classList.toggle('on', t === tab);
    b.classList.toggle('hot', id === 'tab-attention' && n0 > 0);
  }
  let body = '';
  if (tab === 'attention') body = renderAttention(app);
  else if (tab === 'agents') body = renderAgentsTable(app);
  else if (tab === 'messages') body = renderMessages(app);
  else if (tab === 'spend') body = renderSpend(app);

  const db = el('drawer-body');
  const scroll = db ? db.scrollTop : 0;
  setHtml('drawer-body', body);
  if (db) db.scrollTop = scroll;
}

function renderAttention(app) {
  if (!app.alerts.length) {
    return '<p class="calm">✓ Nothing needs you right now. No stuck, looping or long-waiting agents, no cross-pool traffic in the last hour.</p>';
  }
  let h = '<ul class="alerts">';
  app.alerts.forEach((al, i) => {
    const sev = ['crit', 'warn', 'info'][Math.min(2, al.sev)];
    const who = al.addr ? app.name(al.addr) : '';
    const cur = i === app.alertCursor - 1 && app.alertCursor > 0 ? ' cur' : '';
    const act = al.addr ? `data-action="focus" data-addr="${esc(al.addr)}"` : '';
    h += `<li class="${sev}${cur}" ${act}><span class="k">${esc(al.kind)}</span><b>${esc(who)}</b> <span class="muted">${al.at_ms > 0 ? ago(al.at_ms) : ''}</span><div class="what">${esc(al.text)}</div></li>`;
  });
  h += '</ul><p class="muted small">Press <kbd>n</kbd> to jump through these on the map.</p>';
  return h;
}

function sortKey(app, addr, col) {
  const a = app.agents.get(addr);
  const i = app.insight(addr);
  switch (col) {
    case 'name': return [0, app.name(addr).toLowerCase()];
    case 'kind': return [0, a.kind || ''];
    case 'host': return [0, a.host || ''];
    case 'health': return [i ? i.health : 9, ''];
    case 'tokens': return [tokensFresh(a.tokens), ''];
    case 'cw': return [a.tokens?.cache_write || 0, ''];
    case 'cr': return [a.tokens?.cache_read || 0, ''];
    case 'burn': return [i ? Math.round(i.burnTpm) : 0, ''];
    case 'cost': return [i && i.cost !== null ? Math.round(i.cost * 1000) : -1, ''];
    case 'files': return [(a.files || []).length, ''];
    case 'msgs': return [i ? i.msgs : 0, ''];
    case 'subs': return [runningSubs(app, addr), ''];
    default: return [a.last_active_ms || 0, ''];
  }
}

function runningSubs(app, addr) {
  const now = Date.now();
  const a = app.agents.get(addr);
  const subs = (a?.subagents || []).filter((s) => s.status !== 'done' && now - (s.last_ms || 0) < 180000).length;
  const kids = (app.nodes.get(addr)?.children || []).filter((c) => now - (app.agents.get(c)?.last_active_ms || 0) < 300000).length;
  return subs + kids;
}

function miniSpark(app, addr) {
  const v = burnSeries(app.tokHist.get(addr), 15);
  const peak = Math.max(0, ...v);
  if (peak <= 0) return '';
  let s = '<svg class="spark" viewBox="0 0 60 14" preserveAspectRatio="none">';
  v.forEach((x, i) => {
    if (x > 0) {
      const h = Math.max(1, (x / peak) * 13);
      s += `<rect x="${i * 4}" y="${(14 - h).toFixed(1)}" width="3" height="${h.toFixed(1)}" fill="${rgba(heat(x, 2000, 2000000), 0.9)}"/>`;
    }
  });
  return s + '</svg>';
}

function renderAgentsTable(app) {
  const rows = app.orderedMatches();
  const [col, desc] = app.sort;
  rows.sort((x, y) => {
    const px = app.pins.has(x) ? 0 : 1;
    const py = app.pins.has(y) ? 0 : 1;
    if (px !== py) return px - py;
    const [nx, sx] = sortKey(app, x, col);
    const [ny, sy] = sortKey(app, y, col);
    const cmp = nx !== ny ? nx - ny : sx < sy ? -1 : sx > sy ? 1 : 0;
    return desc ? -cmp : cmp;
  });
  const head = (id, label) => {
    const arrow = col === id ? (desc ? ' ▾' : ' ▴') : '';
    return `<th data-sort="${id}" class="${col === id ? 'on' : ''}">${label}${arrow}</th>`;
  };
  let totalCost = 0, totalFresh = 0, totalCw = 0, totalCr = 0, burn = 0, burnCache = 0;
  let h = `<table class="fleet"><thead><tr><th></th>${head('name', 'Agent')}<th>${head('kind', 'Type').replace('<th ', '<span ').replace('</th>', '</span>')} · ${head('host', 'Computer').replace('<th ', '<span ').replace('</th>', '</span>')}</th>${head('health', 'Status')}${head('active', 'Active')}${head('tokens', 'Fresh')}${head('cw', 'Cache W')}${head('cr', 'Cache R')}${head('burn', 'Tok/min')}<th>15 min</th>${head('cost', 'Cost')}${head('files', 'Files')}${head('msgs', 'Msgs')}${head('subs', 'Subs')}</tr></thead><tbody>`;
  for (const addr of rows.slice(0, 400)) {
    const a = app.agents.get(addr);
    const i = app.insight(addr);
    if (!a || !i) continue;
    totalFresh += tokensFresh(a.tokens);
    totalCw += a.tokens?.cache_write || 0;
    totalCr += a.tokens?.cache_read || 0;
    totalCost += i.cost || 0;
    burn += i.burnTpm;
    burnCache += i.burnCwTpm + i.burnCrTpm;
    const sel = app.selection.type === 'agent' && app.selection.addr === addr;
    const cacheBurn = i.burnCwTpm + i.burnCrTpm;
    const burnCell = i.burnTpm <= 0 && cacheBurn <= 0 ? '·' : cacheBurn <= 0 ? big(i.burnTpm) : `${big(i.burnTpm)}<div class="muted">+${big(cacheBurn)} cache</div>`;
    const subs = runningSubs(app, addr);
    h += `<tr data-action="focus" data-addr="${esc(addr)}" class="${sel ? 'sel' : ''}"><td><span class="pin${app.pins.has(addr) ? ' on' : ''}" data-action="pin" data-addr="${esc(addr)}">${app.pins.has(addr) ? '★' : '☆'}</span></td><td class="nm" title="${esc(`${a.title || ''}\n${a.cwd || ''}`)}"><b>${esc(truncate(app.name(addr), 34))}</b><div class="muted">${esc(truncate(a.focus || a.title || '', 60))}</div></td><td class="wh"><span style="color:${rgba(kindColor(a.kind), 1)}">${esc(a.kind || 'agent')}</span><div style="color:${hostColorHex(a.host)}">${esc(a.host)}</div></td><td>${healthPill(i.health)}</td><td>${a.last_active_ms > 0 ? ago(a.last_active_ms) : '—'}</td><td class="n">${big(tokensFresh(a.tokens))}</td><td class="n muted">${a.tokens?.cache_write ? big(a.tokens.cache_write) : '·'}</td><td class="n muted">${a.tokens?.cache_read ? big(a.tokens.cache_read) : '·'}</td><td class="n">${burnCell}</td><td>${miniSpark(app, addr)}</td><td class="n">${i.cost !== null ? `${usd(i.cost)}${i.costEstimated ? '<sup>est</sup>' : ''}` : '—'}</td><td class="n">${(a.files || []).length || '·'}</td><td class="n">${i.msgs || '·'}</td><td class="n">${subs ? `<b class="live">${subs}</b>` : '·'}</td></tr>`;
  }
  h += `</tbody><tfoot><tr><td></td><td colspan="5">${rows.length} agents${app.query.isEmpty ? '' : ` matching <code>${esc(app.search)}</code>`}</td><td class="n">${big(totalFresh)}</td><td class="n muted">${big(totalCw)}</td><td class="n muted">${big(totalCr)}</td><td class="n">${burnCache > 0 ? `${big(burn)}<div class="muted">+${big(burnCache)} cache</div>` : big(burn)}</td><td></td><td class="n">${usd(totalCost)}</td><td colspan="3"></td></tr></tfoot></table>`;
  h += '<p class="muted small">Filter with search: <code>kind:codex</code> <code>host:laptop</code> <code>status:waiting</code> <code>status:alert</code> <code>project:…</code> <code>model:opus</code> <code>has:git</code>. Click a column to sort.</p>';
  return h;
}

function renderMessages(app) {
  const now = Date.now();
  const sel = app.selection.type === 'agent' ? app.selection.addr : null;
  let h = sel ? `<p class="muted small">Showing messages of <b>${esc(app.name(sel))}</b>. Press Esc to see everyone's.</p>` : '';
  h += '<ul class="mlog">';
  let n = 0;
  for (let idx = app.messages.length - 1; idx >= 0; idx--) {
    const m = app.messages[idx];
    if (now - m.sent_ms >= app.windowMs) continue;
    if (m.source === 'swarm' && !app.showSwarm) continue;
    if (sel && m.from_agent !== sel && m.to_agent !== sel) continue;
    const from = m.from_agent ? app.name(m.from_agent) : m.from;
    const to = m.to_agent ? app.name(m.to_agent) : m.to;
    if (app.query.words.length) {
      const hay = `${from} ${to} ${m.subject || ''} ${m.body || ''} ${m.from}`.toLowerCase();
      if (!app.query.words.every((w) => hay.includes(w))) continue;
    }
    if (++n > 300) break;
    const open = app.openMsgs.has(m.id);
    const what = m.subject || truncate(String(m.body || '').replace(/\n/g, ' '), 140);
    h += `<li data-action="toggle-msg" data-a="${esc(m.from_agent || '')}" data-b="${esc(m.to_agent || '')}" data-id="${esc(m.id)}" class="${app.isCross(m) ? 'cross ' : ''}${m.source === 'swarm' ? 'swarm ' : ''}${open ? 'open' : ''}"><div class="hd"><span class="t">${dayClock(m.sent_ms)}</span> <b>${esc(truncate(from, 26))}</b> → <b>${esc(truncate(to, 26))}</b> <span class="src">${esc(m.source)}</span><span class="sz">${bytes(m.bytes)}</span></div><div class="what">${esc(what)}</div>${open ? `<pre>${esc(m.body)}</pre><div class="acts"><a data-action="channel" data-a="${esc(m.from_agent || '')}" data-b="${esc(m.to_agent || '')}">open conversation ⇄</a> <code>${esc(m.from)}</code> → <code>${esc(m.to)}</code></div>` : ''}</li>`;
  }
  if (n === 0) h += '<li class="muted">No messages match.</li>';
  return h + '</ul>';
}

function renderSpend(app) {
  const addrs = [...app.drawnAs.keys()].filter((k) => {
    const a = app.agents.get(k);
    return a && a.kind !== 'operator' && !a.ghost && app.matchesSearch(a);
  });
  let total = 0, totalH = 0, unpricedTok = 0, unpriced = 0;
  for (const k of addrs) {
    const i = app.insight(k);
    if (i && i.cost !== null) total += i.cost;
    else {
      unpriced++;
      unpricedTok += tokensFresh(app.agents.get(k).tokens);
    }
    if (i) totalH += i.burnUsdH;
  }
  let h = `<div class="spend-top"><div><b>${usd(total)}</b><span>spent by ${addrs.length - unpriced} agents</span></div><div><b>${totalH > 0 ? usd(totalH) : '—'}</b><span>per hour right now</span></div><div><b>${big(unpricedTok)}</b><span>fresh tokens in ${unpriced} unpriced agents</span></div></div><div class="spend-grid">`;
  const dims = [
    ['Project', (a) => projectOf(a.cwd || '')],
    ['Computer', (a) => a.host || ''],
    ['Model', (a) => (a.model || '(unknown)').split('[')[0].trim()],
    ['Agent type', (a) => a.kind || 'other'],
  ];
  for (const [title, fn] of dims) {
    const rows = new Map();
    for (const k of addrs) {
      const a = app.agents.get(k);
      const i = app.insight(k);
      const name = fn(a);
      if (!rows.has(name)) rows.set(name, { cost: 0, est: false, tokens: 0, usdH: 0, agents: 0 });
      const r = rows.get(name);
      r.cost += i?.cost || 0;
      r.est = r.est || Boolean(i?.costEstimated);
      r.tokens += tokensFresh(a.tokens);
      r.usdH += i?.burnUsdH || 0;
      r.agents++;
    }
    const v = [...rows.entries()].sort((a, b) => b[1].cost - a[1].cost || b[1].tokens - a[1].tokens).slice(0, 10);
    const peak = v[0] ? Math.max(0.01, v[0][1].cost) : 1;
    h += `<section><h4>${title}</h4><ul class="bars">`;
    for (const [name, r] of v) {
      const filter = title === 'Project' ? `project:${name}` : title === 'Computer' ? `host:${name}` : title === 'Model' ? `model:${name}` : `kind:${name}`;
      h += `<li data-action="filter" data-q="${esc(filter)}"><div class="lbl"><span>${esc(truncate(name, 28))}</span><span class="muted">${r.agents} agent${r.agents === 1 ? '' : 's'} · ${big(r.tokens)} fresh tok${r.usdH > 0.5 ? ` · ${usd(r.usdH)}/h` : ''}</span><b>${r.cost > 0 ? `${usd(r.cost)}${r.est ? '<sup>est</sup>' : ''}` : '—'}</b></div><div class="track"><i style="width:${Math.min(100, (r.cost / peak) * 100).toFixed(1)}%"></i></div></li>`;
    }
    h += '</ul></section>';
  }
  return h + '</div>';
}

export function renderPanel(app, caps) {
  if (app.selection.type === 'none') {
    show('panel', false);
    return;
  }
  show('panel', true);
  if (app.selection.type === 'agent') setHtml('panel-body', agentPanel(app, app.selection.addr, caps));
  else setHtml('panel-body', channelPanel(app, app.selection.a, app.selection.b));
}

function tokenSplitBar(a, ins) {
  const tk = a.tokens || {};
  const total = tokensTotal(tk);
  if (total === 0) return '';
  const parts = ins?.costEstimated ? costParts(a) : null;
  const segs = [
    ['input', tk.input || 0, parts?.input, 'tk-in'],
    ['output', tk.output || 0, parts?.output, 'tk-out'],
    ['cache write', tk.cache_write || 0, parts?.cacheWrite, 'tk-cw'],
    ['cache read', tk.cache_read || 0, parts?.cacheRead, 'tk-cr'],
  ];
  let bar = '<div class="tk-bar">';
  let legend = '<div class="tk-legend muted small">';
  for (const [label, n, costUsd, cls] of segs) {
    if (!n) continue;
    const pct = (n / total) * 100;
    const c = costUsd !== undefined ? ` · ${usd(costUsd)}` : '';
    bar += `<i class="${cls}" style="width:${pct.toFixed(2)}%" title="${label}: ${big(n)} tokens (${pct.toFixed(0)}%)${c}"></i>`;
    legend += `<span><i class="${cls}"></i>${label} ${pct.toFixed(0)}%${c}</span>`;
  }
  return `${bar}</div>${legend}</div>`;
}

function activitySvg(app, addr) {
  const a = app.agents.get(addr);
  if (!a) return '';
  const now = Date.now();
  const from = now - app.windowMs;
  const bins = 60;
  const step = app.windowMs / bins;
  const rows = [
    ['msgs', '#4cb8ff', new Array(bins).fill(0)],
    ['files', '#46f5a0', new Array(bins).fill(0)],
    ['git', '#ff8a4c', new Array(bins).fill(0)],
    ['subs', '#c78bff', new Array(bins).fill(0)],
  ];
  const put = (r, t) => {
    if (t >= from && t <= now) {
      const i = Math.min(bins - 1, Math.max(0, Math.floor((t - from) / step)));
      rows[r][2][i]++;
    }
  };
  for (const m of app.messagesOf(addr)) put(0, m.sent_ms);
  for (const f of a.files || []) put(1, f.at_ms);
  for (const g of a.git || []) put(2, g.at_ms);
  for (const s of a.subagents || []) {
    put(3, s.started_ms);
    if (s.last_ms !== s.started_ms) put(3, s.last_ms);
  }
  const w = 440, rh = 13, cw = w / bins;
  let svg = `<svg class="act" viewBox="0 0 ${w + 40} ${rh * 4 + 14}" preserveAspectRatio="none">`;
  rows.forEach(([lbl, col, v], r) => {
    const y = r * rh;
    svg += `<text x="0" y="${(y + 9.5).toFixed(1)}" class="lb">${lbl}</text><line x1="40" x2="${w + 40}" y1="${(y + 6.5).toFixed(1)}" y2="${(y + 6.5).toFixed(1)}" class="gl"/>`;
    const peak = Math.max(1, ...v);
    v.forEach((n, i) => {
      if (!n) return;
      const k = 0.35 + 0.65 * (n / peak);
      svg += `<rect x="${(40 + i * cw).toFixed(1)}" y="${(y + 1.5).toFixed(1)}" width="${Math.max(1.5, cw - 1).toFixed(1)}" height="${(rh - 3).toFixed(1)}" fill="${col}" opacity="${k.toFixed(2)}"/>`;
    });
  });
  if (a.last_active_ms >= from) {
    const x = 40 + ((a.last_active_ms - from) / app.windowMs) * w;
    svg += `<line x1="${x.toFixed(1)}" x2="${x.toFixed(1)}" y1="0" y2="${(rh * 4).toFixed(1)}" class="la"/>`;
  }
  return `${svg}<text x="40" y="${(rh * 4 + 12).toFixed(1)}" class="lb">${dayClock(from)}</text><text x="${w + 40}" y="${(rh * 4 + 12).toFixed(1)}" class="lb" text-anchor="end">now</text></svg>`;
}

function burnSvg(app, addr) {
  const v = burnSeries(app.tokHist.get(addr), 30);
  const peak = Math.max(0, ...v);
  if (peak <= 0) return '<p class="muted small">No fresh token use observed since opened.</p>';
  const w = 440, h = 34, cw = w / v.length;
  let s = `<svg class="burn" viewBox="0 0 ${w} ${h + 2}" preserveAspectRatio="none">`;
  v.forEach((x, i) => {
    if (x <= 0) return;
    const bh = Math.max(1.5, (x / peak) * h);
    s += `<rect x="${(i * cw).toFixed(1)}" y="${(h - bh).toFixed(1)}" width="${Math.max(1, cw - 1).toFixed(1)}" height="${bh.toFixed(1)}" fill="${rgba(heat(x, 2000, 2000000), 0.9)}"/>`;
  });
  s += '</svg>';
  const cr = cacheReadSeries(app.tokHist.get(addr), 30).reduce((a, b) => a + b, 0);
  const note = cr > 0 ? `<span>fresh tokens · ${big(cr)} cache reads</span>` : '<span>fresh tokens</span>';
  return `${s}<div class="muted small sp"><span>30 min ago</span>${note}<span>peak ${big(peak)}/min</span><span>now</span></div>`;
}

function openButtons(app, a, caps) {
  const cwd = a.cwd && a.cwd.startsWith('/') ? a.cwd : '';
  if (!cwd) return '';
  const local = a.host === app.selfHost;
  let h = '';
  const url = local ? `vscode://file${cwd}` : `vscode://vscode-remote/ssh-remote+${a.host}${cwd}`;
  if (caps?.openLinks) {
    h += `<button data-action="open-url" data-url="${esc(url)}">Open in VS Code</button>`;
  } else {
    h += `<button data-action="copy" data-text="${esc(url)}">Copy VS Code URI</button>`;
  }
  const resume =
    a.kind === 'claude' ? `claude --resume ${a.id}` :
    a.kind === 'codex' ? `codex resume ${a.id}` :
    a.kind === 'opencode' ? `opencode -s ${a.id}` : '';
  if (resume) {
    const inner = `cd ${sq(cwd)} && ${resume}`;
    const cmd = local ? inner : `ssh -t ${a.host} ${sq(inner)}`;
    h += `<button data-action="copy" data-text="${esc(cmd)}">Copy resume</button>`;
  }
  h += `<button data-action="copy" data-text="${esc(cwd)}">Copy folder</button>`;
  return h;
}

function agentPanel(app, addr, caps) {
  const a = app.agents.get(addr);
  if (!a) return '<p>Agent not found.</p>';
  const now = Date.now();
  const pool = a.pool || UNPOOLED;
  const pcol = app.poolColor(a.pool);
  const status =
    a.ghost ? '<span class="st ghost">seen in messages only</span>' :
    a.alive === true ? `<span class="st on">● alive · ${esc(a.state || 'running')}</span>` :
    now - (a.last_active_ms || 0) < 300000 ? '<span class="st on">● active</span>' :
    a.alive === false ? '<span class="st off">○ process ended</span>' :
    '<span class="st idle">◌ idle</span>';
  const ins = app.insight(addr);
  const health = ins ? `${healthPill(ins.health)} ${ins.reasons.map((r) => `<span class="why">${esc(r)}</span>`).join(' ')}` : '';
  const askBtn = caps?.message && !a.ghost ? `<button class="ask-btn" data-action="ask-chat" data-addr="${esc(addr)}">Ask in chat</button>` : '';

  let h = `<header><div class="row">${kindBadge(a.kind)}<span class="pool" style="color:${rgba(pcol, 1)}">${esc(pool)}</span><span class="host" style="color:${hostColorHex(a.host)}">@${esc(a.host)}</span></div><h2>${esc(app.name(addr))}</h2>${status} ${health}<code class="addr">${esc(a.addr)}</code><div class="acts"><button data-action="pin" data-addr="${esc(addr)}">${app.pins.has(addr) ? '★ Watching' : '☆ Watch'}</button><button data-action="copy" data-text="${esc(addr)}">Copy address</button><button data-action="copy" data-text="${esc(`relay send '${addr}' --subject '…' '…'`)}">Copy relay send</button><button data-action="show-msgs">Messages</button><button data-action="open-log" data-addr="${esc(addr)}">Session log</button>${askBtn}${openButtons(app, a, caps)}</div></header>`;

  if (a.parent) h += `<p class="muted">Spawned by <a data-action="focus" data-addr="${esc(a.parent)}">${esc(app.name(a.parent))}</a></p>`;
  h += `<section class="work"><h4>Working on</h4><p class="title">${esc(a.title || '(no task recorded)')}</p>${a.focus ? `<h4>Right now</h4><p class="focus">▸ ${esc(a.focus)}</p>` : ''}${a.last_text ? `<details><summary>Last thing it said</summary><p class="said">${esc(a.last_text)}</p></details>` : ''}</section>`;

  const tk = a.tokens || {};
  const costLabel = ins && ins.cost !== null ? (ins.costEstimated ? 'cost (est.)' : 'cost') : 'tokens in all';
  h += `<section class="grid"><div><b>${big(tk.input)}</b><span>input</span></div><div><b>${big(tk.output)}</b><span>output</span></div><div><b>${ins && ins.cost !== null ? usd(ins.cost) : big(tokensTotal(tk))}</b><span>${costLabel}</span></div><div class="cache"><b>${big(tk.cache_write)}</b><span>cache write</span></div><div class="cache"><b>${big(tk.cache_read)}</b><span>cache read</span></div><div><b>${ago(a.last_active_ms)}</b><span>last active</span></div></section>`;
  h += tokenSplitBar(a, ins);
  if (ins) {
    const sessMin = Math.max(1, ((a.last_active_ms || 0) - (a.started_ms || 0)) / 60000);
    h += `<section class="grid g2"><div><b>${ins.burnTpm > 0 ? big(ins.burnTpm) : '0'}</b><span>fresh tok/min · 10m</span></div><div class="cache"><b>${ins.burnCwTpm + ins.burnCrTpm > 0 ? `${big(ins.burnCwTpm)} + ${big(ins.burnCrTpm)}` : '0'}</b><span>cache tok/min</span></div><div><b>${ins.burnUsdH > 0 ? usd(ins.burnUsdH) : '—'}</b><span>spend/hour · 10m</span></div><div><b>${a.started_ms > 0 ? big(tokensFresh(tk) / sessMin) : '—'}</b><span>avg fresh tok/min</span></div><div><b>${a.started_ms > 0 ? dayClock(a.started_ms) : '—'}</b><span>started</span></div></section>`;
  }
  h += `<details open><summary>Activity <em>${dur(app.windowMs)} window</em></summary>${activitySvg(app, addr)}<h4>Token burn, last 30 min</h4>${burnSvg(app, addr)}</details>`;

  const meta = [a.model ? `model ${esc(a.model)}` : '', a.cwd ? `<span title="${esc(a.cwd)}">${esc(shortPath(a.cwd, 3))}</span>` : ''].filter(Boolean);
  if (meta.length) h += `<p class="meta">${meta.join(' · ')}</p>`;

  const kids = app.nodes.get(addr)?.children || [];
  const nSub = (a.subagents || []).length + kids.length;
  h += `<details open><summary>Sub-agents &amp; threads <em>${nSub}</em></summary><ul class="subs">`;
  for (const s of (a.subagents || []).slice(-40).reverse()) {
    const live = now - (s.last_ms || 0) < 180000 && s.status !== 'done';
    h += `<li><span class="st ${live ? 'on' : 'idle'}">${live ? '●' : '○'}</span> <b>${esc(s.name)}</b> <span class="muted">${esc(s.kind)} · ${ago(s.last_ms)}</span><div class="task">${esc(truncate(s.task, 260))}</div></li>`;
  }
  for (const c of kids.slice(0, 60)) {
    const ca = app.agents.get(c);
    if (!ca) continue;
    const live = now - (ca.last_active_ms || 0) < 300000;
    h += `<li><span class="st ${live ? 'on' : 'idle'}">${live ? '●' : '○'}</span> <a data-action="focus" data-addr="${esc(c)}"><b>${esc(displayName(ca))}</b></a> <span class="muted">swarm · ${ago(ca.last_active_ms)}</span><div class="task">${esc(truncate(ca.focus || ca.title || '', 220))}</div></li>`;
  }
  if (!nSub) h += '<li class="muted">None recorded.</li>';
  h += '</ul></details>';

  const files = [...(a.files || [])].sort((x, y) => y.at_ms - x.at_ms);
  const created = files.filter((f) => f.op === 'created').length;
  h += `<details open><summary>Files <em>${files.length} touched · ${created} created</em></summary><ul class="files">`;
  for (const f of files.slice(0, 60)) {
    const op = f.op === 'created' ? 'new' : f.op === 'written' ? 'write' : f.op === 'deleted' ? 'del' : 'edit';
    h += `<li><span class="op ${op}">${op}</span><span class="path" title="${esc(f.path)}">${esc(shortPath(f.path, 3))}</span><span class="muted">${f.count > 1 ? `×${f.count} · ` : ''}${ago(f.at_ms)}</span></li>`;
  }
  if (!files.length) h += '<li class="muted">No file writes recorded.</li>';
  h += '</ul></details>';

  const sk = Object.entries(a.skills || {}).sort((x, y) => y[1] - x[1]);
  h += `<details open><summary>Skills <em>${sk.length}</em></summary><div class="chips">${sk.map(([k, n]) => `<span class="tag skill">${esc(k)} <b>${n}</b></span>`).join('') || '<span class="muted">None used.</span>'}</div></details>`;

  const relayMsgs = app.messagesOf(addr).filter((m) => m.source !== 'swarm');
  h += `<details open><summary>Relay messages <em>${relayMsgs.length}</em></summary><ul class="msgs">`;
  for (const m of relayMsgs.slice(-30).reverse()) {
    const out = m.from_agent === addr;
    const peer = (out ? m.to_agent : m.from_agent) || '';
    h += `<li data-action="channel" data-a="${esc(addr)}" data-b="${esc(peer)}" class="${app.isCross(m) ? 'cross' : ''}"><span class="dir">${out ? '→' : '←'}</span> <b>${esc(app.name(peer))}</b> <span class="muted">${bytes(m.bytes)} · ${ago(m.sent_ms)}</span><div class="what">${esc(m.subject || truncate(String(m.body || '').replace(/\n/g, ' '), 120))}</div></li>`;
  }
  if (!relayMsgs.length) h += '<li class="muted">No relay messages in history.</li>';
  h += '</ul></details>';

  if (RELAY_TARGET_KINDS.has(a.kind) && !a.ghost) {
    h += `<button class="primary" data-action="message-agent" data-addr="${esc(addr)}">Message this agent…</button>`;
  }
  return h;
}

function channelPanel(app, a, b) {
  const msgs = app.messagesBetween(a, b);
  const total = msgs.reduce((acc, m) => acc + (m.bytes || 0), 0);
  const cross = msgs.some((m) => app.isCross(m));
  let h = `<header><div class="row"><span class="badge">channel</span>${cross ? '<span class="badge alert">crosses isolated pools</span>' : ''}</div><h2>${esc(app.name(a))} ⇄ ${esc(app.name(b))}</h2><p class="muted">${msgs.length} messages · ${bytes(total)}</p></header><section class="pair">`;
  for (const x of [a, b]) {
    const ag = app.agents.get(x);
    if (!ag) continue;
    h += `<div class="side"><div class="row">${kindBadge(ag.kind)} <a data-action="focus" data-addr="${esc(x)}"><b>${esc(displayName(ag))}</b></a> <span class="host" style="color:${hostColorHex(ag.host)}">@${esc(ag.host)}</span></div><p class="title">${esc(truncate(ag.title || '(no task recorded)', 200))}</p>${ag.focus ? `<p class="focus">▸ ${esc(ag.focus)}</p>` : ''}</div>`;
  }
  h += '</section><h4>Conversation</h4><ul class="convo">';
  for (const m of msgs.slice(-80).reverse()) {
    const from = m.from_agent ? app.name(m.from_agent) : m.from;
    const right = (m.from_agent && (app.drawnAs.get(m.from_agent) || m.from_agent)) === b;
    h += `<li class="${right ? 'r' : 'l'} ${m.source === 'swarm' ? 'swarm' : ''}"><div class="hd"><b>${esc(from)}</b> <span class="muted">${dayClock(m.sent_ms)} · ${bytes(m.bytes)} · ${esc(m.source)}</span></div>${m.subject ? `<div class="subj">${esc(m.subject)}</div>` : ''}<details><summary>${esc(truncate(String(m.body || '').replace(/\n/g, ' '), 140))}</summary><pre>${esc(m.body)}</pre></details></li>`;
  }
  return h + '</ul>';
}

export function renderGroupPanel(app) {
  const rows = app.levels.map((l) => [l, true]);
  for (const l of ALL_LEVELS) {
    if (!app.levels.includes(l)) rows.push([l, false]);
  }
  let h = '';
  rows.forEach(([l, on], i) => {
    h += `<li class="${on ? '' : 'off'}"><input type="checkbox" data-action="group-toggle" data-level="${l}" ${on ? 'checked' : ''}><span class="n">${on ? i + 1 : '–'}</span><span class="t">${esc(levelTitle(l))}</span><button data-action="group-up" data-level="${l}" ${on && i > 0 ? '' : 'disabled'}>↑</button><button data-action="group-down" data-level="${l}" ${on && i + 1 < app.levels.length ? '' : 'disabled'}>↓</button></li>`;
  });
  setHtml('group-list', h);
}

export function openBroadcast(app, preset = null) {
  let opts = '<option value="all">Every live agent</option>';
  for (const p of app.pools) {
    opts += `<option value="pool:${esc(p.name)}">${p.auto ? 'Computer · project' : 'Pool'}: ${esc(p.name)}</option>`;
  }
  if (app.selection.type === 'agent') {
    opts += `<option value="agent:${esc(app.selection.addr)}">Selected: ${esc(app.name(app.selection.addr))}</option>`;
  }
  if (preset && !(app.selection.type === 'agent' && app.selection.addr === preset)) {
    opts += `<option value="agent:${esc(preset)}">Agent: ${esc(app.name(preset))}</option>`;
  }
  setHtml('bc-scope', opts);
  if (preset) setInputVal('bc-scope', `agent:${preset}`);
  app.broadcastPlan = null;
  app.broadcastResults = [];
  app.broadcastSending = false;
  const b = el('bc-send');
  if (b) {
    b.removeAttribute('data-armed');
    b.textContent = 'Send';
  }
  renderBroadcast(app);
  show('modal', true);
}

export function renderBroadcast(app) {
  if (!app.broadcastPlan) {
    setHtml('bc-plan', '<p class="muted">Preview shows exactly who will receive it. Messages go through agent-relay (<code>relay send</code>).</p>');
    return;
  }
  const [targets, skipped] = app.broadcastPlan;
  let h = `<p><b>${targets.length}</b> recipient(s)</p><ul class="targets">`;
  for (const t of targets) {
    const res = app.broadcastResults.find((r) => r.addr === t);
    const cls = res ? (res.ok ? 'ok' : 'bad') : '';
    const note = res ? `${res.ok ? '✓' : '✗'} ${res.detail}` : app.broadcastSending ? 'sending…' : '';
    h += `<li class="${cls}"><b>${esc(app.name(t))}</b> <code>${esc(t)}</code><div class="muted">${esc(note)}</div></li>`;
  }
  h += '</ul>';
  if (skipped.length) {
    h += `<details><summary>${skipped.length} skipped</summary><ul class="muted">${skipped.slice(0, 80).map((s) => `<li>${esc(s)}</li>`).join('')}</ul></details>`;
  }
  setHtml('bc-plan', h);
}

export function renderLogHead(name, addr, info, hasRaw = false) {
  setHtml(
    'log-head',
    `<div class="lt"><b>Session log</b> <span>${esc(name)}</span><div class="muted small">${esc(info)}</div></div><div class="lb"><label><input type="checkbox" data-action="log-toggle" data-class="hide-tools"> hide tools</label><label><input type="checkbox" data-action="log-toggle" data-class="hide-thinking"> hide thinking</label>${hasRaw ? `<button data-action="copy-raw-log" data-addr="${esc(addr)}">Copy raw</button>` : ''}<button data-action="log-close" title="Close (Esc)">✕</button></div>`,
  );
}

export function renderLogBody(v) {
  let h = '';
  for (const n of v.notes || []) h += `<p class="muted small">Skipped: ${esc(n)}</p>`;
  if (!v.entries?.length) h += '<p class="muted">The log has no readable turns.</p>';
  for (const e of v.entries || []) {
    const when = e.t > 0 ? dayClock(e.t) : '';
    const text = esc(e.text);
    if (e.role === 'tool' || e.role === 'result' || e.role === 'thinking') {
      const first = String(e.text || '').split('\n')[0].slice(0, 120);
      const lbl = e.role === 'tool' ? `⚙ ${esc(e.name || 'tool')}` : e.role === 'result' ? '↳ result' : '… thinking';
      h += `<details class="le r-${e.role}"><summary><b>${lbl}</b> <code>${esc(first)}</code><span class="muted">${when}</span></summary><pre>${text}</pre></details>`;
    } else {
      const who = e.role === 'user' ? 'User' : e.role === 'assistant' ? 'Agent' : 'System';
      h += `<div class="le r-${esc(e.role)}"><div class="lh"><b>${who}</b>${e.name ? ` <code>${esc(e.name)}</code>` : ''}<span class="muted">${when}</span></div><pre>${text}</pre></div>`;
    }
  }
  setHtml('log-body', h);
  const body = el('log-body');
  if (body) body.scrollTop = body.scrollHeight;
}
