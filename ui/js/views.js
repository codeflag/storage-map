// Startseite, Scan-Fortschritt, Uebersicht, Karte, Ringe, Ordnerbaum.

import {
  api, qs, h, icon, clear, fill, App, fmtBytes, fmtNum, fmtPct, fmtDate, fmtAgo, fmtDuration,
  catVar, catName, riskBadge, rowMenu, fail, toast, withTip, tipBlock, withPct,
} from './util.js';
import { segBar, catParts, rankBars, columns, ageColor, sparkline, severity, stateBadge } from './charts.js';
import { TreeMap, ageLegend } from './treemap.js';
import { Sunburst } from './sunburst.js';
import { t, tn } from './i18n.js';

const mediaName = (m, other = '') => (['ssd', 'hdd', 'net', 'unknown'].includes(m) ? t(`media.${m}`) : other);

/* ---------- Startseite ---------- */
export async function viewHome(root, S) {
  const list = h('div');
  const isTest = S.status?.mode !== 'normal';
  root.append(h('div', { class: 'home' },
    h('h1', null, t('home.title')),
    h('p', { class: 'lead' }, t('home.lead')),
    list,
    h('div', { class: 'home-foot' },
      h('button', { class: 'btn', onclick: () => App.pickFolder(t('home.scanFolderTitle'), (p) => App.startScan(p)) }, icon('folder'), t('home.scanFolder')),
      h('button', { class: 'btn ghost', onclick: () => App.go('mcp') }, icon('plug'), t('nav.mcp')),
      !S.status?.elevated && S.status?.canElevate ? h('button', { class: 'btn ghost', onclick: () => App.elevate() }, icon('shield'), t('common.adminRestart')) : null,
      !S.status?.elevated && !isTest ? h('span', { class: 'muted' }, S.status?.canElevate ? t('home.adminHint') : t('home.sudoHint')) : null,
    )));
  try {
    const d = await api('drives');
    clear(list);
    d.drives.forEach((v) => list.append(driveCard(v)));
    if (d.folders.length) {
      list.append(h('div', { class: 'label', style: { margin: '18px 0 8px' } }, t('home.recent')));
      d.folders.forEach((f) => list.append(h('div', { class: 'panel row', style: { marginBottom: '6px' } },
        icon('folder'),
        h('span', { class: 'grow mono ellipsis', title: f.root }, f.root),
        h('span', { class: 'muted' }, fmtAgo(f.ts)),
        h('button', { class: 'btn small', onclick: () => App.openCached(f.root) }, t('common.open')),
        h('button', { class: 'btn small', onclick: () => App.startScan(f.root) }, t('top.rescan')))));
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
        h('b', { class: 'ellipsis' }, windowsDrive ? d.label || (d.kind === 'removable' ? t('drive.removable') : t('drive.local')) : d.root),
        !windowsDrive && d.label && !isFolder ? h('span', { class: 'soft ellipsis' }, d.label) : null,
        h('span', { class: 'tag' }, mediaName(d.media, d.kind)),
        d.fs ? h('span', { class: 'muted' }, d.fs) : null,
        d.lastScan ? h('span', { class: 'muted', style: { marginLeft: 'auto' } }, t('drive.lastScan', { ago: fmtAgo(d.lastScan) })) : null),
      isFolder ? h('div', { class: 'mono muted ellipsis', title: d.root }, d.root) : d.ready ? [
        h('div', { class: 'bar' }, h('i', { style: { width: `${share * 100}%`, background: sev.color } })),
        h('div', { class: 'row muted num' }, t('drive.freeOf', { free: fmtBytes(d.free), total: fmtBytes(d.total) }), h('span', { style: { marginLeft: 'auto' } }, stateBadge(share))),
      ] : h('div', { class: 'muted' }, t('drive.notReady'))),
    h('div', { class: 'acts' },
      d.lastScan ? h('button', { class: 'btn', onclick: () => App.openCached(d.root) }, t('drive.openLast')) : null,
      h('button', { class: 'btn ' + (d.lastScan ? '' : 'primary'), disabled: !canScan, onclick: () => App.startScan(d.root) }, icon('refresh'), d.lastScan ? t('top.rescan') : t('drive.scan'))));
}

/* ---------- Scan-Fortschritt ---------- */
export function viewScanning(root) {
  const num = h('div', { class: 'hero-num num' }, '0 B');
  const sub = h('div', { class: 'hero-sub' }, t('scan.starting'));
  const pfill = h('i', { style: { width: '0%' } });
  const pbar = h('div', { class: 'pbar' }, pfill);
  const pct = h('div', { class: 'muted num' });
  const cur = h('div', { class: 'current', style: { marginTop: '10px' } });
  const stats = h('div', { class: 'stats', style: { marginTop: '18px' } });
  const live = h('div');
  root.append(h('div', { class: 'scanning' },
    h('div', { class: 'panel' },
      h('div', { class: 'label', id: 'scan-title' }, t('scan.title')),
      h('div', { style: { marginTop: '8px' } }, num, sub),
      pbar, pct, cur, stats,
      h('div', { class: 'row', style: { marginTop: '16px' } },
        h('button', { class: 'btn', onclick: () => api('scan/cancel', {}).catch(fail) }, icon('x'), t('common.cancel')),
        h('span', { class: 'muted' }, t('scan.readOnly')))),
    h('div', { class: 'panel' }, h('h2', null, t('scan.growing')), live)));
  return (p) => {
    if (!p) return;
    num.textContent = fmtBytes(p.bytes);
    const frac = p.expected ? Math.min(0.99, p.bytes / p.expected) : null;
    const speed = p.bytes / Math.max(1, p.elapsedMs / 1000);
    const left = frac && frac > 0.02 ? (p.elapsedMs / frac) * (1 - frac) : null;
    const phase = { scan: t('scan.phaseScan'), analyze: t('scan.phaseAnalyze'), save: t('scan.phaseSave') }[p.phase] || '';
    sub.textContent = `${phase} · ${p.root}`;
    if (frac !== null) { pbar.classList.remove('indet'); pfill.style.width = `${frac * 100}%`; pct.textContent = t('scan.progress', { pct: fmtPct(frac) }) + (left ? ` · ${t('scan.left', { d: fmtDuration(left) })}` : ''); }
    else { pbar.classList.add('indet'); pct.textContent = ''; }
    cur.textContent = '‎' + (p.current || '');
    fill(stats, 
      statTile(fmtNum(p.files), t('common.files')),
      statTile(fmtNum(p.dirs), t('common.folders')),
      statTile(fmtDuration(p.elapsedMs), t('scan.runtime')),
      statTile(`${fmtBytes(speed)}/s`, t('scan.threads', { n: p.threads, media: mediaName(p.media) })),
      p.errors ? statTile(fmtNum(p.errors), t('scan.unreadable')) : null);
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
      h('div', { class: 'label' }, `${s.root} · ${vol.label || t('over.drive')} · ${mediaName(vol.media)} · ${vol.fs}`),
      h('div', { class: 'row', style: { alignItems: 'flex-end', gap: '16px', marginTop: '8px' } },
        h('div', { class: 'hero-num' }, fmtBytes(free)),
        h('div', { id: 'free-spark' })),
      h('div', { class: 'hero-sub' }, t('over.freeOf', { total: fmtBytes(vol.total), pct: fmtPct(share) })),
      h('div', { class: 'meter', role: 'meter', 'aria-valuenow': Math.round(share * 100), 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-label': t('over.usage') },
        h('i', { style: { width: `${share * 100}%`, background: sev.color } })),
      h('div', { class: 'row', style: { marginTop: '6px' } }, stateBadge(share),
        h('span', { class: 'muted', style: { marginLeft: 'auto' } }, t('over.scanInfo', { ago: fmtAgo(s.scannedAt), d: fmtDuration(s.durationMs) }))))
    : h('div', null,
      h('div', { class: 'label' }, t('common.folder')),
      h('div', { class: 'mono soft', style: { wordBreak: 'break-all', margin: '6px 0' } }, s.root),
      h('div', { class: 'hero-num' }, fmtBytes(s.scanned)),
      h('div', { class: 'hero-sub' }, tn('over.usedIn', s.files)),
      h('div', { class: 'muted', style: { marginTop: '6px' } }, t('over.scanInfo', { ago: fmtAgo(s.scannedAt), d: fmtDuration(s.durationMs) })));

  const stats = h('div', { class: 'stats' },
    statTile(fmtBytes(s.safeBytes), t('over.quick'), t('over.noWorries'), () => go('clean')),
    statTile(s.dupWasted === null ? t('over.dupCheck') : fmtBytes(s.dupWasted), t('over.dup'), s.dupWasted === null ? t('over.notAnalyzed') : t('over.needless'), () => go('dups')),
    statTile(fmtBytes(s.stale2y), t('over.old'), t('over.old2y'), () => go('old')),
    statTile(fmtNum(s.files), t('common.files'), tn('unit.folders', s.dirs), () => go('files')),
    isDrive ? statTile(fmtBytes(s.unaccounted), t('map.notCaptured'), s.denied ? tn('over.foldersUnreadable', s.denied) : t('over.sysAdmin'), () => App.showDenied()) : null,
    s.hardlinks ? statTile(fmtNum(s.hardlinks), t('over.hardlinks'), t('over.countedOnce')) : null,
    s.cloud.count ? statTile(fmtBytes(s.cloud.bytes), t('over.online'), tn('over.cloud', s.cloud.count)) : null,
  );
  const health = h('div', { id: 'health' });

  root.append(h('div', { class: 'grid' },
    h('div', { class: 'panel span-12 hero' }, head, h('div', { class: 'stack' }, stats, health)),

    h('div', { class: 'panel span-7' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('nav.map')),
        h('button', { class: 'link more', onclick: () => go('map') }, t('common.open'))),
      (() => { const m = h('div', { class: 'minimap', id: 'minimap', tabindex: 0, role: 'button', 'aria-label': t('over.mapOpen'), onclick: () => go('map'), onkeydown: (e) => { if (e.key === 'Enter') go('map'); } }); return m; })(),
      h('div', { style: { marginTop: '10px' } }, segBar(catParts(s.cats), { onClick: (p) => go('files', null, { cat: p.id }) }))),

    h('div', { class: 'panel span-5' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('over.best')),
        h('button', { class: 'link more', onclick: () => go('clean') }, t('over.seeAll'))),
      s.recs.length ? h('div', null, s.recs.slice(0, 8).map((r) => h('button', {
        class: 'rec', onclick: () => {
          if (r.kind === 'dups') go('dups');
          else if (r.kind === 'stale') go('old');
          else go('clean', null, { rule: r.rule });
        },
      }, riskBadge(r.risk), h('div', { style: { minWidth: 0 } }, h('div', { class: 'ellipsis', style: { fontWeight: 500 } }, r.title), h('div', { class: 'muted ellipsis', style: { fontSize: '12px' }, title: r.why }, r.why)), h('div', { class: 'val' }, fmtBytes(r.bytes))))) : h('div', { class: 'empty' }, t('over.noRecs'))),

    h('div', { class: 'panel span-6' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('over.where')),
        h('button', { class: 'link more', onclick: () => go('tree') }, t('nav.tree'))),
      rankBars(s.hotspots.slice(0, 10), {
        name: (r) => r.name, path: (r) => r.path, value: (r) => r.alloc, color: (r) => catVar(r.cat), total: s.scanned,
        onClick: (r) => App.select(r.id), onContext: (r, e) => rowMenu(r, e.clientX, e.clientY),
      })),

    h('div', { class: 'panel span-6' },
      h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('over.largest')),
        h('button', { class: 'link more', onclick: () => go('files') }, t('common.all'))),
      rankBars(s.topFiles.slice(0, 10), {
        name: (r) => r.name, path: (r) => r.path, value: (r) => r.alloc, color: (r) => catVar(r.cat), total: s.scanned,
        onClick: (r) => App.select(r.id), onContext: (r, e) => rowMenu(r, e.clientX, e.clientY),
      })),

    h('div', { class: 'panel span-4' }, h('h2', null, t('over.age')),
      columns(s.ages, { color: (r, i) => ageColor(i, s.ages.length), label: (r) => r.short, sub: (r) => tn('unit.files', r.files), onClick: () => go('old') })),
    h('div', { class: 'panel span-4' }, h('h2', null, t('over.sizes')),
      columns(s.sizes, { color: (r, i) => ageColor(i, s.sizes.length), label: (r) => r.label.replaceAll(' MB', 'M').replaceAll(' GB', 'G').replace('< ', '<').replace('> ', '>'), sub: (r) => tn('unit.files', r.files) })),
    h('div', { class: 'panel span-4' }, h('h2', null, t('nav.types')),
      rankBars(s.exts.slice(0, 7), {
        name: (r) => (r.ext ? `.${r.ext}` : t('over.noExt')), value: (r) => r.alloc, total: s.scanned, color: () => 'var(--c0)',
        onClick: (r) => go('files', null, { ext: r.ext || '(ohne)' }),
      })),
    s.years.length > 1 ? h('div', { class: 'panel span-12' }, h('h2', null, t('over.years')),
      columns(s.years, { value: (r) => r.alloc, label: (r) => String(r.year), height: 150, sub: (r) => tn('unit.files', r.files), color: () => 'var(--c0)' })) : null,
  ));

  const mini = new TreeMap(root.querySelector('#minimap'), { mini: true });
  api('map?' + qs({ id: 0, limit: 900, minfrac: 0.0006, maxdepth: 5 })).then((d) => mini.setData(d)).catch(() => {});

  // Laufwerkszustand und Verlauf kommen verzoegert, damit die Seite sofort steht.
  if (isDrive) loadHealth(root, s.root);
  api('history').then((hist) => {
    const vals = hist.entries.map((e) => e.total - e.free);
    const sp = sparkline(vals, { width: 130, height: 40 });
    const el = root.querySelector('#free-spark');
    if (sp && el) { withTip(el, () => tipBlock(null, t('over.usedSpace'), t('over.overScans', { n: vals.length }))); el.append(sp); }
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
      const state = ['Warning', 'Unhealthy'].includes(v.health) ? t(`health.${v.health}`) : v.health;
      const parts = [unknown ? null : v.model, v.bus, v.temp ? `${v.temp} °C` : null, v.wear !== null && v.wear !== undefined ? t('health.wear', { pct: withPct(v.wear) }) : null, v.hours ? tn('health.hours', v.hours) : null].filter(Boolean);
      fill(el, h('div', { class: 'notice ' + (unknown ? 'info' : good ? 'ok' : 'warn') }, icon(unknown ? 'info' : good ? 'check' : 'warn'),
        h('div', null, h('b', null, unknown ? v.model : good ? t('health.ok') : t('health.bad', { state })), h('div', { class: 'muted' }, parts.join(' · ')))));
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
  if (mode === 'age') return [ageLegend(), h('span', { class: 'muted' }, t('map.ageLegend'))];
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
  const modeSeg = h('div', { class: 'seg', role: 'group', 'aria-label': t('map.coloring') });
  const detSeg = h('div', { class: 'seg', role: 'group', 'aria-label': t('map.detailLevel') });
  const upBtn = h('button', { class: 'btn', title: t('map.upTitle'), onclick: () => goUp() }, icon('up'), t('map.up'));
  const freeBtn = h('button', { class: 'chipbtn' + (st.showFree ? ' on' : ''), onclick: () => { st.showFree = !st.showFree; freeBtn.classList.toggle('on', st.showFree); load(cur); } }, t('map.showFree'));
  root.append(
    h('div', { class: 'mapbar' }, upBtn, crumb,
      h('span', { class: 'label' }, t('map.color')), modeSeg,
      h('span', { class: 'label' }, t('map.detail')), detSeg, freeBtn),
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
  const drawMode = () => draw(modeSeg, [['type', t('map.byType')], ['age', t('map.byAge')]], st.mode, (m) => { st.mode = m; tm.setMode(m); drawMode(); fill(legend, ...legendFor(tm, m, S.summary.cats)); });
  const drawDet = () => draw(detSeg, [['grob', t('map.coarse')], ['mittel', t('map.medium')], ['fein', t('map.fine')]], st.detail, (d) => { st.detail = d; drawDet(); load(cur); });
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
  const upBtn = h('button', { class: 'btn', onclick: () => goUp() }, icon('up'), t('map.up'));
  root.append(h('div', { class: 'mapbar' }, upBtn, crumb, h('span', { class: 'muted' }, t('rings.hint'))), stage, legend);
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
  const sortSel = h('select', { class: 'field', 'aria-label': t('tree.sort'), onchange: () => { S.treeSort = sortSel.value; rebuild(); } },
    h('option', { value: 'alloc' }, t('tree.bySize')), h('option', { value: 'name' }, t('tree.byName')), h('option', { value: 'mtime' }, t('tree.byChange')), h('option', { value: 'files' }, t('tree.byFiles')));
  sortSel.value = S.treeSort || 'alloc';
  const panel = h('div', { class: 'panel', style: { padding: '0 0 4px' } },
    h('table', { class: 'tbl' },
      h('thead', null, h('tr', null, h('th', null, t('common.name')), h('th', { class: 'w-bar' }, t('common.share')), h('th', { class: 'w-size right' }, t('common.used')), h('th', { class: 'w-num right' }, t('common.files')), h('th', { class: 'w-date right' }, t('common.modified')), h('th', { class: 'w-act' }))),
      tbody));
  root.append(
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.tree')), h('p', null, t('tree.intro'))),
      h('div', { class: 'actions' }, sortSel, h('button', { class: 'btn', onclick: () => followHeavy() }, icon('search'), t('tree.follow')))),
    panel);

  const open = new Map(); // id -> Zeile
  const rowFor = (n, depth, parentAlloc) => {
    const sel = S.selId === n.id;
    const isDir = n.dir;
    const exp = h('button', {
      class: 'expander', style: { visibility: isDir && n.kids ? 'visible' : 'hidden' }, 'aria-label': t('tree.expand'), 'aria-expanded': 'false',
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
        n.flags & 8 ? h('span', { class: 'tag', title: t('tree.linkTitle') }, t('tree.link')) : null,
        n.flags & 64 ? h('span', { class: 'tag', title: t('tree.deniedTitle') }, t('tree.denied')) : null,
        n.flags & 16 ? h('span', { class: 'tag', title: t('tree.hardTitle') }, t('tree.hard')) : null,
        n.flags & 32 ? h('span', { class: 'tag', title: t('tree.onlineTitle') }, t('over.online')) : null)),
      h('td', null, withTip(h('div', { class: 'tbar' }, h('i', { style: { width: `${share * 100}%`, background: catVar(n.cat) } })), () => tipBlock(fmtPct(share), t('tree.ofParent'), catName(n.cat)))),
      h('td', { class: 'right num' }, fmtBytes(n.alloc)),
      h('td', { class: 'right num muted' }, isDir ? fmtNum(n.files) : ''),
      h('td', { class: 'right num muted', title: fmtDate(isDir ? n.newest : n.mtime) }, fmtDate(isDir ? n.newest : n.mtime)),
      h('td', null, h('button', {
        class: 'btn ghost small icon-only', 'aria-label': t('common.actions'),
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
      const more = h('tr', null, h('td', { colspan: 6, class: 'muted', style: { paddingLeft: `${(depth + 1) * 16 + 26}px` } }, t('tree.more', { n: fmtNum(d.total - d.rows.length) })));
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
    if (!target) { toast(t('tree.noDominant')); return; }
    await revealPath(target.id);
    const tr = tbody.querySelector(`tr[data-id="${target.id}"]`);
    if (tr && tr._node.dir && !open.has(target.id)) await toggle(tr, tr._node, tr._depth);
  }
  await rebuild();
  if (focusId) await revealPath(focusId).catch(fail);
  return { destroy: () => {}, update: (nid) => revealPath(nid).catch(fail), refresh: () => rebuild().catch(fail) };
}

export { statTile };
