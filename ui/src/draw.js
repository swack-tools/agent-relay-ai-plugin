import { Health, HEALTH_META } from './insight.js';
import { bytes, displayName, hashStr, kindColor, parseHex, rgba, truncate } from './util.js';

const TAU = Math.PI * 2;
const MONO = "ui-monospace, 'SF Mono', Menlo, Consolas, monospace";

function bezierCtrl(ax, ay, bx, by, bend) {
  const mx = (ax + bx) / 2;
  const my = (ay + by) / 2;
  const dx = bx - ax;
  const dy = by - ay;
  return [mx - dy * bend, my + dx * bend];
}

export function bez(ax, ay, cx, cy, bx, by, t) {
  const u = 1 - t;
  return [u * u * ax + 2 * u * t * cx + t * t * bx, u * u * ay + 2 * u * t * cy + t * t * by];
}

export function arcPoints(app, a, b) {
  const na = app.nodes.get(a);
  const nb = app.nodes.get(b);
  if (!na || !nb) return null;
  const [ax, ay] = app.worldToScreen(na.x, na.y);
  const [bx, by] = app.worldToScreen(nb.x, nb.y);
  const [cx, cy] = a < b ? bezierCtrl(ax, ay, bx, by, 0.18) : bezierCtrl(bx, by, ax, ay, 0.18);
  return [ax, ay, cx, cy, bx, by];
}

export function distToArc(px, py, p) {
  const [ax, ay, cx, cy, bx, by] = p;
  let best = Infinity;
  let prev = [ax, ay];
  for (let i = 1; i <= 24; i++) {
    const cur = bez(ax, ay, cx, cy, bx, by, i / 24);
    const dx = cur[0] - prev[0];
    const dy = cur[1] - prev[1];
    const len2 = dx * dx + dy * dy;
    const u = len2 > 0 ? Math.max(0, Math.min(1, ((px - prev[0]) * dx + (py - prev[1]) * dy) / len2)) : 0;
    const qx = prev[0] + u * dx;
    const qy = prev[1] + u * dy;
    best = Math.min(best, Math.hypot(px - qx, py - qy));
    prev = cur;
  }
  return best;
}

function focusSet(app) {
  if (app.selection.type === 'none') return null;
  if (app.selection.type === 'channel') return new Set([app.selection.a, app.selection.b]);
  const me = app.drawnAs.get(app.selection.addr) || app.selection.addr;
  const s = new Set([me]);
  for (const c of app.channels) {
    if (c.a === me) s.add(c.b);
    if (c.b === me) s.add(c.a);
  }
  return s;
}

function drawBackground(app, ctx, w, h, t) {
  const g = ctx.createRadialGradient(w / 2, h / 2, 0, w / 2, h / 2, Math.max(w, h) * 0.75);
  g.addColorStop(0, '#0b1430');
  g.addColorStop(1, '#03060f');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, h);

  const step = 34;
  const z = app.cam.zoom;
  if (z * step > 9) {
    const [x0, y0] = app.screenToWorld(0, 0);
    const [x1, y1] = app.screenToWorld(w, h);
    const sx = Math.floor(x0 / step);
    const sy = Math.floor(y0 / step);
    const ex = Math.ceil(x1 / step);
    const ey = Math.ceil(y1 / step);
    ctx.fillStyle = 'rgba(90,130,220,0.16)';
    let count = 0;
    for (let gy = sy; gy <= ey; gy++) {
      for (let gx = sx; gx <= ex; gx++) {
        if (++count > 9000) break;
        const wx = gx * step;
        const wy = gy * step;
        const d = Math.hypot(wx, wy);
        const n = ((Math.imul(gx, 73856093) ^ Math.imul(gy, 19349663)) >>> 0) % 100 / 100;
        if (n < Math.min(0.85, d / 1800)) continue;
        const [px, py] = app.worldToScreen(wx, wy);
        ctx.fillRect(px - 0.8, py - 0.8, 1.6, 1.6);
      }
    }
  }

  const [cx, cy] = app.worldToScreen(0, 0);
  ctx.lineWidth = 1;
  for (let i = 1; i < 6; i++) {
    ctx.strokeStyle = `rgba(80,140,255,${(0.07 - i * 0.01).toFixed(3)})`;
    ctx.beginPath();
    ctx.arc(cx, cy, i * 160 * z, 0, TAU);
    ctx.stroke();
  }
  const sweep = ((t / 4000) % 1) * TAU;
  const rr = 900 * z;
  for (let k = 0; k < 24; k++) {
    const a0 = sweep - k * 0.025;
    ctx.fillStyle = `rgba(70,160,255,${(0.028 * (1 - k / 24)).toFixed(3)})`;
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.arc(cx, cy, rr, a0 - 0.025, a0);
    ctx.closePath();
    ctx.fill();
  }
}

function drawRegions(app, ctx, t) {
  for (const r of app.regions) {
    r.x += (r.tx - r.x) * 0.08;
    r.y += (r.ty - r.y) * 0.08;
    r.r += (r.tr - r.r) * 0.08;
  }
  for (const r of app.regions) {
    const [x, y] = app.worldToScreen(r.x, r.y);
    const rad = r.r * app.cam.zoom;
    const deep = Math.max(1, r.depth) - 1;
    const g = ctx.createRadialGradient(x, y, rad * 0.2, x, y, rad);
    g.addColorStop(0, rgba(r.color, 0.09 / (1 + deep * 0.5)));
    g.addColorStop(1, rgba(r.color, 0.03));
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(x, y, rad, 0, TAU);
    ctx.fill();

    ctx.lineWidth = r.depth <= 1 ? 1.6 : 1.0;
    ctx.strokeStyle = rgba(r.color, 0.55 - deep * 0.1);
    ctx.setLineDash([6, 8]);
    ctx.lineDashOffset = -t / 120;
    ctx.stroke();
    ctx.setLineDash([]);

    const fs = Math.max(9, Math.min(18, (14 - deep * 2) * Math.sqrt(app.cam.zoom)));
    if (rad < 22 || (deep > 0 && rad < 40)) continue;
    ctx.font = `${deep === 0 ? 700 : 500} ${fs.toFixed(0)}px ${MONO}`;
    ctx.textAlign = 'center';
    ctx.fillStyle = rgba(r.color, 0.95 - deep * 0.15);
    const lock = r.isolated ? ' ◆ isolated' : '';
    const text = deep === 0 ? r.label.toUpperCase() : truncate(r.label, 34);
    const ly = deep === 0 ? y - rad - 10 : y - rad + fs + 4;
    ctx.fillText(`${text}  ·  ${r.members}${lock}`, x, ly);
  }
}

function drawChannels(app, ctx, t, focus) {
  const now = Date.now();
  for (const c of app.channels) {
    const pts = arcPoints(app, c.a, c.b);
    if (!pts) continue;
    const [ax, ay, cx, cy, bx, by] = pts;
    const lit = !focus || (focus.has(c.a) && focus.has(c.b));
    const sel =
      (app.selection.type === 'channel' && app.selection.a === c.a && app.selection.b === c.b) ||
      (app.hoverChannel && ((app.hoverChannel[0] === c.a && app.hoverChannel[1] === c.b) || (app.hoverChannel[0] === c.b && app.hoverChannel[1] === c.a)));
    const recent = Math.max(0, Math.min(1, (now - c.lastMs) / 3600000));
    const baseA = (c.swarmOnly ? 0.1 : 0.22) + 0.25 * (1 - recent);
    const alpha = sel ? 0.95 : lit ? baseA : 0.04;
    const width = Math.min(6, 0.8 + Math.max(0, Math.log2(c.count)) * 0.8) * (sel ? 1.8 : 1);

    ctx.lineCap = 'round';
    ctx.strokeStyle = rgba(c.color, alpha * 0.35);
    ctx.lineWidth = width * 4;
    ctx.beginPath();
    ctx.moveTo(ax, ay);
    ctx.quadraticCurveTo(cx, cy, bx, by);
    ctx.stroke();

    ctx.strokeStyle = rgba(c.color, Math.min(1, alpha * 2.2));
    ctx.lineWidth = width;
    ctx.setLineDash([3, 9]);
    const dir = c.aToB * 2 >= c.count ? -1 : 1;
    ctx.lineDashOffset = (dir * t) / 40;
    ctx.beginPath();
    ctx.moveTo(ax, ay);
    ctx.quadraticCurveTo(cx, cy, bx, by);
    ctx.stroke();
    ctx.setLineDash([]);

    if (sel || (lit && focus)) {
      const [mx, my] = bez(ax, ay, cx, cy, bx, by, 0.5);
      ctx.font = `11px ${MONO}`;
      ctx.textAlign = 'center';
      ctx.fillStyle = 'rgba(220,230,255,0.9)';
      ctx.fillText(`${c.count} msgs · ${bytes(c.bytes)}`, mx, my - 6);
    }
  }
}

function drawPulses(app, ctx, t) {
  const arrivals = [];
  app.pulses = app.pulses.filter((p) => t - p.t0 < p.dur + 50);
  for (const p of app.pulses) {
    const k = Math.max(0, Math.min(1, (t - p.t0) / p.dur));
    if (p.internal) {
      const n = app.nodes.get(p.to);
      if (n) {
        const [x, y] = app.worldToScreen(n.x, n.y);
        const rr = (n.r + 4 + k * (18 + p.size * 2)) * Math.max(0.4, app.cam.zoom);
        ctx.strokeStyle = rgba(p.color, 0.6 * (1 - k));
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(x, y, rr, 0, TAU);
        ctx.stroke();
      }
      continue;
    }
    const pts = arcPoints(app, p.from, p.to);
    if (!pts) continue;
    const [ax, ay, cx, cy, bx, by] = pts;
    const e = 1 - Math.pow(1 - k, 2);
    const zs = Math.max(0.5, Math.min(1.6, Math.sqrt(app.cam.zoom)));
    const size = p.size * zs * (p.swarm ? 0.6 : p.cross ? 1.3 : 1);
    const tail = Math.max(0, e - 0.22 - size / 60);
    const steps = 14;
    for (let i = 0; i < steps; i++) {
      const u0 = tail + ((e - tail) * i) / steps;
      const u1 = tail + ((e - tail) * (i + 1)) / steps;
      const [x0, y0] = bez(ax, ay, cx, cy, bx, by, u0);
      const [x1, y1] = bez(ax, ay, cx, cy, bx, by, u1);
      const a = (i + 1) / steps;
      ctx.strokeStyle = rgba(p.color, 0.55 * a);
      ctx.lineWidth = Math.max(1, size * 0.9 * a);
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      ctx.lineTo(x1, y1);
      ctx.stroke();
    }
    const [hx, hy] = bez(ax, ay, cx, cy, bx, by, e);
    const g = ctx.createRadialGradient(hx, hy, 0, hx, hy, size * 2.6);
    g.addColorStop(0, 'rgba(255,255,255,0.95)');
    g.addColorStop(0.25, rgba(p.color, 0.9));
    g.addColorStop(1, rgba(p.color, 0));
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.arc(hx, hy, size * 2.6, 0, TAU);
    ctx.fill();
    if (k >= 1) arrivals.push([p.to, p.color, size]);
  }
  for (const [to, color, size] of arrivals) {
    const n = app.nodes.get(to);
    if (n) app.ripples.push({ x: n.x, y: n.y, t0: t, color, size });
  }
  app.ripples = app.ripples.filter((r) => t - r.t0 < 900);
  for (const r of app.ripples) {
    const k = (t - r.t0) / 900;
    const [x, y] = app.worldToScreen(r.x, r.y);
    ctx.strokeStyle = rgba(r.color, 0.8 * (1 - k));
    ctx.lineWidth = 2 * (1 - k) + 0.5;
    ctx.beginPath();
    ctx.arc(x, y, (10 + k * (24 + r.size * 3)) * Math.max(0.5, Math.min(1.6, Math.sqrt(app.cam.zoom))), 0, TAU);
    ctx.stroke();
  }
}

function drawNodes(app, ctx, t, focus) {
  for (const n of app.nodes.values()) {
    n.x += (n.tx - n.x) * 0.09;
    n.y += (n.ty - n.y) * 0.09;
  }
  const now = Date.now();
  const order = [...app.nodes.keys()].sort();
  for (const addr of order) {
    const n = app.nodes.get(addr);
    const a = app.agents.get(addr);
    if (!a) continue;
    const [x, y] = app.worldToScreen(n.x, n.y);
    if (x < -60 || y < -60 || x > app.width + 60 || y > app.height + 60) continue;
    const z = Math.max(0.45, Math.min(1.8, Math.sqrt(app.cam.zoom)));
    const r = n.r * z;
    const col = app.nodeColor(addr);
    const health = app.insight(addr)?.health;
    const lit = (!focus || focus.has(addr)) && app.matchesSearch(a);
    const dim = lit ? 1 : 0.18;
    const appear = Math.max(0, Math.min(1, (now - n.born) / 600));
    const recent = now - (a.last_active_ms || 0) < 5 * 60000;
    const busy = a.state === 'busy' || recent;

    if ((a.alive === true || recent) && !a.ghost) {
      const pulse = 0.5 + 0.5 * Math.sin(t / 700 + (hashStr(addr) % 100));
      const g = ctx.createRadialGradient(x, y, r * 0.5, x, y, r * (2.2 + pulse * 0.6));
      g.addColorStop(0, rgba(col, 0.35 * dim));
      g.addColorStop(1, rgba(col, 0));
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(x, y, r * 3, 0, TAU);
      ctx.fill();
    }

    ctx.fillStyle = `rgba(8,14,30,${(0.92 * Math.max(0.4, dim)).toFixed(2)})`;
    ctx.beginPath();
    ctx.arc(x, y, r * appear, 0, TAU);
    ctx.fill();

    ctx.lineWidth = app.hover === addr ? 3 : 2;
    ctx.strokeStyle = rgba(col, 0.95 * dim);
    if (a.ghost) ctx.setLineDash([2, 3]);
    ctx.stroke();
    ctx.setLineDash([]);

    ctx.fillStyle = rgba(col, 0.9 * dim);
    ctx.font = `700 ${Math.max(8, r * 0.95).toFixed(0)}px ${MONO}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    const glyph =
      a.kind === 'claude' ? 'C' :
      a.kind === 'codex' ? 'X' :
      a.kind === 'antigravity' ? 'A' :
      a.kind === 'opencode' ? 'O' :
      a.kind === 'pi' ? 'π' :
      a.kind === 'operator' ? '◎' : '?';
    ctx.fillText(glyph, x, y + 0.5);
    ctx.textBaseline = 'alphabetic';

    if (busy && !a.ghost && a.kind !== 'operator') {
      const s = (t / 500) % TAU;
      ctx.strokeStyle = rgba(col, 0.9 * dim);
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(x, y, r + 4, s, s + Math.PI * 0.6);
      ctx.stroke();
    }

    if (health === Health.Stuck || health === Health.Looping) {
      const hc = HEALTH_META[health].color;
      const beat = 0.55 + 0.45 * Math.abs(Math.sin(t / 260));
      const bx = x - r * 0.8;
      const by = y - r * 0.8;
      const br = Math.max(5.5, r * 0.42);
      ctx.strokeStyle = rgba(hc, 0.55 * beat * dim);
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(x, y, r + 7 + 3 * beat, 0, TAU);
      ctx.stroke();
      ctx.fillStyle = rgba(hc, 0.95 * dim);
      ctx.beginPath();
      ctx.arc(bx, by, br, 0, TAU);
      ctx.fill();
      ctx.fillStyle = `rgba(10,10,20,${dim.toFixed(2)})`;
      ctx.font = `700 ${(br * 1.5).toFixed(0)}px ${MONO}`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(health === Health.Stuck ? '!' : '↻', bx, by + 0.5);
      ctx.textBaseline = 'alphabetic';
    } else if (health === Health.Waiting) {
      ctx.strokeStyle = rgba(HEALTH_META[Health.Waiting].color, 0.7 * dim);
      ctx.lineWidth = 1.5;
      ctx.setLineDash([3, 4]);
      ctx.lineDashOffset = -t / 200;
      ctx.beginPath();
      ctx.arc(x, y, r + 6, 0, TAU);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    if (app.pins.has(addr)) {
      ctx.fillStyle = `rgba(255,209,102,${(0.95 * dim).toFixed(2)})`;
      ctx.font = `${Math.max(10, r * 0.9).toFixed(0)}px ${MONO}`;
      ctx.textAlign = 'center';
      ctx.fillText('★', x - r * 0.85, y + r * 1.15);
    }

    const kids = n.children.length;
    if (kids > 0 && !(app.selection.type === 'agent' && app.selection.addr === addr)) {
      const shown = Math.min(14, kids);
      for (let i = 0; i < shown; i++) {
        const th = t / 3000 + (i / shown) * TAU;
        const rr = r + 9 * z;
        ctx.fillStyle = rgba(col, 0.7 * dim);
        ctx.beginPath();
        ctx.arc(x + rr * Math.cos(th), y + rr * Math.sin(th), 1.6 * z, 0, TAU);
        ctx.fill();
      }
    }

    const talks = app.channels.some((c) => (c.a === addr || c.b === addr) && !c.swarmOnly);
    const showLabel =
      (app.cam.zoom > 1.1 && lit) ||
      (app.cam.zoom > 0.35 && lit && (talks || recent || kids > 0)) ||
      app.hover === addr ||
      a.kind === 'operator' ||
      (focus && focus.has(addr)) ||
      (!app.query.isEmpty && lit) ||
      app.pins.has(addr) ||
      health === Health.Stuck ||
      health === Health.Looping;
    if (showLabel) {
      const name = app.name(addr);
      const label = kids > 0 ? `${name} +${kids}` : name;
      ctx.font = `${Math.max(9, Math.min(14, 10.5 * z)).toFixed(0)}px ${MONO}`;
      ctx.fillStyle = `rgba(225,232,255,${(0.9 * dim).toFixed(2)})`;
      ctx.textAlign = 'center';
      ctx.fillText(truncate(label, 26), x, y + r + 14 * z);
    }
  }
}

function drawOrbit(app, ctx, t, addr) {
  const me = app.drawnAs.get(addr) || addr;
  const n = app.nodes.get(me);
  const a = app.agents.get(addr);
  if (!n || !a) return;
  const [x, y] = app.worldToScreen(n.x, n.y);
  const now = Date.now();
  const sats = (a.subagents || [])
    .slice(-12)
    .reverse()
    .map((s) => [s.name, s.kind, now - (s.last_ms || 0) < 180000 && s.status !== 'done']);
  const kids = [...n.children]
    .sort((c1, c2) => (app.agents.get(c2)?.last_active_ms || 0) - (app.agents.get(c1)?.last_active_ms || 0))
    .slice(0, 12);
  for (const c of kids) {
    const ca = app.agents.get(c);
    if (ca) sats.push([displayName(ca), 'swarm', now - (ca.last_active_ms || 0) < 300000]);
  }
  if (!sats.length) return;
  const col = kindColor(a.kind);
  const per = 9;
  const base = n.r * Math.max(0.45, Math.min(1.8, Math.sqrt(app.cam.zoom))) + 70;
  const rings = Math.ceil(sats.length / per);
  for (let ring = 0; ring < rings; ring++) {
    ctx.strokeStyle = rgba(col, 0.18);
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(x, y, base + ring * 62, 0, TAU);
    ctx.stroke();
  }
  sats.forEach(([name, kind, live], i) => {
    const ring = Math.floor(i / per);
    const inRing = Math.min(per, sats.length - ring * per);
    const rr = base + ring * 62;
    const th = (t / (14000 + ring * 5000)) * (ring % 2 === 0 ? 1 : -1) + ((i % per) / inRing) * TAU + ring * 0.35;
    const sx = x + rr * Math.cos(th);
    const sy = y + rr * Math.sin(th);
    ctx.strokeStyle = rgba(col, 0.25);
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.lineTo(sx, sy);
    ctx.stroke();
    const c2 = kind === 'swarm' ? parseHex('#7fd4ff') : col;
    ctx.fillStyle = rgba(c2, live ? 0.95 : 0.45);
    ctx.beginPath();
    ctx.arc(sx, sy, live ? 5 : 3.5, 0, TAU);
    ctx.fill();
    ctx.font = `10px ${MONO}`;
    ctx.textAlign = 'center';
    ctx.fillStyle = live ? 'rgba(235,240,255,0.95)' : 'rgba(200,210,240,0.55)';
    ctx.fillText(truncate(name, 22), sx, sy - 9);
  });
}

export function drawFrame(app, ctx, t) {
  ctx.setTransform(app.dpr, 0, 0, app.dpr, 0, 0);
  drawBackground(app, ctx, app.width, app.height, t);
  drawRegions(app, ctx, t);
  const focus = focusSet(app);
  drawChannels(app, ctx, t, focus);
  drawPulses(app, ctx, t);
  drawNodes(app, ctx, t, focus);
  if (app.selection.type === 'agent') drawOrbit(app, ctx, t, app.selection.addr);
}
