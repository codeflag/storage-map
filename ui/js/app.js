// Rahmen der Anwendung: Navigation, Statusabfrage, Inspektor, Aufraeumliste.

import {
  initToken, authToken, api, qs, h, icon, clear, fill, App, fmtBytes, fmtNum, fmtDate, fmtAgo,
  catVar, CAT_NAMES, fail, toast, dialog, hideTip, closeMenu, rowMenu,
} from './util.js';
import { severity } from './charts.js';
import { viewHome, viewScanning, viewOverview, viewMap, viewRings, viewTree } from './views.js';
import { viewFiles, viewTypes, viewDups, viewOld, viewClean, viewHistory, fileRow } from './views2.js';
import { viewMcp, LEVEL_NAME } from './views3.js';

const S = {
  status: null, view: null, id: null, params: {}, selId: null, selDir: false, summary: null,
  mapState: { mode: 'type', detail: 'mittel', showFree: false },
  sideOpen: true, lastGen: 0, lastState: null,
};

const NAV = [
  'Ansichten',
  ['overview', 'Übersicht', 'dash', viewOverview],
  ['map', 'Speicherkarte', 'map', viewMap],
  ['rings', 'Ringe', 'rings', viewRings],
  ['tree', 'Ordner', 'tree', viewTree],
  ['files', 'Große Dateien', 'files', viewFiles],
  ['types', 'Dateitypen', 'types', viewTypes],
  'Aufräumen & Verlauf',
  ['clean', 'Aufräumen', 'broom', viewClean],
  ['dups', 'Duplikate', 'dup', viewDups],
  ['old', 'Alt & ungenutzt', 'clock', viewOld],
  ['history', 'Verlauf', 'history', viewHistory],
  'Zugriff',
  ['mcp', 'KI-Zugriff', 'plug', viewMcp],
];
const NAV_ITEMS = NAV.filter((n) => Array.isArray(n));
const VIEWS = Object.fromEntries(NAV_ITEMS.map((n) => [n[0], n]));

App.isWindows = () => S.status?.platform !== 'linux';

const appEl = document.getElementById('app');
let mainEl, navEl, sideEl, trayEl, chipEl, searchEl, bannerEl, shellEl;
let current = null;
let scanUpdate = null;

function theme() {
  let t = null;
  try { t = localStorage.getItem('sm-theme'); } catch { /* egal */ }
  if (t) document.documentElement.dataset.theme = t;
  const dark = t ? t === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.dataset.resolvedTheme = dark ? 'dark' : 'light';
}
theme();
function toggleTheme() {
  const dark = document.documentElement.dataset.resolvedTheme === 'dark';
  const next = dark ? 'light' : 'dark';
  try { localStorage.setItem('sm-theme', next); } catch { /* egal */ }
  document.documentElement.dataset.theme = next;
  document.documentElement.dataset.resolvedTheme = next;
  remount();
}

function shell() {
  bannerEl = h('div', { class: 'banner', hidden: true });
  chipEl = h('button', { class: 'chip', title: 'Laufwerk wechseln', onclick: () => App.closeIndex() });
  searchEl = h('input', {
    type: 'search', placeholder: 'Dateien und Ordner suchen…', 'aria-label': 'Suche',
    onkeydown: (e) => {
      if (e.key === 'Enter' && searchEl.value.trim()) { App.go('files', null, { q: searchEl.value.trim() }); searchEl.value = ''; searchEl.blur(); }
      if (e.key === 'Escape') searchEl.blur();
    },
  });
  const top = h('div', { class: 'top' },
    h('button', { class: 'brand', onclick: () => App.go('overview'), 'aria-label': 'Zur Übersicht' },
      h('img', { class: 'mark', src: 'favicon.svg', alt: '' }), h('span', { class: 'word' }, 'STORAGE MAP')),
    chipEl,
    h('button', { class: 'btn', id: 'rescan', onclick: () => { const r = S.status?.index?.root; if (r) App.startScan(r); } }, icon('refresh'), 'Neu scannen'),
    h('div', { class: 'search' }, icon('search'), searchEl, h('kbd', null, 'Strg K')),
    h('button', { class: 'btn icon-only', title: 'Hell / Dunkel', 'aria-label': 'Hell oder dunkel', onclick: toggleTheme }, icon('theme')),
    h('button', { class: 'btn icon-only', title: 'Storage Map beenden', 'aria-label': 'Beenden', onclick: () => App.quit() }, icon('power')));
  navEl = h('nav', { class: 'nav', 'aria-label': 'Ansichten' });
  mainEl = h('main', { class: 'main', tabindex: -1 });
  sideEl = h('aside', { class: 'side closed', 'aria-label': 'Details' });
  trayEl = h('div', { class: 'tray', hidden: true });
  trayEl.style.display = 'none';
  shellEl = h('div', { class: 'shell' }, bannerEl, top, navEl, mainEl, sideEl, trayEl);
  fill(appEl, shellEl);
}
shell();

function drawNav() {
  const inIdx = !!S.status?.index;
  navEl.style.visibility = inIdx ? '' : 'hidden';
  clear(navEl);
  let n = 1;
  NAV.forEach((item) => {
    if (typeof item === 'string') { if (item !== 'Ansichten') navEl.append(h('div', { class: 'label sep' }, item)); return; }
    const [key, label, ic] = item;
    const level = S.status?.mcpLevel;
    const badge = key === 'clean' && S.summary?.safeBytes ? fmtBytes(S.summary.safeBytes) : key === 'dups' && S.status?.dups?.running ? 'läuft' : key === 'mcp' ? LEVEL_NAME[level] : null;
    navEl.append(h('button', { class: S.view === key ? 'on' : '', 'aria-current': S.view === key ? 'page' : undefined, title: `${label} (${n})`, onclick: () => App.go(key) },
      icon(ic), h('span', null, label), badge ? h('span', { class: 'badge num' + (key === 'mcp' && (level === 'write' || level === 'delete') ? ' badge-warn' : '') }, badge) : null));
    n++;
  });
  navEl.append(h('div', { class: 'foot' }, `Version ${S.status?.version || ''}`));
}

let chipKey = null, trayKey = null;
function drawChip() {
  const ix = S.status?.index;
  chipEl.style.display = ix ? '' : 'none';
  shellEl.querySelector('#rescan').style.display = ix ? '' : 'none';
  if (!ix) { chipKey = null; return; }
  const key = `${ix.root}|${ix.free}|${ix.total}|${ix.alloc}`;
  if (key === chipKey) return;
  chipKey = key;
  const used = ix.total - ix.free;
  const share = ix.total ? used / ix.total : 0;
  const sev = severity(share);
  fill(chipEl, 
    icon('drive'), h('b', { class: 'ellipsis' }, ix.root),
    ix.total ? h('div', { class: 'chip-meter' }, h('i', { style: { width: `${share * 100}%`, background: sev.color } })) : null,
    ix.total ? h('span', { class: 'num soft', style: { whiteSpace: 'nowrap' } }, `${fmtBytes(ix.free)} frei`) : h('span', { class: 'num soft' }, fmtBytes(ix.alloc)));
}

function drawBanner() {
  const m = S.status?.mode;
  const text = m === 'demo' ? 'DEMO: Erfundene Laufwerke und Dateien. Es wird nichts gelesen und nichts gelöscht.'
    : m === 'sandbox' ? 'TESTBETRIEB: Nur der Testordner wird angefasst. Papierkorb und Dateimanager sind simuliert.' : '';
  bannerEl.hidden = !text;
  bannerEl.style.display = text ? '' : 'none';
  bannerEl.textContent = text;
}

function drawTray() {
  const c = S.status?.cart;
  const show = c && c.count > 0;
  trayEl.style.display = show ? '' : 'none';
  if (!show) { trayKey = null; return; }
  const key = `${c.count}|${c.bytes}`;
  if (key === trayKey) return;
  trayKey = key;
  fill(trayEl, 
    icon('broom'), h('span', null, 'Aufräumliste:'),
    h('b', null, fmtBytes(c.bytes)), h('span', { class: 'muted num' }, `${fmtNum(c.count)} ${c.count === 1 ? 'Eintrag' : 'Einträge'}`),
    h('span', { class: 'grow' }),
    h('button', { class: 'btn primary', onclick: () => openCart() }, 'Prüfen und aufräumen…'));
}

/* ---------- Router ---------- */
function parseHash() {
  const raw = location.hash.replace(/^#\/?/, '');
  const [path, query] = raw.split('?');
  const parts = path.split('/').filter(Boolean);
  const params = Object.fromEntries(new URLSearchParams(query || ''));
  for (const k of ['cat']) if (k in params) params[k] = Number(params[k]);
  return { view: parts[0] || 'overview', id: parts[1] !== undefined ? Number(parts[1]) : null, params };
}
App.go = (view, id, params) => {
  const q = params ? '?' + qs(params) : '';
  const target = `#/${view}${id !== null && id !== undefined ? '/' + id : ''}${q}`;
  if (location.hash === target) { route(true); return; }
  location.hash = target;
};
window.addEventListener('hashchange', () => route());

async function teardown() {
  hideTip();
  closeMenu();
  if (current) {
    const d = current.destroy || (typeof current === 'function' ? current : null);
    try { d && d(); } catch { /* egal */ }
    current = null;
  }
}

let routeToken = 0;
async function route(force) {
  const st = S.status;
  if (!st) return;
  const mine = ++routeToken;
  if (st.scan) { showScan(); return; }
  let { view, id, params } = parseHash();
  // KI-Zugriff braucht keinen Scan, alles andere schon.
  if (!st.index && view !== 'mcp') { showHome(); return; }
  if (!VIEWS[view]) view = 'overview';
  // Wechsel innerhalb derselben Ansicht: nur aktualisieren (Karte zoomt, Baum klappt auf).
  if (!force && current && S.view === view && current.update && id !== null) {
    S.id = id;
    current.update(id);
    return;
  }
  await teardown();
  S.view = view; S.id = id; S.params = params;
  drawNav();
  mainEl.className = 'main' + (view === 'map' || view === 'rings' ? ' flush' : '');
  // Jeder Aufruf zeichnet in einen eigenen Behaelter: Ein langsamerer, ueberholter Aufruf landet so im Leeren.
  const host = h('div', { class: 'view-host' });
  fill(mainEl, host);
  mainEl.scrollTop = 0;
  try {
    const r = await VIEWS[view][3](host, S, id, params);
    if (mine !== routeToken) { const d = r?.destroy || (typeof r === 'function' ? r : null); d && d(); return; }
    current = r || null;
  } catch (e) {
    if (mine === routeToken) host.append(h('div', { class: 'panel empty' }, e.message));
  }
  drawNav();
}
function remount() { route(true); if (S.selId !== null) App.select(S.selId); }

function showHome() {
  teardown();
  S.view = null; S.summary = null;
  drawNav();
  closeSide();
  mainEl.className = 'main';
  clear(mainEl);
  viewHome(mainEl, S);
}
function showScan() {
  teardown();
  S.view = null; S.summary = null;
  drawNav();
  closeSide();
  mainEl.className = 'main';
  clear(mainEl);
  scanUpdate = viewScanning(mainEl);
  scanUpdate(S.status.scan);
}

/* ---------- Status ---------- */
async function poll() {
  let st;
  try { st = await api('status'); } catch (e) {
    if (String(e.message).includes('angemeldet')) {
      clear(mainEl);
      mainEl.append(h('div', { class: 'panel empty' }, 'Sitzung abgelaufen. Bitte Storage Map neu starten.'));
    }
    return null;
  }
  const prev = S.status;
  S.status = st;
  const state = st.scan ? 'scan' : st.index ? `index:${st.index.root}:${st.index.scannedAt}` : 'home';
  if (state !== S.lastState) {
    const wasScan = S.lastState === 'scan';
    S.lastState = state;
    S.lastGen = st.gen;
    S.selId = null;
    if (wasScan && st.scanError) toast(st.scanError, 'err');
    drawBanner(); drawChip(); drawTray();
    if (st.index) { if (!location.hash || wasScan) location.hash = '#/overview'; }
    else if (S.view === 'mcp') location.hash = '';
    await route(true);
    return st;
  }
  if (st.scan && scanUpdate) scanUpdate(st.scan);
  drawChip(); drawTray();
  if (st.gen !== S.lastGen) {
    S.lastGen = st.gen;
    S.summary = null;
    if (st.index && !st.cleanup.running) {
      api('summary').then((s) => { S.summary = s; drawNav(); }).catch(() => {});
      if (current?.refresh) current.refresh();
      else if (['overview', 'types', 'history'].includes(S.view)) route(true);
      if (S.selId !== null) App.select(S.selId, true);
    }
  }
  if (current?.tick) current.tick(st);
  if (prev?.dups?.running !== st.dups.running || prev?.mcpLevel !== st.mcpLevel) drawNav();
  handleCleanup(st);
  return st;
}
App.poll = poll;
App.home = () => route(true);

let pollTimer;
function loop() {
  const fast = S.status?.scan || S.status?.dups?.running || S.status?.cleanup?.running;
  pollTimer = setTimeout(async () => { await poll(); loop(); }, fast ? 300 : 1500);
}

/* ---------- Aktionen ---------- */
App.startScan = (path) => api('scan', { path }).then(() => poll()).catch(fail);
App.openCached = (path) => api('open-cached', { path }).then(() => poll()).catch(fail);
App.closeIndex = () => api('close', {}).then(() => { location.hash = ''; return poll(); }).catch(fail);
App.reveal = (id) => api('reveal', { id }).then((r) => { if (r.simulated) toast('Im Test-Betrieb nur simuliert.'); }).catch(fail);
App.open = (id) => api('open', { id }).then((r) => { if (r.simulated) toast('Im Test-Betrieb nur simuliert.'); }).catch(fail);
App.copyPath = async (id, path) => {
  try {
    const p = path || (await api('node?' + qs({ id }))).path;
    await navigator.clipboard.writeText(p);
    toast('Pfad kopiert.');
  } catch (e) { fail(e); }
};
App.afterCart = (r) => {
  if (r.blocked?.length) {
    toast(`${r.blocked.length === 1 ? 'Ein geschützter Eintrag wurde' : `${fmtNum(r.blocked.length)} geschützte Einträge wurden`} ausgelassen: ${r.blocked[0].why}`, 'err');
  }
  if (r.added) toast(`${fmtNum(r.added)} hinzugefügt. Aufräumliste: ${fmtBytes(r.bytes)}`);
  else if (!r.blocked?.length) toast('Schon auf der Aufräumliste.');
  poll();
};
App.addToCart = async (ids) => {
  const r = await api('cart/add', { ids }).catch(fail);
  if (r) App.afterCart(r);
  return r;
};
App.elevate = () => api('elevate', {}).then(() => toast('Storage Map startet neu…')).catch(fail);
App.quit = () => {
  dialog({
    title: 'Storage Map beenden?',
    body: h('p', { class: 'soft' }, 'Der letzte Scan bleibt gespeichert und lässt sich beim nächsten Start sofort öffnen.'),
    actions: [
      { label: 'Abbrechen' },
      { label: 'Beenden', kind: 'primary', run: () => api('quit', {}).then((r) => {
        // Das eigene Fenster schliesst das Programm selbst. Einen Tab im normalen Browser darf eine Seite nicht schliessen.
        fill(document.body, h('div', { class: 'empty', style: { padding: '20vh 0' } }, r.closesWindow ? 'Storage Map wird beendet…' : 'Storage Map wurde beendet. Du kannst diesen Tab schließen.'));
        window.close();
      }).catch(fail) },
    ],
  });
};
App.emptyBin = (bytes) => {
  dialog({
    title: 'Papierkorb leeren?',
    body: [
      h('p', null, `Das gibt ${fmtBytes(bytes)} auf diesem Laufwerk frei.`),
      h('div', { class: 'notice warn' }, icon('warn'), h('div', null, App.isWindows() ? 'Die Dateien im Papierkorb sind danach endgültig weg. Windows fragt zur Sicherheit noch einmal nach.' : 'Die Dateien im Papierkorb sind danach endgültig weg.')),
    ],
    actions: [
      { label: 'Abbrechen' },
      { label: 'Papierkorb leeren', kind: 'danger', icon: 'trash', run: () => api('recycle/empty', {}).then(() => { toast('Erledigt.'); poll(); }).catch(fail) },
    ],
  });
};
App.showDenied = async () => {
  const d = await api('denied').catch(fail);
  if (!d) return;
  dialog({
    title: 'Nicht erfasst',
    wide: true,
    body: [
      h('p', { class: 'soft' }, App.isWindows()
        ? 'Windows meldet mehr belegten Platz, als Dateien gefunden wurden. Das sind Systemverwaltung (MFT), Schattenkopien und Ordner ohne Leserechte. Mit Adminrechten lässt sich mehr erfassen.'
        : 'Das System meldet mehr belegten Platz, als Dateien gefunden wurden. Das sind Verwaltungsdaten des Dateisystems, reservierter Platz, Snapshots und Ordner ohne Leserechte. Mit sudo gestartet lässt sich mehr erfassen.'),
      d.total ? h('div', { class: 'label' }, `${fmtNum(d.total)} nicht lesbare Ordner`) : null,
      d.total ? h('div', { class: 'dirlist', style: { height: '260px' } }, d.rows.map((r) => h('div', { class: 'mono', style: { padding: '3px 8px', borderBottom: '1px solid var(--line)' } }, r.path))) : null,
    ],
    actions: [{ label: 'Schließen' }, S.status?.elevated || !S.status?.canElevate ? null : { label: 'Mit Adminrechten neu starten', kind: 'primary', icon: 'shield', run: () => App.elevate() }].filter(Boolean),
  });
};

App.pickFolder = (title, done) => {
  let cur = '';
  const input = h('input', { class: 'field', style: { width: '100%' }, placeholder: App.isWindows() ? 'D:\\Daten' : '/mnt/daten', 'aria-label': 'Pfad' });
  const list = h('div', { class: 'dirlist' });
  const load = async (p) => {
    try {
      const d = await api('browse?' + qs({ path: p }));
      cur = d.path;
      input.value = d.path;
      clear(list);
      if (d.path) list.append(h('button', { onclick: () => load(d.parent || '') }, icon('up'), '..'));
      d.dirs.forEach((x) => list.append(h('button', { onclick: () => load(x.path) }, icon('folder'), x.name)));
      if (!d.dirs.length) list.append(h('div', { class: 'empty' }, 'Keine Unterordner.'));
    } catch (e) { fail(e); }
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') load(input.value); });
  dialog({
    title,
    body: [input, list],
    actions: [
      { label: 'Abbrechen' },
      { label: 'Diesen Ordner wählen', kind: 'primary', run: () => { const p = input.value.trim() || cur; if (!p) { toast('Bitte einen Ordner wählen.'); return true; } done(p); return false; } },
    ],
  });
  load('');
};

/* ---------- Inspektor ---------- */
function closeSide() { S.selId = null; sideEl.classList.add('closed'); clear(sideEl); }
App.select = async (id, silent) => {
  if (id === null || id === undefined || id < 0) return;
  let n;
  try { n = await api('node?' + qs({ id })); } catch (e) { if (!silent) fail(e); closeSide(); return; }
  S.selId = id;
  S.selDir = n.dir;
  sideEl.classList.remove('closed');
  const parent = n.ancestors.at(-2);
  const blocked = n.guard.level === 'block';
  const flags = [];
  if (n.flags & 2) flags.push('versteckt');
  if (n.flags & 4) flags.push('Systemdatei');
  if (n.flags & 8) flags.push('Verknüpfung');
  if (n.flags & 512) flags.push('anderes Laufwerk eingehängt');
  if (n.flags & 16) flags.push('Hardlink');
  if (n.flags & 32) flags.push('nur online');
  if (n.flags & 256) flags.push('komprimiert');
  const catTotal = (n.cats || []).reduce((a, b) => a + b, 0);
  fill(sideEl, 
    h('div', { class: 'row' }, h('span', { class: 'label' }, n.dir ? 'Ordner' : 'Datei'),
      h('button', { class: 'btn ghost small icon-only', style: { marginLeft: 'auto' }, 'aria-label': 'Details schließen', onclick: closeSide }, icon('x'))),
    h('div', null, h('div', { class: 'title' }, n.name), h('div', { class: 'path', style: { marginTop: '4px' } }, n.path)),
    h('div', null, h('div', { class: 'big num' }, fmtBytes(n.alloc)),
      h('div', { class: 'muted' }, 'belegt auf dem Datenträger')),
    n.guard.level === 'block' ? h('div', { class: 'notice block' }, icon('lock'), h('div', null, n.guard.reason))
      : n.guard.level === 'warn' ? h('div', { class: 'notice warn' }, icon('warn'), h('div', null, n.guard.reason)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary wide', disabled: blocked || id === 0, onclick: () => App.addToCart([id]) }, icon(blocked ? 'lock' : 'plus'), blocked ? 'Geschützt' : 'Zur Aufräumliste'),
      h('button', { class: 'btn', onclick: () => App.reveal(id) }, icon('explorer'), App.isWindows() ? 'Explorer' : 'Dateimanager'),
      h('button', { class: 'btn', onclick: () => App.open(id) }, icon('open'), 'Öffnen'),
      h('button', { class: 'btn', onclick: () => App.go('map', n.dir ? id : parent?.id ?? 0) }, icon('map'), 'Karte'),
      h('button', { class: 'btn', onclick: () => App.go('tree', id) }, icon('tree'), 'Baum'),
      h('button', { class: 'btn wide', onclick: () => App.copyPath(id, n.path) }, icon('copy'), 'Pfad kopieren')),
    h('dl', { class: 'kv' },
      h('dt', null, 'Größe'), h('dd', null, fmtBytes(n.size)),
      n.dir ? [h('dt', null, 'Dateien'), h('dd', null, fmtNum(n.files)), h('dt', null, 'Unterordner'), h('dd', null, fmtNum(n.dirs))] : [h('dt', null, 'Typ'), h('dd', null, `${CAT_NAMES[n.cat]}${n.ext ? ` (.${n.ext})` : ''}`)],
      h('dt', null, n.dir ? 'Letzte Änderung' : 'Geändert'), h('dd', { title: fmtAgo(n.dir ? n.newest : n.mtime) }, fmtDate(n.dir ? n.newest : n.mtime)),
      h('dt', null, 'Geöffnet'), h('dd', null, fmtDate(n.atime)),
      h('dt', null, 'Erstellt'), h('dd', null, fmtDate(n.ctime)),
      flags.length ? [h('dt', null, 'Merkmale'), h('dd', null, flags.join(', '))] : null),
    n.dir && catTotal > 0 ? h('div', null, h('div', { class: 'label', style: { marginBottom: '6px' } }, 'Inhalt'),
      h('div', { class: 'segbar', style: { height: '8px' } }, n.cats.map((v, i) => (v > 0 ? h('div', { class: 'part', style: { flex: `${v} 1 0`, background: catVar(i), padding: 0 }, title: `${CAT_NAMES[i]}: ${fmtBytes(v)}` }) : null)))) : null,
    n.heavy?.length ? h('div', null, h('div', { class: 'label', style: { marginBottom: '4px' } }, 'Hauptverursacher'),
      h('div', { class: 'chain' }, n.heavy.map((c, i) => h('button', { style: { paddingLeft: `${3 + i * 7}px` }, onclick: () => App.select(c.id) }, h('span', { class: 'ellipsis' }, c.name), h('span', { class: 'pct' }, fmtBytes(c.alloc)))))) : null,
    n.top?.length ? h('div', null, h('div', { class: 'label', style: { marginBottom: '4px' } }, 'Größte Einträge'),
      h('div', { class: 'chain' }, n.top.map((c) => h('button', { onclick: () => App.select(c.id), oncontextmenu: (e) => { e.preventDefault(); api('node?' + qs({ id: c.id })).then((r) => rowMenu(r, e.clientX, e.clientY)).catch(fail); } },
        icon(c.dir ? 'folder' : 'file'), h('span', { class: 'ellipsis' }, c.name), h('span', { class: 'pct' }, fmtBytes(c.alloc)))))) : null,
  );
};

/* ---------- Aufraeumliste ---------- */
async function openCart() {
  const c = await api('cart?' + qs({ limit: 1000 })).catch(fail);
  if (!c) return;
  const isNet = S.status?.index?.kind === 'network';
  const isDemo = S.status?.mode === 'demo';
  const list = h('tbody', null, c.rows.map((r) => {
    const tr = fileRow(r, { cols: ['alloc', 'mtime'] });
    const last = tr.lastElementChild;
    fill(last, h('button', {
      class: 'btn ghost small icon-only', 'aria-label': `${r.name} entfernen`, title: 'Von der Liste nehmen',
      onclick: async (e) => { e.stopPropagation(); await api('cart/remove', { ids: [r.id] }).catch(fail); tr.remove(); poll(); },
    }, icon('x')));
    return tr;
  }));
  const d = dialog({
    title: 'Aufräumliste prüfen',
    wide: true,
    body: [
      h('div', { class: 'stats' },
        h('div', { class: 'stat' }, h('div', { class: 'label' }, 'Gesamt'), h('div', { class: 'v' }, fmtBytes(c.bytes))),
        h('div', { class: 'stat' }, h('div', { class: 'label' }, 'Einträge'), h('div', { class: 'v' }, fmtNum(c.count)))),
      c.warnings ? h('div', { class: 'notice warn' }, icon('warn'), h('div', null, `${fmtNum(c.warnings)} ${c.warnings === 1 ? 'Eintrag liegt' : 'Einträge liegen'} in Programm- oder Einstellungsordnern (mit „Vorsicht“ markiert). Bitte prüfen.`)) : null,
      isDemo ? h('div', { class: 'notice info' }, icon('info'), h('div', null, 'Demo: Es wird nichts wirklich gelöscht, nur die Anzeige ändert sich.')) : null,
      h('div', { style: { overflow: 'auto', maxHeight: '42vh' } }, h('table', { class: 'tbl' }, list)),
      c.count > c.rows.length ? h('div', { class: 'muted' }, `Angezeigt werden die ${fmtNum(c.rows.length)} größten. Beim Aufräumen zählen alle ${fmtNum(c.count)}.`) : null,
    ],
    actions: [
      { label: 'Liste leeren', kind: 'ghost', run: () => api('cart/clear', {}).then(() => poll()).catch(fail) },
      h('a', { class: 'btn ghost', href: '#', onclick: (e) => { e.preventDefault(); location.href = '/api/export?' + qs({ what: 'cart', t: authToken() }); } }, icon('export'), 'CSV'),
      h('span', { class: 'grow' }),
      { label: 'Endgültig löschen…', kind: 'ghost', icon: 'x', run: () => { confirmPermanent(c); return false; } },
      { label: 'Verschieben…', icon: 'move', run: () => { App.pickFolder('Wohin verschieben?', (dest) => startCleanup({ mode: 'move', dest })); return false; } },
      { label: 'In den Papierkorb', kind: 'primary', icon: 'trash', disabled: isNet, run: () => { startCleanup({ mode: 'recycle' }); return false; } },
    ],
  });
  if (isNet) d.bodyEl.prepend(h('div', { class: 'notice warn' }, icon('warn'), h('div', null, 'Netzlaufwerke haben keinen Papierkorb. Hier kannst du nur verschieben oder endgültig löschen.')));
}

function confirmPermanent(c) {
  const input = h('input', { class: 'field', placeholder: 'LÖSCHEN', 'aria-label': 'Bestätigungswort', style: { width: '100%' }, autocomplete: 'off', oninput: () => { document.getElementById('perm-go').disabled = input.value !== 'LÖSCHEN'; } });
  dialog({
    title: 'Endgültig löschen?',
    body: [
      h('div', { class: 'notice block' }, icon('warn'), h('div', null, `${fmtNum(c.count)} ${c.count === 1 ? 'Eintrag' : 'Einträge'} mit ${fmtBytes(c.bytes)} ${c.count === 1 ? 'wird' : 'werden'} sofort entfernt und ${c.count === 1 ? 'landet' : 'landen'} nicht im Papierkorb. Das lässt sich nicht rückgängig machen.`)),
      h('label', null, 'Tippe zur Bestätigung ', h('b', null, 'LÖSCHEN'), ':'),
      input,
    ],
    actions: [
      { label: 'Abbrechen' },
      { label: 'Endgültig löschen', kind: 'danger', icon: 'trash', id: 'perm-go', disabled: true, run: () => { startCleanup({ mode: 'permanent', confirm: input.value }); return false; } },
    ],
  });
}

let cleanupDlg = null;
async function startCleanup(body) {
  document.querySelectorAll('.scrim').forEach((s) => s.remove());
  try { await api('cleanup/start', body); } catch (e) { fail(e); return; }
  const fill = h('i', { style: { width: '0%' } });
  const txt = h('div', { class: 'muted num' }, 'Wird vorbereitet…');
  cleanupDlg = { dlg: dialog({
    title: body.mode === 'move' ? 'Wird verschoben…' : 'Wird aufgeräumt…',
    body: [h('div', { class: 'pbar' }, fill), txt, body.mode === 'recycle' && App.isWindows() ? h('p', { class: 'muted' }, 'Falls etwas nicht in den Papierkorb passt, fragt Windows nach. Das Fenster kann hinter Storage Map liegen.') : null],
    actions: [],
  }), fill, txt, mode: body.mode };
  poll();
}

function handleCleanup(st) {
  const c = st.cleanup;
  if (cleanupDlg && c.running) {
    cleanupDlg.fill.style.width = `${(c.done / Math.max(1, c.total)) * 100}%`;
    cleanupDlg.txt.textContent = `${fmtNum(c.done)} von ${fmtNum(c.total)}`;
  }
  if (!c.running && c.last) {
    if (cleanupDlg) { cleanupDlg.dlg.close(); cleanupDlg = null; }
    const last = c.last;
    api('cleanup/ack', {}).catch(() => {});
    const mode = last.mode;
    const verb = mode === 'recycle' ? 'in den Papierkorb verschoben' : mode === 'move' ? 'verschoben' : 'gelöscht';
    dialog({
      title: last.failed ? 'Fast alles erledigt' : 'Erledigt',
      wide: last.failed > 0,
      body: [
        h('div', { class: 'stats' },
          h('div', { class: 'stat' }, h('div', { class: 'label' }, verb), h('div', { class: 'v' }, fmtBytes(last.freed)), h('div', { class: 's' }, `${fmtNum(last.ok)} ${last.ok === 1 ? 'Eintrag' : 'Einträge'}`)),
          last.failed ? h('div', { class: 'stat' }, h('div', { class: 'label' }, 'Nicht geschafft'), h('div', { class: 'v' }, fmtNum(last.failed))) : null),
        mode === 'recycle' ? h('div', { class: 'notice info' }, icon('info'), h('div', null, 'Der Platz wird erst frei, wenn du den Papierkorb leerst. Bis dahin kannst du alles wiederherstellen.')) : null,
        last.failed ? h('div', { style: { maxHeight: '40vh', overflow: 'auto' } }, h('table', { class: 'tbl' }, h('tbody', null, last.failures.map((f) => h('tr', null, h('td', null, h('div', { class: 'pt mono ellipsis', title: f.path }, f.path), h('div', { class: 'muted' }, f.reason))))))) : null,
      ],
      actions: [
        mode === 'recycle' && last.ok ? { label: 'Papierkorb leeren…', icon: 'trash', run: () => { App.emptyBin(last.freed); return false; } } : null,
        { label: 'Fertig', kind: 'primary' },
      ].filter(Boolean),
    });
  }
}

/* ---------- Tastatur ---------- */
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); searchEl.focus(); searchEl.select(); return; }
  if (e.target.closest?.('input, select, textarea') || e.ctrlKey || e.altKey || e.metaKey) return;
  if (document.querySelector('.scrim')) return;
  const keys = NAV_ITEMS.map((n) => n[0]);
  const n = Number(e.key);
  if (n >= 1 && n <= 9 && S.status?.index) App.go(keys[n - 1]);
  if (e.key === 'Delete' && S.selId) App.addToCart([S.selId]);
});

/* ---------- Start ---------- */
(async () => {
  await initToken();
  await poll();
  if (!S.status) {
    mainEl.append(h('div', { class: 'panel empty' }, 'Keine Verbindung zu Storage Map. Bitte das Programm neu starten.'));
  }
  loop();
})();

