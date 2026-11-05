// Startseite, Scan-Fortschritt, Uebersicht, Karte, Ringe, Ordnerbaum.

import {
  api, qs, h, icon, clear, fill, App, fmtBytes, fmtNum, fmtPct, fmtDate, fmtAgo, fmtDuration,
  catVar, CAT_NAMES, riskBadge, rowMenu, fail, toast, withTip, tipBlock,
} from './util.js';
import { segBar, catParts, rankBars, columns, ageColor, sparkline, severity, stateBadge } from './charts.js';
import { TreeMap, ageLegend } from './treemap.js';
import { Sunburst } from './sunburst.js';

const MEDIA = { ssd: 'SSD', hdd: 'HDD', net: 'Netzlaufwerk', unknown: 'Datenträger' };

/* ---------- Startseite ---------- */
export async function viewHome(root, S) {
  const list = h('div');
  const isTest = S.status?.mode !== 'normal';
  root.append(h('div', { class: 'home' },
    h('h1', null, 'Welches Laufwerk soll kartiert werden?'),
    h('p', { class: 'lead' }, 'Storage Map liest Dateinamen, Größen und Datumsangaben und zeigt, was wie viel Platz belegt. Entfernt wird nur, was du selbst bestätigst.'),
    list,
    h('div', { class: 'home-foot' },
      h('button', { class: 'btn', onclick: () => App.pickFolder('Ordner scannen', (p) => App.startScan(p)) }, icon('folder'), 'Einzelnen Ordner scannen…'),
      h('button', { class: 'btn ghost', onclick: () => App.go('mcp') }, icon('plug'), 'KI-Zugriff'),
      !S.status?.elevated && S.status?.canElevate ? h('button', { class: 'btn ghost', onclick: () => App.elevate() }, icon('shield'), 'Mit Adminrechten neu starten') : null,
      !S.status?.elevated && !isTest ? h('span', { class: 'muted' }, S.status?.canElevate ? 'Mit Adminrechten lassen sich auch gesperrte Systemordner lesen.' : 'Gesperrte Systemordner lassen sich nur lesen, wenn du Storage Map mit sudo startest.') : null,
    )));
  try {
    const d = await api('drives');
    clear(list);
    d.drives.forEach((v) => list.append(driveCard(v)));
    if (d.folders.length) {
      list.append(h('div', { class: 'label', style: { margin: '18px 0 8px' } }, 'Zuletzt gescannte Ordner'));
      d.folders.forEach((f) => list.append(h('div', { class: 'panel row', style: { marginBottom: '6px' } },
        icon('folder'),
        h('span', { class: 'grow mono ellipsis', title: f.root }, f.root),
        h('span', { class: 'muted' }, fmtAgo(f.ts)),
        h('button', { class: 'btn small', onclick: () => App.openCached(f.root) }, 'Öffnen'),
        h('button', { class: 'btn small', onclick: () => App.startScan(f.root) }, 'Neu scannen'))));
    }
  } catch (e) { fail(e); }
}

function driveCard(d) {
  const used = d.total - d.free;
  const share = d.total ? used / d.total : 0;
  const sev = severity(share);
  const isFolder = d.fs === 'TEST';
  const windowsDrive = /^[A-Za-z]:\\$/.test(d.root);
  const canScan = d.ready || isFolder;
  return h('div', { class: 'panel drive' },
    h('div', { class: 'letter' }, isFolder ? icon('folder') : windowsDrive ? d.root.slice(0, 2) : icon('drive')),
    h('div', { style: { minWidth: 0 } },
      h('div', { class: 'row' },
        h('b', { class: 'ellipsis' }, windowsDrive ? d.label || (d.kind === 'removable' ? 'Wechseldatenträger' : 'Lokaler Datenträger') : d.root),
        !windowsDrive && d.label && !isFolder ? h('span', { class: 'soft ellipsis' }, d.label) : null,
        h('span', { class: 'tag' }, MEDIA[d.media] || d.kind),
        d.fs ? h('span', { class: 'muted' }, d.fs) : null,
        d.lastScan ? h('span', { class: 'muted', style: { marginLeft: 'auto' } }, `Letzter Scan ${fmtAgo(d.lastScan)}`) : null),
      isFolder ? h('div', { class: 'mono muted ellipsis', title: d.root }, d.root) : d.ready ? [
        h('div', { class: 'bar' }, h('i', { style: { width: `${share * 100}%`, background: sev.color } })),
        h('div', { class: 'row muted num' }, `${fmtBytes(d.free)} frei von ${fmtBytes(d.total)}`, h('span', { style: { marginLeft: 'auto' } }, stateBadge(share))),
      ] : h('div', { class: 'muted' }, 'Nicht bereit')),
    h('div', { class: 'acts' },
      d.lastScan ? h('button', { class: 'btn', onclick: () => App.openCached(d.root) }, 'Letzten Scan öffnen') : null,
      h('button', { class: 'btn ' + (d.lastScan ? '' : 'primary'), disabled: !canScan, onclick: () => App.startScan(d.root) }, icon('refresh'), d.lastScan ? 'Neu scannen' : 'Scannen')));
}

/* ---------- Scan-Fortschritt ---------- */
export function viewScanning(root) {
  const num = h('div', { class: 'hero-num num' }, '0 B');
  const sub = h('div', { class: 'hero-sub' }, 'Wird gestartet…');
  const pfill = h('i', { style: { width: '0%' } });
  const pbar = h('div', { class: 'pbar' }, pfill);
  const pct = h('div', { class: 'muted num' });
  const cur = h('div', { class: 'current', style: { marginTop: '10px' } });
  const stats = h('div', { class: 'stats', style: { marginTop: '18px' } });
  const live = h('div');
  root.append(h('div', { class: 'scanning' },
    h('div', { class: 'panel' },
      h('div', { class: 'label', id: 'scan-title' }, 'Scan läuft'),
      h('div', { style: { marginTop: '8px' } }, num, sub),
      pbar, pct, cur, stats,
      h('div', { class: 'row', style: { marginTop: '16px' } },
        h('button', { class: 'btn', onclick: () => api('scan/cancel', {}).catch(fail) }, icon('x'), 'Abbrechen'),
        h('span', { class: 'muted' }, 'Es wird nur gelesen, nichts verändert.'))),
    h('div', { class: 'panel' }, h('h2', null, 'Wächst gerade'), live)));
  return (p) => {
    if (!p) return;
    num.textContent = fmtBytes(p.bytes);
    const frac = p.expected ? Math.min(0.99, p.bytes / p.expected) : null;
    const speed = p.bytes / Math.max(1, p.elapsedMs / 1000);
    const left = frac && frac > 0.02 ? (p.elapsedMs / frac) * (1 - frac) : null;
    const phase = { scan: 'Dateien werden gelesen', analyze: 'Wird ausgewertet', save: 'Wird gespeichert' }[p.phase] || '';
    sub.textContent = `${phase} · ${p.root}`;
    if (frac !== null) { pbar.classList.remove('indet'); pfill.style.width = `${frac * 100}%`; pct.textContent = `${fmtPct(frac)} des belegten Platzes${left ? ` · noch etwa ${fmtDuration(left)}` : ''}`; }
    else { pbar.classList.add('indet'); pct.textContent = ''; }
    cur.textContent = '‎' + (p.current || '');
    fill(stats, 
      statTile(fmtNum(p.files), 'Dateien'),
      statTile(fmtNum(p.dirs), 'Ordner'),
      statTile(fmtDuration(p.elapsedMs), 'Laufzeit'),
      statTile(`${fmtBytes(speed)}/s`, `${p.threads} Threads · ${MEDIA[p.media] || ''}`),
      p.errors ? statTile(fmtNum(p.errors), 'nicht lesbar') : null);
    const top = Math.max(...p.tops.map((t) => t.bytes), 1);
    fill(live, ...p.tops.map((t) => h('div', { class: 'live-row' },
      h('div', { class: 'ellipsis' }, t.name),
      h('div', { class: 'num soft' }, fmtBytes(t.bytes)),
      h('div', { class: 'bar', style: { width: `${(t.bytes / top) * 100}%` } }))));
  };
}

function statTile(v, label, sub, onClick) {
  const el = h('div', { class: 'stat' + (onClick ? ' clickable' : ''), onclick: onClick, tabindex: onClick ? 0 : undefined },
    h('div', { class: 'label' }, label),
    h('div', { class: 'v' }, v),
    sub ? h('div', { class: 's' }, sub) : null);
  return el;
}

/* ---------- Uebersicht ---------- */
export async function viewOverview(root, S) {
  const s = await api('summary');
  S.summary = s;
  const vol = s.volume;
  const isDrive = s.isDrive && vol.total > 0;
  const used = vol.total - (S.status?.index?.free ?? vol.free);
  const free = S.status?.index?.free ?? vol.free;
  const share = isDrive ? used / vol.total : 0;
  const sev = severity(share);
  const go = App.go;

  const head = isDrive
    ? h('div', null,
      h('div', { class: 'label' }, `${s.root} · ${vol.label || 'Laufwerk'} · ${MEDIA[vol.media] || ''} · ${vol.fs}`),
      h('div', { class: 'row', style: { alignItems: 'flex-end', gap: '16px', marginTop: '8px' } },
        h('div', { class: 'hero-num' }, fmtBytes(free)),
        h('div', { id: 'free-spark' })),
      h('div', { class: 'hero-sub' }, `frei von ${fmtBytes(vol.total)}, ${fmtPct(share)} belegt`),
      h('div', { class: 'meter', role: 'meter', 'aria-valuenow': Math.round(share * 100), 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-label': 'Belegung' },
        h('i', { style: { width: `${share * 100}%`, background: sev.color } })),
      h('div', { class: 'row', style: { marginTop: '6px' } }, stateBadge(share),
        h('span', { class: 'muted', style: { marginLeft: 'auto' } }, `Scan ${fmtAgo(s.scannedAt)}, dauerte ${fmtDuration(s.durationMs)}`)))
    : h('div', null,
      h('div', { class: 'label' }, 'Ordner'),
      h('div', { class: 'mono soft', style: { wordBreak: 'break-all', margin: '6px 0' } }, s.root),
      h('div', { class: 'hero-num' }, fmtBytes(s.scanned)),
      h('div', { class: 'hero-sub' }, `belegt, in ${fmtNum(s.files)} Dateien`),
      h('div', { class: 'muted', style: { marginTop: '6px' } }, `Scan ${fmtAgo(s.scannedAt)}, dauerte ${fmtDuration(s.durationMs)}`));

  const stats = h('div', { class: 'stats' },
    statTile(fmtBytes(s.safeBytes), 'Schnell freimachbar', 'ohne Bedenken', () => go('clean')),
    statTile(s.dupWasted === null ? 'Prüfen' : fmtBytes(s.dupWasted), 'Doppelt', s.dupWasted === null ? 'noch nicht analysiert' : 'unnötige Kopien', () => go('dups')),
    statTile(fmtBytes(s.stale2y), 'Alt', 'über 2 Jahre unverändert', () => go('old')),
    statTile(fmtNum(s.files), 'Dateien', `${fmtNum(s.dirs)} Ordner`, () => go('files')),
    isDrive ? statTile(fmtBytes(s.unaccounted), 'Nicht erfasst', s.denied ? `${fmtNum(s.denied)} Ordner nicht lesbar` : 'System und Verwaltung', () => App.showDenied()) : null,
    s.hardlinks ? statTile(fmtNum(s.hardlinks), 'Hardlinks', 'nur einmal gezählt') : null,
    s.cloud.count ? statTile(fmtBytes(s.cloud.bytes), 'Nur online', `${fmtNum(s.cloud.count)} Cloud-Dateien, nicht auf der Platte`) : null,
  );
  const health = h('div', { id: 'health' });

  root.append(h('div', { class: 'grid' },
    h('div', { class: 'panel span-12 hero' }, head, h('div', { class: 'stack' }, stats, health)),

    h('div', { class: 'panel span-7' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, 'Speicherkarte'),
        h('button', { class: 'link more', onclick: () => go('map') }, 'Öffnen')),
      (() => { const m = h('div', { class: 'minimap', id: 'minimap', tabindex: 0, role: 'button', 'aria-label': 'Speicherkarte öffnen', onclick: () => go('map'), onkeydown: (e) => { if (e.key === 'Enter') go('map'); } }); return m; })(),
      h('div', { style: { marginTop: '10px' } }, segBar(catParts(s.cats), { onClick: (p) => go('files', null, { cat: p.id }) }))),

    h('div', { class: 'panel span-5' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, 'Was bringt am meisten?'),
        h('button', { class: 'link more', onclick: () => go('clean') }, 'Alles ansehen')),
      s.recs.length ? h('div', null, s.recs.slice(0, 8).map((r) => h('button', {
        class: 'rec', onclick: () => {
          if (r.kind === 'dups') go('dups');
          else if (r.kind === 'stale') go('old');
          else go('clean', null, { rule: r.rule });
        },
      }, riskBadge(r.risk), h('div', { style: { minWidth: 0 } }, h('div', { class: 'ellipsis', style: { fontWeight: 500 } }, r.title), h('div', { class: 'muted ellipsis', style: { fontSize: '12px' }, title: r.why }, r.why)), h('div', { class: 'val' }, fmtBytes(r.bytes))))) : h('div', { class: 'empty' }, 'Keine größeren Aufräum-Kandidaten gefunden.')),

    h('div', { class: 'panel span-6' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, 'Wo der Platz steckt'),
        h('button', { class: 'link more', onclick: () => go('tree') }, 'Ordner')),
      rankBars(s.hotspots.slice(0, 10), {
        name: (r) => r.name, path: (r) => r.path, value: (r) => r.alloc, color: (r) => catVar(r.cat), total: s.scanned,
        onClick: (r) => App.select(r.id), onContext: (r, e) => rowMenu(r, e.clientX, e.clientY),
      })),

    h('div', { class: 'panel span-6' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, 'Größte Dateien'),
        h('button', { class: 'link more', onclick: () => go('files') }, 'Alle')),
      rankBars(s.topFiles.slice(0, 10), {
        name: (r) => r.name, path: (r) => r.path, value: (r) => r.alloc, color: (r) => catVar(r.cat), total: s.scanned,
        onClick: (r) => App.select(r.id), onContext: (r, e) => rowMenu(r, e.clientX, e.clientY),
      })),

    h('div', { class: 'panel span-4' }, h('h2', null, 'Alter der Daten'),
      columns(s.ages, { color: (r, i) => ageColor(i, s.ages.length), label: (r) => r.label.replace(' Jahre', ' J').replace(' Monate', ' M').replace(' Monat', ' M').replace(' Woche', ' W').replace(' Jahr', ' J').replace('< ', '<').replace('> ', '>').replace(' ', ''), sub: (r) => `${fmtNum(r.files)} Dateien`, onClick: () => go('old') })),
    h('div', { class: 'panel span-4' }, h('h2', null, 'Dateigrößen'),
      columns(s.sizes, { color: (r, i) => ageColor(i, s.sizes.length), label: (r) => r.label.replaceAll(' MB', 'M').replaceAll(' GB', 'G').replace('< ', '<').replace('> ', '>'), sub: (r) => `${fmtNum(r.files)} Dateien` })),
    h('div', { class: 'panel span-4' }, h('h2', null, 'Dateitypen'),
      rankBars(s.exts.slice(0, 7), {
        name: (r) => (r.ext ? `.${r.ext}` : '(ohne Endung)'), value: (r) => r.alloc, total: s.scanned, color: () => 'var(--c0)',
        onClick: (r) => go('files', null, { ext: r.ext || '(ohne)' }),
      })),
    s.years.length > 1 ? h('div', { class: 'panel span-12' }, h('h2', null, 'Datenmenge nach Jahr der letzten Änderung'),
      columns(s.years, { value: (r) => r.alloc, label: (r) => String(r.year), height: 150, sub: (r) => `${fmtNum(r.files)} Dateien`, color: () => 'var(--c0)' })) : null,
  ));

  const mini = new TreeMap(root.querySelector('#minimap'), { mini: true });
  api('map?' + qs({ id: 0, limit: 900, minfrac: 0.0006, maxdepth: 5 })).then((d) => mini.setData(d)).catch(() => {});

  // Laufwerkszustand und Verlauf kommen verzoegert, damit die Seite sofort steht.
  if (isDrive) loadHealth(root, s.root);
  api('history').then((hist) => {
    const vals = hist.entries.map((e) => e.total - e.free);
    const sp = sparkline(vals, { width: 130, height: 40 });
    const el = root.querySelector('#free-spark');
    if (sp && el) { withTip(el, () => tipBlock(null, 'Belegter Platz', `über ${vals.length} Scans`)); el.append(sp); }
  }).catch(() => {});
  return () => mini.destroy();
}

async function loadHealth(root, drive) {
  const el = root.querySelector('#health');
  for (let i = 0; i < 12 && el.isConnected; i++) {
    const v = await api('health?' + qs({ root: drive })).catch(() => null);
    if (!v || v.unavailable) return;
    if (!v.pending) {
      const good = v.health === 'Healthy';
      const unknown = v.health === 'Unknown';
      const parts = [unknown ? null : v.model, v.bus, v.temp ? `${v.temp} °C` : null, v.wear !== null && v.wear !== undefined ? `Verschleiß ${v.wear} %` : null, v.hours ? `${fmtNum(v.hours)} Betriebsstunden` : null].filter(Boolean);
      fill(el, h('div', { class: 'notice ' + (unknown ? 'info' : good ? 'ok' : 'warn') }, icon(unknown ? 'info' : good ? 'check' : 'warn'),
        h('div', null, h('b', null, unknown ? v.model : good ? 'Datenträger meldet: fehlerfrei' : `Datenträger meldet: ${v.health}`), h('div', { class: 'muted' }, parts.join(' · ')))));
      return;
    }
    await new Promise((r) => setTimeout(r, 1500));
  }
}

/* ---------- Karte ---------- */
const DETAIL = { grob: { limit: 1500, minfrac: 0.0004 }, mittel: { limit: 6000, minfrac: 0.00004 }, fein: { limit: 18000, minfrac: 0.000012 } };

function crumbs(data, onPick) {
  const bits = [];
  data.ancestors.forEach((a, i) => {
    if (i) bits.push(h('span', { class: 'slash' }, '›'));
    bits.push(h('button', { onclick: () => onPick(a.id) }, a.name));
  });
  return bits;
}

function legendFor(tm, mode, cats) {
  if (mode === 'age') return [ageLegend(), h('span', { class: 'muted' }, 'Zeigt, wie lange die Dateien unverändert sind')];
  return [...cats.filter((c) => c.alloc > 0).sort((a, b) => b.alloc - a.alloc).map((c) => h('button', {
    onpointerenter: () => tm.setDim(c.id), onpointerleave: () => tm.setDim(null),
    onfocus: () => tm.setDim(c.id), onblur: () => tm.setDim(null),
    onclick: () => App.go('files', null, { cat: c.id }),
  }, h('i', { class: 'swatch', style: { background: catVar(c.id) } }), c.name))];
}

export async function viewMap(root, S, id) {
  const st = S.mapState;
  const crumb = h('div', { class: 'crumbs' });
  const stage = h('div', { class: 'mapstage' });
  const legend = h('div', { class: 'maplegend' });
  const modeSeg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Färbung' });
  const detSeg = h('div', { class: 'seg', role: 'group', 'aria-label': 'Detailgrad' });
  const upBtn = h('button', { class: 'btn', title: 'Eine Ebene höher (Backspace)', onclick: () => goUp() }, icon('up'), 'Hoch');
  const freeBtn = h('button', { class: 'chipbtn' + (st.showFree ? ' on' : ''), onclick: () => { st.showFree = !st.showFree; freeBtn.classList.toggle('on', st.showFree); load(cur); } }, 'Freien Platz zeigen');
  root.append(
    h('div', { class: 'mapbar' }, upBtn, crumb,
      h('span', { class: 'label' }, 'Farbe'), modeSeg,
      h('span', { class: 'label' }, 'Detail'), detSeg, freeBtn),
    stage, legend);

  let cur = id ?? 0;
  let ancestors = [];
  const tm = new TreeMap(stage, {
    onSelect: (n) => App.select(n.id),
    onZoom: (n) => App.go('map', n.id),
    onUp: () => goUp(),
    onContext: (n, e) => api('node?' + qs({ id: n.id })).then((r) => rowMenu(r, e.clientX, e.clientY)).catch(fail),
  });
  tm.mode = st.mode;

  const goUp = () => { if (ancestors.length > 1) { tm.focus = { x: tm.W * .25, y: tm.H * .25, w: tm.W * .5, h: tm.H * .5 }; App.go('map', ancestors.at(-2).id); } };
  const draw = (seg, opts, val, pick) => fill(seg, ...opts.map(([k, l]) => h('button', { class: k === val ? 'on' : '', onclick: () => pick(k) }, l)));
  const drawMode = () => draw(modeSeg, [['type', 'Dateityp'], ['age', 'Alter']], st.mode, (m) => { st.mode = m; tm.setMode(m); drawMode(); fill(legend, ...legendFor(tm, m, S.summary.cats)); });
  const drawDet = () => draw(detSeg, [['grob', 'Grob'], ['mittel', 'Mittel'], ['fein', 'Fein']], st.detail, (d) => { st.detail = d; drawDet(); load(cur); });
  drawMode(); drawDet();

  async function load(nid, anim) {
    const d = DETAIL[st.detail];
    const data = await api('map?' + qs({ id: nid, ...d }));
    ancestors = data.ancestors;
    cur = nid;
    fill(crumb, ...crumbs(data, (a) => { if (a !== cur) { tm.focus = null; App.go('map', a); } }));
    upBtn.disabled = ancestors.length <= 1;
    tm.setData(data, { anim, showFree: st.showFree });
  }
  S.summary = S.summary || await api('summary');
  fill(legend, ...legendFor(tm, st.mode, S.summary.cats));
  await load(cur);

  const key = (e) => {
    if (e.target.closest?.('input, select, textarea')) return;
    if (e.key === 'Backspace' || e.key === 'Escape') { e.preventDefault(); goUp(); }
  };
  document.addEventListener('keydown', key);
  return {
    destroy: () => { tm.destroy(); document.removeEventListener('keydown', key); },
    /// Beim Wechsel innerhalb der Karte wird nichts neu aufgebaut, nur gezoomt.
    update: (nid) => load(nid, ancestors.some((a) => a.id === nid) ? 'out' : 'in').catch(fail),
    refresh: () => load(cur).catch(fail),
  };
}

/* ---------- Ringe ---------- */
export async function viewRings(root, S, id) {
  const crumb = h('div', { class: 'crumbs' });
  const stage = h('div', { class: 'mapstage', style: { background: 'transparent' } });
  const legend = h('div', { class: 'maplegend' });
  const upBtn = h('button', { class: 'btn', onclick: () => goUp() }, icon('up'), 'Hoch');
  root.append(h('div', { class: 'mapbar' }, upBtn, crumb, h('span', { class: 'muted' }, 'Klick auf einen Ring: hineinzoomen. Klick in die Mitte: zurück.')), stage, legend);
  let cur = id ?? 0, ancestors = [];
  const sb = new Sunburst(stage, {
    onZoom: (n) => App.go('rings', n.id),
    onUp: () => goUp(),
    onSelect: (n) => App.select(n.id),
    onContext: (n, e) => api('node?' + qs({ id: n.id })).then((r) => rowMenu(r, e.clientX, e.clientY)).catch(fail),
  });
  const goUp = () => { if (ancestors.length > 1) App.go('rings', ancestors.at(-2).id); };
  async function load(nid) {
    const data = await api('map?' + qs({ id: nid, limit: 5000, minfrac: 0.0005, maxdepth: 6 }));
    ancestors = data.ancestors;
    cur = nid;
    fill(crumb, ...crumbs(data, (a) => App.go('rings', a)));
    upBtn.disabled = ancestors.length <= 1;
    sb.setData(data);
  }
  S.summary = S.summary || await api('summary');
  fill(legend, ...S.summary.cats.filter((c) => c.alloc > 0).sort((a, b) => b.alloc - a.alloc).map((c) => h('span', { class: 'row' }, h('i', { class: 'swatch', style: { background: catVar(c.id) } }), c.name)));
  await load(cur);
  return { destroy: () => sb.destroy(), update: (nid) => load(nid).catch(fail), refresh: () => load(cur).catch(fail) };
}

/* ---------- Ordnerbaum ---------- */
export async function viewTree(root, S, focusId) {
  const tbody = h('tbody');
  const sortSel = h('select', { class: 'field', 'aria-label': 'Sortierung', onchange: () => { S.treeSort = sortSel.value; rebuild(); } },
    h('option', { value: 'alloc' }, 'Nach Größe'), h('option', { value: 'name' }, 'Nach Name'), h('option', { value: 'mtime' }, 'Nach Änderung'), h('option', { value: 'files' }, 'Nach Dateianzahl'));
  sortSel.value = S.treeSort || 'alloc';
  const panel = h('div', { class: 'panel', style: { padding: '0 0 4px' } },
    h('table', { class: 'tbl' },
      h('thead', null, h('tr', null, h('th', null, 'Name'), h('th', { class: 'w-bar' }, 'Anteil'), h('th', { class: 'w-size right' }, 'Belegt'), h('th', { class: 'w-num right' }, 'Dateien'), h('th', { class: 'w-date right' }, 'Geändert'), h('th', { class: 'w-act' }))),
      tbody));
  root.append(
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, 'Ordner'), h('p', null, 'Klappe Ordner auf und sieh, welcher Unterordner wie viel belegt.')),
      h('div', { class: 'actions' }, sortSel, h('button', { class: 'btn', onclick: () => followHeavy() }, icon('search'), 'Größtem Pfad folgen'))),
    panel);

  const open = new Map(); // id -> Zeile
  const rowFor = (n, depth, parentAlloc) => {
    const sel = S.selId === n.id;
    const isDir = n.dir;
    const exp = h('button', {
      class: 'expander', style: { visibility: isDir && n.kids ? 'visible' : 'hidden' }, 'aria-label': 'Aufklappen', 'aria-expanded': 'false',
      onclick: (e) => { e.stopPropagation(); toggle(tr, n, depth); },
    }, icon('chevron'));
    const share = parentAlloc ? n.alloc / parentAlloc : 0;
    const tr = h('tr', {
      class: 'rowlink' + (sel ? ' sel' : ''), 'data-id': n.id,
      onclick: () => App.select(n.id),
      ondblclick: () => { if (isDir && n.kids) toggle(tr, n, depth); },
      oncontextmenu: (e) => { e.preventDefault(); api('node?' + qs({ id: n.id })).then((r) => rowMenu(r, e.clientX, e.clientY)).catch(fail); },
    },
      h('td', null, h('div', { class: 'cell-name', style: { paddingLeft: `${depth * 16}px` } }, exp,
        icon(isDir ? 'folder' : 'file'),
        h('span', { class: 'nm', title: n.name }, n.name),
        n.flags & 8 ? h('span', { class: 'tag', title: 'Verknüpfung auf einen anderen Ort. Wird nicht mitgezählt.' }, 'Link') : null,
        n.flags & 64 ? h('span', { class: 'tag', title: 'Dieser Ordner konnte nicht gelesen werden.' }, 'Kein Zugriff') : null,
        n.flags & 16 ? h('span', { class: 'tag', title: 'Weiterer Name einer Datei, die schon anderswo gezählt ist.' }, 'Hardlink') : null,
        n.flags & 32 ? h('span', { class: 'tag', title: 'Liegt nur in der Cloud, nicht auf der Platte.' }, 'Nur online') : null)),
      h('td', null, withTip(h('div', { class: 'tbar' }, h('i', { style: { width: `${share * 100}%`, background: catVar(n.cat) } })), () => tipBlock(fmtPct(share), 'des übergeordneten Ordners', CAT_NAMES[n.cat]))),
      h('td', { class: 'right num' }, fmtBytes(n.alloc)),
      h('td', { class: 'right num muted' }, isDir ? fmtNum(n.files) : ''),
      h('td', { class: 'right num muted', title: fmtDate(isDir ? n.newest : n.mtime) }, fmtDate(isDir ? n.newest : n.mtime)),
      h('td', null, h('button', {
        class: 'btn ghost small icon-only', 'aria-label': 'Aktionen',
        onclick: (e) => { e.stopPropagation(); api('node?' + qs({ id: n.id })).then((r) => rowMenu(r, e.clientX, e.clientY)).catch(fail); },
      }, icon('menu'))));
    tr._exp = exp;
    tr._node = n;
    tr._depth = depth;
    return tr;
  };

  async function fetchKids(id, parentAlloc) {
    const d = await api('children?' + qs({ id, sort: S.treeSort || 'alloc', dir: (S.treeSort === 'name') ? 'asc' : 'desc', limit: 1000 }));
    return { rows: d.rows, total: d.total, parentAlloc: d.parentAlloc || parentAlloc };
  }
  async function toggle(tr, n, depth) {
    if (open.has(n.id)) return collapse(tr, n);
    const d = await fetchKids(n.id, n.alloc);
    tr._exp.classList.add('open');
    tr._exp.setAttribute('aria-expanded', 'true');
    let after = tr;
    const added = [];
    for (const c of d.rows) { const r = rowFor(c, depth + 1, d.parentAlloc); after.after(r); after = r; added.push(r); }
    if (d.total > d.rows.length) {
      const more = h('tr', null, h('td', { colspan: 6, class: 'muted', style: { paddingLeft: `${(depth + 1) * 16 + 26}px` } }, `… und ${fmtNum(d.total - d.rows.length)} weitere. Alle stehen in „Große Dateien“.`));
      after.after(more); added.push(more);
    }
    open.set(n.id, added);
    return added;
  }
  function collapse(tr, n) {
    const rows = open.get(n.id) || [];
    for (const r of rows) {
      if (r._node && open.has(r._node.id)) collapse(r, r._node);
      r.remove();
    }
    open.delete(n.id);
    tr._exp.classList.remove('open');
    tr._exp.setAttribute('aria-expanded', 'false');
  }
  async function rebuild() {
    const d = await fetchKids(0);
    open.clear();
    clear(tbody);
    d.rows.forEach((c) => tbody.append(rowFor(c, 0, d.parentAlloc)));
  }
  async function revealPath(id) {
    const node = await api('node?' + qs({ id }));
    let last = null;
    for (const a of node.ancestors.slice(1)) {
      const tr = tbody.querySelector(`tr[data-id="${a.id}"]`);
      if (!tr) break;
      last = tr;
      if (a.id !== id && !open.has(a.id)) await toggle(tr, tr._node, tr._depth);
    }
    if (last) {
      tbody.querySelectorAll('tr.sel').forEach((t) => t.classList.remove('sel'));
      last.classList.add('sel');
      last.scrollIntoView({ block: 'center' });
    }
    App.select(id);
  }
  async function followHeavy() {
    const node = await api('node?' + qs({ id: S.selId && S.selDir ? S.selId : 0 }));
    const target = node.heavy.at(-1);
    if (!target) { toast('Kein einzelner Unterordner dominiert hier.'); return; }
    await revealPath(target.id);
    const tr = tbody.querySelector(`tr[data-id="${target.id}"]`);
    if (tr && tr._node.dir && !open.has(target.id)) await toggle(tr, tr._node, tr._depth);
  }
  await rebuild();
  if (focusId) await revealPath(focusId).catch(fail);
  return { destroy: () => {}, update: (nid) => revealPath(nid).catch(fail), refresh: () => rebuild().catch(fail) };
}

export { statTile };
