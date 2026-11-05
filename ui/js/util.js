// Gemeinsame Helfer. Datei- und Ordnernamen sind fremde Daten: Sie landen immer ueber
// textContent im DOM, nie ueber innerHTML.

let token = '';

// Das Fenster startet mit einem Einmal-Code und tauscht ihn gegen das Zugriffs-Token.
// So steht das Token in keiner Kommandozeile. Mit --no-open kommt es direkt als ?t=.
export async function initToken() {
  const u = new URL(location.href);
  let t = u.searchParams.get('t');
  const code = u.searchParams.get('c');
  if (code) {
    const r = await fetch('/session', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ code }) }).catch(() => null);
    const j = r && r.ok ? await r.json().catch(() => null) : null;
    t = j?.token || null;
  }
  try {
    if (t) sessionStorage.setItem('sm-token', t);
    token = t || sessionStorage.getItem('sm-token') || '';
  } catch { token = t || ''; }
  if (t || code) {
    u.searchParams.delete('t');
    u.searchParams.delete('c');
    history.replaceState(null, '', u.pathname + u.hash);
  }
}
export const authToken = () => token;

export async function api(path, body) {
  const opt = { headers: { 'X-SM-Token': token } };
  if (body !== undefined) {
    opt.method = 'POST';
    opt.body = JSON.stringify(body);
    opt.headers['Content-Type'] = 'application/json';
  }
  const r = await fetch('/api/' + path, opt);
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `Fehler ${r.status}`);
  return j;
}

export function qs(obj) {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(obj)) {
    if (v !== undefined && v !== null && v !== '' && v !== false) p.set(k, v);
  }
  return p.toString();
}

// Windows rechnet in 1024er-Schritten und nennt das GB. Wir zeigen dasselbe, damit die Zahlen zum Dateimanager passen.
const nf0 = new Intl.NumberFormat('de-DE', { maximumFractionDigits: 0 });
const nf1 = new Intl.NumberFormat('de-DE', { minimumFractionDigits: 1, maximumFractionDigits: 1 });
const nf2 = new Intl.NumberFormat('de-DE', { minimumFractionDigits: 2, maximumFractionDigits: 2 });

export function fmtBytes(n) {
  if (n === null || n === undefined) return '–';
  if (n < 1024) return `${nf0.format(n)} B`;
  const units = ['KB', 'MB', 'GB', 'TB', 'PB'];
  let v = n, i = -1;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  const s = v >= 100 ? nf0.format(v) : v >= 10 ? nf1.format(v) : nf2.format(v);
  return `${s} ${units[i]}`;
}
export const fmtNum = (n) => (n === null || n === undefined ? '–' : nf0.format(n));
export const fmtPct = (x) => (x >= 0.1 || x === 0 ? nf0.format(x * 100) : x >= 0.001 ? nf1.format(x * 100) : '< 0,1') + ' %';

export function fmtDate(ts) {
  if (!ts) return '–';
  return new Date(ts * 1000).toLocaleDateString('de-DE', { day: '2-digit', month: '2-digit', year: 'numeric' });
}
export function fmtDateTime(ts) {
  if (!ts) return '–';
  return new Date(ts * 1000).toLocaleString('de-DE', { day: '2-digit', month: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit' });
}
export function fmtAgo(ts) {
  if (!ts) return '–';
  const s = Date.now() / 1000 - ts;
  if (s < 90) return 'gerade eben';
  if (s < 3600) return `vor ${Math.round(s / 60)} Min.`;
  if (s < 86400) return `vor ${Math.round(s / 3600)} Std.`;
  const d = s / 86400;
  if (d < 2) return 'gestern';
  if (d < 60) return `vor ${Math.round(d)} Tagen`;
  if (d < 700) return `vor ${Math.round(d / 30.4)} Monaten`;
  return `vor ${Math.round(d / 365)} Jahren`;
}
export function fmtDuration(ms) {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`;
  return `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`;
}

export function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2).toLowerCase(), v);
      else if (k === 'value') el.value = v;
      else if (k === 'checked') el.checked = !!v;
      else if (k === 'disabled') el.disabled = !!v;
      else el.setAttribute(k, v === true ? '' : v);
    }
  }
  append(el, kids);
  return el;
}
function append(el, kids) {
  for (const k of kids) {
    if (k === null || k === undefined || k === false) continue;
    if (Array.isArray(k)) append(el, k);
    else el.append(k instanceof Node ? k : document.createTextNode(String(k)));
  }
}
export const clear = (el) => { el.replaceChildren(); return el; };
/// Ersetzt den Inhalt. Anders als replaceChildren ueberspringt das leere Werte, statt "null" zu schreiben.
export function add(el, ...kids) { append(el, kids); return el; }
export function fill(el, ...kids) { el.replaceChildren(); append(el, kids); return el; }

// Feste Symbole, keine Fremddaten: innerHTML ist hier unbedenklich.
const ICONS = {
  dash: '<rect x="2" y="2" width="5" height="7"/><rect x="9" y="2" width="5" height="4"/><rect x="9" y="8" width="5" height="6"/><rect x="2" y="11" width="5" height="3"/>',
  map: '<rect x="2" y="2" width="12" height="12"/><path d="M8 2v12M8 7h6M2 10h6M11 7v7"/>',
  rings: '<circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="2.5"/><path d="M8 2v3.5M13.2 11l-3-1.7"/>',
  tree: '<path d="M3 2v11M3 5h4M3 10h4M3 13h0"/><rect x="7" y="3" width="7" height="4"/><rect x="7" y="8" width="7" height="4"/>',
  files: '<path d="M4 2h5l3 3v9H4z"/><path d="M9 2v3h3"/><path d="M6 8h4M6 11h4"/>',
  types: '<path d="M3 13V7M6.5 13V3M10 13V9M13.5 13V5"/>',
  dup: '<rect x="2" y="5" width="9" height="9"/><path d="M5 5V2h9v9h-3"/>',
  clock: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.5 1.5"/>',
  broom: '<path d="M10.5 2 7 7.5M4 7.5h5.5L12 14H2.5z"/><path d="M6 14l.7-3M9 14l-.3-3"/>',
  history: '<path d="M2 13l3.5-4 3 2L14 4"/><path d="M10.5 4H14v3.5"/>',
  search: '<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/>',
  folder: '<path d="M2 4h4l1.5 2H14v7H2z"/>',
  file: '<path d="M4 2h5l3 3v9H4z"/><path d="M9 2v3h3"/>',
  chevron: '<path d="M6 3l5 5-5 5"/>',
  x: '<path d="M4 4l8 8M12 4l-8 8"/>',
  trash: '<path d="M3 4.5h10M6 4.5V2.5h4v2M4.5 4.5 5 14h6l.5-9.5"/>',
  move: '<path d="M2 8h9M8 4.5 11.5 8 8 11.5"/><path d="M14 3v10"/>',
  explorer: '<path d="M2 4h4l1.5 2H14v7H2z"/><path d="M8 9.5h3.5M10 8l1.5 1.5L10 11"/>',
  open: '<path d="M6 3H3v10h10v-3"/><path d="M9 2h5v5M14 2 8 8"/>',
  copy: '<rect x="5" y="5" width="9" height="9"/><path d="M11 5V2H2v9h3"/>',
  plus: '<path d="M8 3v10M3 8h10"/>',
  minus: '<path d="M3 8h10"/>',
  check: '<path d="M3 8.5 6.5 12 13 4.5"/>',
  warn: '<path d="M8 2 15 14H1z"/><path d="M8 6.5v3.5M8 12v.2"/>',
  lock: '<rect x="3" y="7" width="10" height="7"/><path d="M5 7V5a3 3 0 0 1 6 0v2"/>',
  shield: '<path d="M8 1.5 14 4v4c0 3.5-2.5 5.5-6 6.5C4.5 13.5 2 11.5 2 8V4z"/><path d="M5.5 8 7.3 10 10.5 6"/>',
  info: '<circle cx="8" cy="8" r="6"/><path d="M8 7.5V11M8 5v.2"/>',
  refresh: '<path d="M13 8a5 5 0 1 1-1.5-3.5"/><path d="M13 2v3h-3"/>',
  drive: '<rect x="2" y="4" width="12" height="8" rx="1"/><path d="M4.5 9.5h.2M7 9.5h.2"/>',
  up: '<path d="M8 13V3M4 7l4-4 4 4"/>',
  theme: '<circle cx="8" cy="8" r="6"/><path d="M8 2a6 6 0 0 1 0 12z" fill="currentColor"/>',
  menu: '<path d="M8 3v.2M8 8v.2M8 13v.2"/>',
  power: '<path d="M8 2v6"/><path d="M4.5 4.2a5 5 0 1 0 7 0"/>',
  export: '<path d="M8 10V2M5 5l3-3 3 3"/><path d="M3 9v5h10V9"/>',
  plug: '<path d="M6 2v3.5M10 2v3.5"/><path d="M4 5.5h8V8a4 4 0 0 1-8 0z"/><path d="M8 12v2.5"/>',
};
export function icon(name) {
  const s = document.createElement('span');
  s.className = 'icon';
  s.innerHTML = `<svg viewBox="0 0 16 16" aria-hidden="true">${ICONS[name] || ''}</svg>`;
  return s;
}

export const cssVar = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
export const CAT_NAMES = ['Videos', 'Bilder', 'Audio', 'Dokumente', 'Archive & Abbilder', 'Programme & Spiele', 'Entwicklung & Daten', 'System & Temp', 'Sonstige'];
export const catVar = (i) => `var(--c${i})`;
/// Textfarbe auf einer Kategorie-Flaeche: weiss, ausser auf Gelb.
export const catInk = (i) => (i === 3 ? '#0b0b0b' : '#ffffff');

// Tooltip
const tipEl = () => document.getElementById('tooltip');
export function showTip(x, y, content) {
  const t = tipEl();
  t.replaceChildren(...(Array.isArray(content) ? content : [content]));
  t.hidden = false;
  const r = t.getBoundingClientRect();
  const nx = x + 14 + r.width > innerWidth ? Math.max(4, x - r.width - 10) : x + 14;
  const ny = y + 14 + r.height > innerHeight ? Math.max(4, y - r.height - 10) : y + 14;
  t.style.left = `${nx}px`;
  t.style.top = `${ny}px`;
}
export const hideTip = () => { tipEl().hidden = true; };
export function tipBlock(value, name, sub, path) {
  return [
    value ? h('div', { class: 'tv' }, value) : null,
    name ? h('div', { class: 'tn' }, name) : null,
    sub ? h('div', { class: 'ts' }, sub) : null,
    path ? h('div', { class: 'tp' }, path) : null,
  ].filter(Boolean);
}
export function withTip(el, fn) {
  el.addEventListener('pointermove', (e) => showTip(e.clientX, e.clientY, fn()));
  el.addEventListener('pointerleave', hideTip);
  el.addEventListener('focus', () => { const r = el.getBoundingClientRect(); showTip(r.left, r.bottom, fn()); });
  el.addEventListener('blur', hideTip);
  return el;
}

export function toast(msg, kind) {
  const t = h('div', { class: 'toast' + (kind === 'err' ? ' err' : '') }, msg);
  document.getElementById('toasts').append(t);
  setTimeout(() => t.remove(), kind === 'err' ? 7000 : 3600);
}
export const fail = (e) => toast(e.message || String(e), 'err');

let menuEl = null;
export function closeMenu() { if (menuEl) { menuEl.remove(); menuEl = null; } }
export function menu(x, y, items) {
  closeMenu();
  hideTip();
  menuEl = h('div', { class: 'menu', role: 'menu' }, items.map((it) => {
    if (it === '-') return h('hr');
    return h('button', {
      role: 'menuitem', disabled: it.disabled,
      onclick: () => { closeMenu(); it.run(); },
    }, it.icon ? icon(it.icon) : null, it.label);
  }));
  document.body.append(menuEl);
  const r = menuEl.getBoundingClientRect();
  menuEl.style.left = `${Math.min(x, innerWidth - r.width - 6)}px`;
  menuEl.style.top = `${Math.min(y, innerHeight - r.height - 6)}px`;
  menuEl.querySelector('button:not(:disabled)')?.focus();
}
document.addEventListener('pointerdown', (e) => { if (menuEl && !menuEl.contains(e.target)) closeMenu(); });
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenu(); });

export function dialog({ title, body, actions = [], wide, onClose }) {
  const prev = document.activeElement;
  const close = () => {
    scrim.remove();
    document.removeEventListener('keydown', key, true);
    if (prev && prev.focus) prev.focus();
    onClose && onClose();
  };
  const key = (e) => {
    if (e.key === 'Escape') { e.stopPropagation(); close(); }
  };
  const footer = h('footer');
  const setActions = (acts) => {
    footer.replaceChildren(...acts.map((a) => a instanceof Node ? a : h('button', {
      class: 'btn ' + (a.kind || ''), disabled: a.disabled, id: a.id,
      onclick: async () => { const keep = await a.run?.(); if (!keep) close(); },
    }, a.icon ? icon(a.icon) : null, a.label)));
  };
  setActions(actions);
  const bodyEl = h('div', { class: 'body' }, body);
  const box = h('div', { class: 'dialog' + (wide ? ' wide' : ''), role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
    h('header', null, h('h2', null, title)), bodyEl, footer);
  const scrim = h('div', { class: 'scrim', onpointerdown: (e) => { if (e.target === scrim) close(); } }, box);
  document.body.append(scrim);
  document.addEventListener('keydown', key, true);
  const first = box.querySelector('input, select, footer .btn.primary, footer .btn');
  first?.focus();
  return { close, setActions, bodyEl, setBody: (...b) => bodyEl.replaceChildren(...b.filter(Boolean)) };
}

// Wird von app.js gefuellt, damit die Ansichten keine Importschleifen brauchen.
export const App = {};

export const RISK_LABEL = { safe: 'Sicher', caution: 'Vorsicht', review: 'Prüfen', info: 'Info' };
export function riskBadge(r) {
  const ic = { safe: 'shield', caution: 'warn', review: 'warn', info: 'info' }[r] || 'info';
  return h('span', { class: 'risk ' + r }, icon(ic), RISK_LABEL[r] || r);
}

export function rowMenu(row, x, y) {
  const blocked = row.guard && row.guard.level === 'block';
  const isDir = row.dir;
  menu(x, y, [
    { label: App.isWindows() ? 'Im Explorer zeigen' : 'Im Dateimanager zeigen', icon: 'explorer', run: () => App.reveal(row.id) },
    { label: isDir ? 'Ordner öffnen' : 'Datei öffnen', icon: 'open', run: () => App.open(row.id) },
    '-',
    { label: 'In der Karte zeigen', icon: 'map', run: () => App.go('map', isDir ? row.id : row.parent) },
    { label: 'Im Baum zeigen', icon: 'tree', run: () => App.go('tree', row.id) },
    { label: 'Pfad kopieren', icon: 'copy', run: () => App.copyPath(row.id, row.path) },
    '-',
    { label: blocked ? 'Geschützt' : 'Zur Aufräumliste', icon: blocked ? 'lock' : 'plus', disabled: blocked, run: () => App.addToCart([row.id]) },
  ]);
}

export function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}
