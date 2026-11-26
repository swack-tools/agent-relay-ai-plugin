import { App, applyDocumentTheme, applyHostStyleVariables } from '@modelcontextprotocol/ext-apps';
import { arcPoints, distToArc, drawFrame } from './src/draw.js';
import { ALL_LEVELS } from './src/group.js';
import { AppState } from './src/state.js';
import { checked, dayClock, el, esc, inputVal, setChecked, setHtml, setInputVal, show } from './src/util.js';
import { openBroadcast, renderBroadcast, renderDrawer, renderGroupPanel, renderHud, renderLogBody, renderLogHead, renderPanel, toast } from './src/views.js';

const mcpApp = new App({ name: 'Agent Relay', version: '1.1.0' }, {});
const state = new AppState();

let lastPanel = 0;
let lastHud = 0;
let lastDrawer = 0;
let receivedInitial = false;
let pollingInFlight = false;
let openLogAddr = null;

function canvasEl() {
  return el('map');
}

function resize() {
  const c = canvasEl();
  if (!c) return;
  const w = window.innerWidth || 1200;
  const h = window.innerHeight || 800;
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  c.width = Math.round(w * dpr);
  c.height = Math.round(h * dpr);
  state.width = w;
  state.height = h;
  state.dpr = dpr;
}

function hitNode(x, y) {
  const z = Math.max(0.45, Math.min(1.8, Math.sqrt(state.cam.zoom)));
  let best = null;
  for (const [k, n] of state.nodes.entries()) {
    const [sx, sy] = state.worldToScreen(n.x, n.y);
    const d = Math.hypot(sx - x, sy - y);
    if (d < n.r * z + 6 && (!best || d < best[0])) best = [d, k];
  }
  return best ? best[1] : null;
}

function hitChannel(x, y) {
  let best = null;
  for (const c of state.channels) {
    const pts = arcPoints(state, c.a, c.b);
    if (!pts) continue;
    const d = distToArc(x, y, pts);
    if (d < 7 && (!best || d < best[0])) best = [d, [c.a, c.b]];
  }
  return best ? best[1] : null;
}

function clearSelection() {
  state.selection = { type: 'none' };
  state.panelDirty = true;
  state.fitAll();
}

function openDrawer(tab) {
  const same = state.drawer === tab;
  state.drawer = same ? null : tab;
  document.body.classList.toggle('drawer-open', !same);
  refit();
  renderDrawerNow();
}

function closeDrawer() {
  state.drawer = null;
  document.body.classList.remove('drawer-open');
  refit();
  renderDrawerNow();
}

function refit() {
  if (state.selection.type === 'none') state.fitAll();
  else if (state.selection.type === 'agent') state.focus(state.selection.addr);
  else if (state.selection.type === 'channel') state.focusChannel(state.selection.a, state.selection.b);
}

function renderDrawerNow() {
  lastDrawer = Date.now();
  state.drawerDirty = false;
  renderDrawer(state);
}

function persistPrefs(patch) {
  mcpApp.callServerTool({ name: 'relay_map_prefs', arguments: { patch } }).catch(() => {});
}

function applyPrefs(prefs) {
  if (!prefs || typeof prefs !== 'object') return;
  if (Array.isArray(prefs.pins)) state.pins = new Set(prefs.pins);
  if (Array.isArray(prefs.levels) && prefs.levels.length) state.levels = prefs.levels.filter((l) => ALL_LEVELS.includes(l));
  if (prefs.color_by) {
    state.colorBy = prefs.color_by;
    setInputVal('color-by', prefs.color_by);
  }
  if (prefs.window_hours) {
    state.windowMs = Number(prefs.window_hours) * 3600000;
    setInputVal('window', String(prefs.window_hours));
  }
  if (typeof prefs.show_unpooled === 'boolean') {
    state.showUnpooled = prefs.show_unpooled;
    setChecked('opt-unpooled', prefs.show_unpooled);
  }
  if (typeof prefs.show_swarm === 'boolean') {
    state.showSwarm = prefs.show_swarm;
    setChecked('opt-swarm', prefs.show_swarm);
  }
  if (typeof prefs.expand_swarms === 'boolean') {
    state.expandSwarms = prefs.expand_swarms;
    setChecked('opt-expand', prefs.expand_swarms);
  }
}

function togglePin(addr) {
  if (!state.pins.delete(addr)) state.pins.add(addr);
  state.panelDirty = true;
  state.drawerDirty = true;
  state.hudDirty = true;
  persistPrefs({ pins: [...state.pins] });
  renderDrawerNow();
}

async function copyText(t) {
  try {
    await navigator.clipboard.writeText(t);
    toast(`Copied: ${t.slice(0, 80)}`, true);
  } catch {
    toast('Clipboard unavailable in this host frame.');
  }
}

async function openLog(addr) {
  const name = state.name(addr);
  openLogAddr = addr;
  show('log', true);
  renderLogHead(name, addr, 'Fetching…', false);
  setHtml('log-body', '<p class="muted">Reading transcript from collector…</p>');
  try {
    const res = await mcpApp.callServerTool({ name: 'relay_transcript', arguments: { addr, max_entries: 600 } });
    if (openLogAddr !== addr) return;
    if (res?.isError || !res?.structuredContent) {
      const msg = (res?.content || []).map((c) => c.text || '').join(' ') || 'Unavailable';
      renderLogHead(name, addr, 'Not found', false);
      setHtml('log-body', `<p>Could not read this agent's session log.</p><pre class="err">${esc(msg)}</pre>`);
      return;
    }
    const v = res.structuredContent;
    renderLogHead(name, addr, `${v.source} · ${(v.entries || []).length} turns · ${Math.round((v.bytes || 0) / 1024)} KB`, true);
    renderLogBody(v);
  } catch (err) {
    renderLogHead(name, addr, 'Error', false);
    setHtml('log-body', `<pre class="err">${esc(String(err))}</pre>`);
  }
}

async function runBroadcast(dryRun) {
  const scope = inputVal('bc-scope') || 'all';
  const subject = inputVal('bc-subject');
  const body = inputVal('bc-body');
  const includeStale = checked('bc-stale');
  if (dryRun) {
    state.broadcastPlan = null;
    state.broadcastResults = [];
    setHtml('bc-plan', '<p class="muted">Asking the MCP server who would receive it…</p>');
  } else {
    state.broadcastSending = true;
    renderBroadcast(state);
  }
  try {
    const res = await mcpApp.callServerTool({
      name: 'relay_broadcast',
      arguments: { scope, subject, body, dry_run: dryRun, include_stale: includeStale },
    });
    state.broadcastSending = false;
    if (res?.isError || !res?.structuredContent) {
      const txt = (res?.content || []).map((c) => c.text || '').join(' ') || 'Broadcast failed';
      toast(txt);
      return;
    }
    const sc = res.structuredContent;
    state.broadcastPlan = [sc.targets || [], sc.skipped || []];
    state.broadcastResults = sc.results || [];
    renderBroadcast(state);
    if (!dryRun) {
      toast(`Broadcast sent to ${(sc.results || []).filter((r) => r.ok).length} agent(s).`, true);
      pollNow();
    }
  } catch (err) {
    state.broadcastSending = false;
    toast(`Broadcast error: ${err?.message || err}`);
  }
}

function handleAction(target) {
  const actEl = target.closest('[data-action]');
  if (!actEl) return;
  const act = actEl.getAttribute('data-action');
  if (act === 'focus') {
    const addr = actEl.getAttribute('data-addr');
    if (addr) state.focus(addr);
  } else if (act === 'channel') {
    const x = actEl.getAttribute('data-a') || '';
    const y = actEl.getAttribute('data-b') || '';
    const xa = state.drawnAs.get(x) || x;
    const yb = state.drawnAs.get(y) || y;
    if (!yb || xa === yb) state.focus(x);
    else {
      const [p, q] = xa < yb ? [xa, yb] : [yb, xa];
      state.focusChannel(p, q);
    }
  } else if (act === 'message-agent') {
    openBroadcast(state, actEl.getAttribute('data-addr'));
  } else if (act === 'pin') {
    const addr = actEl.getAttribute('data-addr');
    if (addr) togglePin(addr);
  } else if (act === 'copy') {
    const txt = actEl.getAttribute('data-text');
    if (txt) copyText(txt);
  } else if (act === 'open-url') {
    const url = actEl.getAttribute('data-url');
    if (url) mcpApp.openLink({ url }).catch(() => copyText(url));
  } else if (act === 'ask-chat') {
    const addr = actEl.getAttribute('data-addr');
    if (addr) {
      mcpApp.sendMessage({
        role: 'user',
        content: [{ type: 'text', text: `Check on agent ${addr} (${state.name(addr)}) and summarize its current status.` }],
      }).catch(() => {});
    }
  } else if (act === 'show-msgs') {
    openDrawer('messages');
  } else if (act === 'filter') {
    const q = actEl.getAttribute('data-q') || '';
    setInputVal('search', q);
    state.setSearch(q);
    renderDrawerNow();
  } else if (act === 'open-log') {
    const addr = actEl.getAttribute('data-addr');
    if (addr) openLog(addr);
  } else if (act === 'log-close') {
    openLogAddr = null;
    show('log', false);
  } else if (act === 'log-toggle') {
    const cls = actEl.getAttribute('data-class');
    if (cls) el('log')?.classList.toggle(cls);
  } else if (act === 'copy-raw-log') {
    const addr = actEl.getAttribute('data-addr');
    if (addr) {
      mcpApp.callServerTool({ name: 'relay_transcript', arguments: { addr, raw: true } }).then((res) => {
        if (res?.structuredContent?.raw_tail) copyText(res.structuredContent.raw_tail);
      }).catch(() => {});
    }
  } else if (act === 'toggle-msg') {
    if (target.closest('pre, a')) return;
    const id = actEl.getAttribute('data-id');
    if (id) {
      if (!state.openMsgs.delete(id)) state.openMsgs.add(id);
      renderDrawerNow();
    }
  } else if (act === 'replay-from') {
    const ms = Number(actEl.getAttribute('data-ms'));
    if (Number.isFinite(ms)) {
      const span = Math.max(4000, Math.min(36000, ((Date.now() - ms) / state.windowMs) * 36000));
      state.startReplayFrom(ms, span);
    }
  }
}

function stepAgent(dir) {
  const list = state.orderedMatches();
  if (!list.length) return;
  const cur = state.selection.type === 'agent' ? list.indexOf(state.selection.addr) : -1;
  const next = cur >= 0 ? (cur + dir + list.length) % list.length : dir > 0 ? 0 : list.length - 1;
  state.focus(list[next]);
}

function nextAlert() {
  const targets = state.alerts.map((a) => a.addr).filter(Boolean);
  if (!targets.length) {
    toast('Nothing needs attention right now.', true);
    return;
  }
  const i = state.alertCursor % targets.length;
  state.alertCursor = i + 1;
  state.focus(targets[i]);
  state.drawerDirty = true;
}

function bindEvents() {
  const c = canvasEl();
  c.addEventListener('mousedown', (e) => {
    state.drag = [e.clientX, e.clientY, state.cam.x, state.cam.y, false];
  });
  c.addEventListener('mousemove', (e) => {
    const x = e.clientX, y = e.clientY;
    if (state.drag) {
      const [sx, sy, cx, cy, moved0] = state.drag;
      const moved = moved0 || Math.abs(x - sx) + Math.abs(y - sy) > 4;
      if (moved) {
        state.cam.tx = cx - (x - sx) / state.cam.zoom;
        state.cam.ty = cy - (y - sy) / state.cam.zoom;
        state.cam.x = state.cam.tx;
        state.cam.y = state.cam.ty;
      }
      state.drag = [sx, sy, cx, cy, moved];
      return;
    }
    const h = hitNode(x, y);
    const overChan = !h && Boolean(hitChannel(x, y));
    state.hover = h;
    c.style.cursor = h || overChan ? 'pointer' : 'grab';
    if (!h) {
      show('tooltip', false);
    } else {
      const ag = state.agents.get(h);
      if (ag) {
        const kids = state.nodes.get(h)?.children.length || 0;
        const freshTok = (ag.tokens?.input || 0) + (ag.tokens?.output || 0) + (ag.tokens?.cache_write || 0);
        setHtml(
          'tooltip',
          `<b>${esc(state.name(h))}</b> <span class="muted">${esc(ag.kind || 'agent')} @${esc(ag.host)}</span><br>${esc((ag.focus || ag.title || '').slice(0, 140))}<br><span class="muted">${freshTok.toLocaleString()} tok${kids ? ` · ${kids} swarm` : ''}</span>`,
        );
        const tt = el('tooltip');
        if (tt) {
          tt.style.left = `${Math.min(x + 16, state.width - 330)}px`;
          tt.style.top = `${y + 12}px`;
        }
        show('tooltip', true);
      }
    }
  });
  c.addEventListener('mouseup', (e) => {
    const drag = state.drag;
    state.drag = null;
    if (!drag || drag[4]) return;
    const h = hitNode(e.clientX, e.clientY);
    if (h) state.focus(h);
    else {
      const ch = hitChannel(e.clientX, e.clientY);
      if (ch) state.focusChannel(ch[0], ch[1]);
      else clearSelection();
    }
  });
  c.addEventListener('mouseleave', () => {
    state.drag = null;
    state.hover = null;
    show('tooltip', false);
  });
  c.addEventListener('wheel', (e) => {
    e.preventDefault();
    const [wx, wy] = state.screenToWorld(e.clientX, e.clientY);
    const nz = Math.max(0.12, Math.min(5.0, state.cam.zoom * Math.exp(-e.deltaY * 0.0015)));
    state.cam.zoom = nz;
    state.cam.tzoom = nz;
    state.cam.x = wx - (e.clientX - state.width / 2) / nz;
    state.cam.y = wy - (e.clientY - state.height / 2) / nz;
    state.cam.tx = state.cam.x;
    state.cam.ty = state.cam.y;
  }, { passive: false });
  c.addEventListener('dblclick', clearSelection);
  window.addEventListener('resize', resize);

  window.addEventListener('keydown', (e) => {
    const tag = e.target?.tagName || '';
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') {
      if (e.key === 'Escape') show('modal', false);
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    switch (e.key) {
      case 'Escape':
        if (!el('log')?.classList.contains('hidden')) { openLogAddr = null; show('log', false); return; }
        show('modal', false);
        show('help', false);
        if (state.selection.type !== 'none') clearSelection();
        else closeDrawer();
        break;
      case '/':
        e.preventDefault();
        el('search')?.focus();
        el('search')?.select();
        break;
      case '?': show('help', true); break;
      case 'j': case 'ArrowDown': stepAgent(1); break;
      case 'k': case 'ArrowUp': stepAgent(-1); break;
      case 'n': nextAlert(); break;
      case 'a': openDrawer('attention'); break;
      case 't': openDrawer('agents'); break;
      case 'm': openDrawer('messages'); break;
      case 's': openDrawer('spend'); break;
      case 'c': {
        const order = ['kind', 'health', 'spend'];
        const next = order[(order.indexOf(state.colorBy) + 1) % order.length];
        state.colorBy = next;
        setInputVal('color-by', next);
        persistPrefs({ color_by: next });
        break;
      }
      case 'w':
        if (state.selection.type === 'agent') togglePin(state.selection.addr);
        break;
      case 'y':
        if (state.selection.type === 'agent') copyText(state.selection.addr);
        break;
      case 'b': openBroadcast(state, null); break;
      case 'r': state.startReplay(36000); break;
      case 'f': state.fitAll(); break;
      case 'g': {
        const hidden = el('group-panel')?.classList.contains('hidden');
        if (hidden) renderGroupPanel(state);
        show('group-panel', hidden);
        break;
      }
    }
  });

  el('window')?.addEventListener('change', () => {
    const h = Number(inputVal('window')) || 24;
    state.windowMs = h * 3600000;
    state.layoutDirty = true;
    state.hudDirty = true;
    state.panelDirty = true;
    persistPrefs({ window_hours: h });
  });
  el('opt-unpooled')?.addEventListener('change', () => {
    state.showUnpooled = checked('opt-unpooled');
    state.layoutDirty = true;
    persistPrefs({ show_unpooled: state.showUnpooled });
  });
  el('opt-swarm')?.addEventListener('change', () => {
    state.showSwarm = checked('opt-swarm');
    state.layoutDirty = true;
    state.hudDirty = true;
    persistPrefs({ show_swarm: state.showSwarm });
  });
  el('opt-expand')?.addEventListener('change', () => {
    state.expandSwarms = checked('opt-expand');
    state.layoutDirty = true;
    persistPrefs({ expand_swarms: state.expandSwarms });
  });
  el('search')?.addEventListener('input', () => {
    state.setSearch(inputVal('search'));
    renderDrawerNow();
  });
  el('search')?.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      const best = state.orderedMatches()[0];
      if (best) state.focus(best);
    } else if (e.key === 'Escape') {
      el('search')?.blur();
    }
  });
  el('color-by')?.addEventListener('change', () => {
    state.colorBy = inputVal('color-by') || 'kind';
    persistPrefs({ color_by: state.colorBy });
  });
  el('stats')?.addEventListener('click', (e) => {
    const st = e.target?.closest('[data-status]')?.getAttribute('data-status');
    if (st) {
      const q = `status:${st}`;
      const next = inputVal('search') === q ? '' : q;
      setInputVal('search', next);
      state.setSearch(next);
      renderDrawerNow();
    }
  });

  el('btn-group')?.addEventListener('click', () => {
    const hidden = el('group-panel')?.classList.contains('hidden');
    if (hidden) renderGroupPanel(state);
    show('group-panel', hidden);
  });
  el('group-panel')?.addEventListener('click', (e) => {
    const preset = e.target?.getAttribute('data-preset');
    if (preset) {
      state.levels = preset.split(',').filter((l) => ALL_LEVELS.includes(l));
      state.layout();
      state.fitAll();
      renderGroupPanel(state);
      persistPrefs({ levels: state.levels });
      return;
    }
    const action = e.target?.getAttribute('data-action');
    const lvl = e.target?.getAttribute('data-level');
    if (!action || !lvl) return;
    const lv = [...state.levels];
    const i = lv.indexOf(lvl);
    if (action === 'group-toggle') {
      if (i >= 0) lv.splice(i, 1);
      else lv.push(lvl);
    } else if (action === 'group-up' && i > 0) {
      [lv[i - 1], lv[i]] = [lv[i], lv[i - 1]];
    } else if (action === 'group-down' && i >= 0 && i + 1 < lv.length) {
      [lv[i + 1], lv[i]] = [lv[i], lv[i + 1]];
    }
    state.levels = lv;
    state.layout();
    state.fitAll();
    renderGroupPanel(state);
    persistPrefs({ levels: state.levels });
  });

  el('btn-replay')?.addEventListener('click', () => state.startReplay(36000));
  el('btn-fit')?.addEventListener('click', clearSelection);
  el('btn-broadcast')?.addEventListener('click', () => openBroadcast(state, null));
  el('panel-close')?.addEventListener('click', clearSelection);
  el('btn-attn')?.addEventListener('click', () => openDrawer('attention'));
  el('btn-table')?.addEventListener('click', () => openDrawer('agents'));
  el('btn-help')?.addEventListener('click', () => show('help', true));
  el('help-close')?.addEventListener('click', () => show('help', false));
  el('drawer-close')?.addEventListener('click', closeDrawer);
  el('drawer')?.addEventListener('click', (e) => {
    const tab = e.target?.closest('[data-tab]')?.getAttribute('data-tab');
    if (tab) {
      state.drawer = tab;
      renderDrawerNow();
    }
  });
  el('drawer')?.addEventListener('mouseenter', () => { state.drawerHover = true; });
  el('drawer')?.addEventListener('mouseleave', () => { state.drawerHover = false; });
  el('feed')?.addEventListener('mouseenter', () => { state.feedPaused = true; });
  el('feed')?.addEventListener('mouseleave', () => { state.feedPaused = false; state.hudDirty = true; });

  for (const id of ['panel-body', 'feed-list', 'watch', 'timeline', 'log']) {
    el(id)?.addEventListener('click', (e) => handleAction(e.target));
  }
  el('drawer-body')?.addEventListener('click', (e) => {
    const col = e.target?.closest('[data-sort]')?.getAttribute('data-sort');
    if (col) {
      if (state.sort[0] === col) state.sort[1] = !state.sort[1];
      else state.sort = [col, !['name', 'kind', 'host', 'health'].includes(col)];
      renderDrawerNow();
      return;
    }
    handleAction(e.target);
  });

  el('bc-close')?.addEventListener('click', () => show('modal', false));
  el('bc-scope')?.addEventListener('change', () => {
    state.broadcastPlan = null;
    state.broadcastResults = [];
    renderBroadcast(state);
  });
  el('bc-preview')?.addEventListener('click', () => runBroadcast(true));
  el('bc-send')?.addEventListener('click', () => {
    const body = inputVal('bc-body').trim();
    if (!body) { toast('Write the message first.'); return; }
    const b = el('bc-send');
    if (!state.broadcastPlan) {
      runBroadcast(true);
      if (b) { b.setAttribute('data-armed', '1'); b.textContent = 'Confirm send'; }
      return;
    }
    if (!state.broadcastPlan[0].length) { toast('Nobody to send to in that scope.'); return; }
    if (!b?.getAttribute('data-armed')) {
      b?.setAttribute('data-armed', '1');
      if (b) b.textContent = `Confirm: send to ${state.broadcastPlan[0].length}`;
      return;
    }
    b.removeAttribute('data-armed');
    b.textContent = 'Send';
    runBroadcast(false);
  });
}

function tick() {
  const ctx = canvasEl()?.getContext('2d');
  if (!ctx) return;
  const t = Date.now();
  const relayout = (state.layoutDirty && t - state.lastLayout > 250) || t - state.lastLayout > 5000;
  if (relayout) {
    const first = state.nodes.size <= 1;
    state.layout();
    if (first && state.nodes.size > 1) {
      state.fitAll();
      state.cam.zoom = state.cam.tzoom;
      state.cam.x = state.cam.tx;
      state.cam.y = state.cam.ty;
    }
  }
  state.cam.x += (state.cam.tx - state.cam.x) * 0.12;
  state.cam.y += (state.cam.ty - state.cam.y) * 0.12;
  state.cam.zoom += (state.cam.tzoom - state.cam.zoom) * 0.12;

  const sim = state.tickReplay();
  if (sim !== null) {
    setHtml('replay-clock', `▶ REPLAY · ${dayClock(sim)}`);
    show('replay-clock', true);
  } else {
    show('replay-clock', false);
  }

  if (t - state.lastInsight > 2000) {
    state.refreshInsights();
    state.hudDirty = true;
    if (state.selection.type === 'agent') state.panelDirty = true;
  }

  drawFrame(state, ctx, t);

  if (state.drawer && state.drawerDirty && !state.drawerHover && t - lastDrawer > 1500) {
    renderDrawerNow();
  }
  if ((state.hudDirty && t - lastHud > 700) || t - lastHud > 5000) {
    lastHud = t;
    state.hudDirty = false;
    renderHud(state);
  }
  if (state.panelDirty && t - lastPanel > 1200) {
    lastPanel = t;
    state.panelDirty = false;
    renderPanel(state, mcpApp.getHostCapabilities());
  }
  window.requestAnimationFrame(tick);
}

async function pollNow() {
  if (pollingInFlight) return;
  pollingInFlight = true;
  try {
    const res = await mcpApp.callServerTool({
      name: 'relay_map_poll',
      arguments: { lookback_hours: Math.round(state.windowMs / 3600000) },
    });
    if (res?.structuredContent) {
      state.ingestSnapshot(res.structuredContent, false);
    }
  } catch {
    state.connected = false;
    state.hudDirty = true;
  } finally {
    pollingInFlight = false;
  }
}

mcpApp.ontoolinput = () => {};

mcpApp.ontoolresult = (result) => {
  receivedInitial = true;
  if (result?.isError || !result?.structuredContent) {
    const txt = (result?.content || []).map((c) => c.text || '').join(' ');
    if (txt) toast(txt);
    return;
  }
  const sc = result.structuredContent;
  applyPrefs(sc.prefs);
  state.ingestSnapshot(sc, state.firstSnapshot);
  if (sc.initial_view) {
    if (sc.initial_view.filter) {
      setInputVal('search', sc.initial_view.filter);
      state.setSearch(sc.initial_view.filter);
    }
    if (sc.initial_view.tab) openDrawer(sc.initial_view.tab);
    if (sc.initial_view.focus_agent) {
      state.layout();
      state.focus(sc.initial_view.focus_agent);
    }
  }
};

mcpApp.onhostcontextchanged = (ctx) => {
  if (ctx?.theme) applyDocumentTheme(ctx.theme);
  if (ctx?.styles?.variables) applyHostStyleVariables(ctx.styles.variables);
};

async function init() {
  resize();
  bindEvents();
  await mcpApp.connect();
  const ctx = mcpApp.getHostContext();
  if (ctx?.theme) applyDocumentTheme(ctx.theme);
  if (ctx?.styles?.variables) applyHostStyleVariables(ctx.styles.variables);
  window.requestAnimationFrame(tick);
  window.setTimeout(() => {
    if (!receivedInitial) pollNow();
  }, 1200);
  window.setInterval(() => {
    if (document.visibilityState === 'visible') pollNow();
  }, 3000);
}

init();
