export const HOST_PALETTE = ['#ffd166', '#06d6a0', '#ef476f', '#8ecae6', '#c77dff', '#f4a261', '#90be6d'];
export const RELAY_TARGET_KINDS = new Set(['claude', 'codex', 'antigravity', 'opencode', 'pi']);

export function el(id) {
  return document.getElementById(id);
}

export function setHtml(id, html) {
  const node = el(id);
  if (node) node.innerHTML = html;
}

export function show(id, on) {
  const node = el(id);
  if (!node) return;
  if (on) node.classList.remove('hidden');
  else node.classList.add('hidden');
}

export function inputVal(id) {
  const node = el(id);
  return node ? String(node.value ?? '') : '';
}

export function setInputVal(id, v) {
  const node = el(id);
  if (node) node.value = v ?? '';
}

export function checked(id) {
  const node = el(id);
  return Boolean(node && node.checked);
}

export function setChecked(id, v) {
  const node = el(id);
  if (node) node.checked = Boolean(v);
}

export function esc(s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function truncate(s, max) {
  const str = String(s ?? '');
  if (str.length <= max) return str;
  return str.slice(0, max) + '…';
}

export function ago(ms) {
  if (!ms || ms <= 0) return 'never';
  const d = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (d < 60) return `${d}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ${Math.floor((d % 3600) / 60)}m ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

export function clock(ms) {
  const d = new Date(ms);
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

export function dayClock(ms) {
  const d = new Date(ms);
  const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const p = (n) => String(n).padStart(2, '0');
  return `${days[d.getDay() % 7]} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function big(n) {
  const f = Number(n || 0);
  if (f >= 1e9) return `${(f / 1e9).toFixed(2)}B`;
  if (f >= 1e6) return `${(f / 1e6).toFixed(1)}M`;
  if (f >= 1e3) return `${(f / 1e3).toFixed(1)}k`;
  return String(Math.round(f));
}

export function bytes(n) {
  const f = Number(n || 0);
  if (f >= 1e6) return `${(f / 1e6).toFixed(1)} MB`;
  if (f >= 1e3) return `${(f / 1e3).toFixed(1)} KB`;
  return `${Math.round(f)} B`;
}

export function hashStr(s) {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h;
}

export function hostColorHex(host) {
  return HOST_PALETTE[hashStr(String(host || '')) % HOST_PALETTE.length];
}

export function shortPath(p, n) {
  const parts = String(p || '').split('/').filter(Boolean);
  if (parts.length <= n) return String(p || '');
  return `…/${parts.slice(-n).join('/')}`;
}

export function projectOf(cwd) {
  const parts = String(cwd || '').split('/').filter(Boolean);
  if (!parts.length) return '(no folder)';
  for (let i = 0; i < parts.length; i++) {
    if (parts[i] === '.claude' || parts[i] === '.worktrees') {
      if (i > 0 && (parts[i] === '.worktrees' || parts[i + 1] === 'worktrees')) return parts[i - 1];
    }
  }
  for (let i = 0; i < parts.length; i++) {
    if (parts[i] === '.codex' && parts[i + 1] === 'worktrees' && parts[i + 3]) return parts[i + 3];
  }
  if ((parts.length === 2 && (parts[0] === 'Users' || parts[0] === 'home')) || (parts.length === 1 && parts[0] === 'root')) {
    return 'home';
  }
  if ((parts.length >= 3 && parts[0] === 'private' && parts[1] === 'tmp') || parts[0] === 'tmp') return 'tmp';
  return parts[parts.length - 1];
}

export function parseHex(c) {
  const hex = String(c || '#9aa7ff').replace(/^#/, '');
  const v = Number.parseInt(hex, 16);
  if (Number.isNaN(v)) return [154, 167, 255];
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
}

export function rgba(rgb, a) {
  const alpha = Math.max(0, Math.min(1, a));
  return `rgba(${rgb[0] | 0},${rgb[1] | 0},${rgb[2] | 0},${alpha.toFixed(3)})`;
}

export function kindColor(k) {
  switch (k) {
    case 'claude': return parseHex('#ff8a4c');
    case 'codex': return parseHex('#46f5a0');
    case 'antigravity': return parseHex('#4cb8ff');
    case 'opencode': return parseHex('#2ee6d6');
    case 'pi': return parseHex('#c78bff');
    case 'operator': return parseHex('#ffffff');
    default: return parseHex('#8892b0');
  }
}

export function heat(v, lo, hi) {
  if (v <= 0) return parseHex('#3c4b78');
  const k = Math.max(0, Math.min(1, (Math.log(Math.max(v, lo)) - Math.log(lo)) / (Math.log(hi) - Math.log(lo))));
  const stops = [
    [0.0, [76, 184, 255]],
    [0.5, [255, 209, 102]],
    [1.0, [255, 59, 92]],
  ];
  const [a, b] = k < 0.5 ? [stops[0], stops[1]] : [stops[1], stops[2]];
  const u = (k - a[0]) / (b[0] - a[0]);
  return [a[1][0] + (b[1][0] - a[1][0]) * u, a[1][1] + (b[1][1] - a[1][1]) * u, a[1][2] + (b[1][2] - a[1][2]) * u];
}

export function tokensTotal(tk) {
  if (!tk) return 0;
  return (tk.input || 0) + (tk.output || 0) + (tk.cache_read || 0) + (tk.cache_write || 0);
}

export function tokensFresh(tk) {
  if (!tk) return 0;
  return (tk.input || 0) + (tk.output || 0);
}

export function nodeWeight(tk) {
  if (!tk) return 0;
  return (tk.input || 0) + (tk.output || 0) + (tk.cache_write || 0);
}

export function displayName(a) {
  if (a.label) return a.label;
  if (a.cwd) {
    const base = String(a.cwd).replace(/\/+$/, '').split('/').filter(Boolean).pop();
    if (base) return base;
  }
  return `${a.kind || 'agent'} ${String(a.id || '').slice(0, 8)}`;
}

export function sq(s) {
  if (s && /^[A-Za-z0-9._/\-+:@]+$/.test(s)) return s;
  return `'${String(s).replace(/'/g, "'\\''")}'`;
}

export function dur(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  return `${Math.floor(s / 86400)}d`;
}

export function usd(c) {
  if (c >= 100) return `$${c.toFixed(0)}`;
  if (c >= 0.995) return `$${c.toFixed(2)}`;
  return `${Math.max(0, c * 100).toFixed(0)}¢`;
}
