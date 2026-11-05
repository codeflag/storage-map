// Grosse Dateien, Typen, Duplikate, Alt & ungenutzt, Aufraeumen, Verlauf.

import {
  api, qs, h, icon, clear, fill, add, App, downloadCsv, fmtBytes, fmtNum, fmtPct, fmtDate, fmtAgo, fmtDateTime,
  catVar, catName, CAT_COUNT, riskBadge, rowMenu, fail, toast, debounce,
} from './util.js';
import { segBar, catParts, columns, ageColor, lineChart, divergingRows } from './charts.js';
import { statTile } from './views.js';
import { t, tn, tp } from './i18n.js';

const YEAR = 365 * 86400;
const now = () => Math.floor(Date.now() / 1000);

/// Datei- und Ordnerzeile, ueberall gleich: Auswahl, Name mit Pfad, Groesse, Datum, Aktionen.
export function fileRow(r, { selected, onToggle, cols = ['alloc', 'mtime'] } = {}) {
  const isSel = selected?.has(r.id);
  const blocked = r.guard?.level === 'block';
  const cb = onToggle ? h('input', {
    type: 'checkbox', checked: isSel, disabled: blocked, 'aria-label': t('files.select', { name: r.name }), title: blocked ? r.guard.reason : undefined,
    onclick: (e) => { e.stopPropagation(); onToggle(r, e.target.checked, tr); },
  }) : null;
  const tr = h('tr', {
    class: 'rowlink' + (isSel ? ' sel' : ''),
    onclick: () => App.select(r.id),
    oncontextmenu: (e) => { e.preventDefault(); rowMenu(r, e.clientX, e.clientY); },
  },
    onToggle ? h('td', { class: 'w-check' }, cb) : null,
    h('td', null, h('div', { class: 'cell-name' },
      r.dir ? icon('folder') : h('i', { class: 'swatch', style: { background: catVar(r.cat) }, title: catName(r.cat) }),
      h('div', { class: 'txt grow' },
        h('div', { class: 'nm', title: r.name }, r.name),
        h('div', { class: 'pt', title: r.path }, '\u200E' + r.path)),
      blocked ? h('span', { class: 'tag', title: r.guard.reason }, t('common.protected')) : r.guard?.level === 'warn' ? h('span', { class: 'tag', title: r.guard.reason }, t('risk.caution')) : null)),
    cols.includes('files') ? h('td', { class: 'right num muted w-num' }, r.dir ? fmtNum(r.files) : '') : null,
    cols.includes('alloc') ? h('td', { class: 'right num w-size', style: { fontWeight: 600 } }, fmtBytes(r.alloc)) : null,
    cols.includes('mtime') ? h('td', { class: 'right num muted w-date', title: t('files.lastModified') }, fmtDate(r.dir ? r.newest : r.mtime)) : null,
    cols.includes('atime') ? h('td', { class: 'right num muted w-date', title: t('files.lastOpened') }, fmtDate(r.atime)) : null,
    h('td', { class: 'w-act' }, h('button', {
      class: 'btn ghost small icon-only', 'aria-label': t('common.actions'),
      onclick: (e) => { e.stopPropagation(); rowMenu(r, e.clientX, e.clientY); },
    }, icon('menu'))));
  return tr;
}

/// Tabelle mit Sortierung, Nachladen und Mehrfachauswahl.
function fileTable({ fetchPage, cols, sort, dir, labels, pageSize = 100, onTotals }) {
  const st = { sort, dir, offset: 0, total: 0 };
  const selected = new Map();
  const tbody = h('tbody');
  const more = h('div', { class: 'loadmore' });
  const bulkInfo = h('span', { class: 'grow' });
  const bulk = h('div', { class: 'bulk', hidden: true },
    bulkInfo,
    h('button', { class: 'btn small', onclick: () => { selected.clear(); tbody.querySelectorAll('input[type=checkbox]').forEach((c) => { c.checked = false; }); tbody.querySelectorAll('tr.sel').forEach((t) => t.classList.remove('sel')); drawBulk(); } }, t('files.clearSel')),
    h('button', { class: 'btn small primary', onclick: async () => { await App.addToCart([...selected.keys()]); selected.clear(); drawBulk(); reload(); } }, icon('plus'), t('common.addToList')));
  const drawBulk = () => {
    bulk.hidden = selected.size === 0;
    if (selected.size === 0) { bulk.style.display = 'none'; return; }
    bulk.style.display = '';
    const sum = [...selected.values()].reduce((a, r) => a + r.alloc, 0);
    bulkInfo.textContent = t('files.selected', { n: fmtNum(selected.size), size: fmtBytes(sum) });
  };
  bulk.style.display = 'none';
  const toggle = (r, on, tr) => { if (on) selected.set(r.id, r); else selected.delete(r.id); tr.classList.toggle('sel', on); drawBulk(); };
  const th = (key, label, cls) => h('th', {
    class: `${cls || ''} ${key ? 'sortable' : ''}`,
    'aria-sort': st.sort === key ? (st.dir === 'asc' ? 'ascending' : 'descending') : undefined,
    onclick: key ? () => { if (st.sort === key) st.dir = st.dir === 'asc' ? 'desc' : 'asc'; else { st.sort = key; st.dir = key === 'name' ? 'asc' : 'desc'; } reload(); } : undefined,
  }, label + (st.sort === key ? (st.dir === 'asc' ? ' ↑' : ' ↓') : ''));
  const thead = h('thead');
  const drawHead = () => fill(thead, h('tr', null,
    h('th', { class: 'w-check' }),
    th('name', labels?.name || t('common.name')),
    cols.includes('files') ? th('files', t('common.files'), 'w-num right') : null,
    cols.includes('alloc') ? th('alloc', t('common.used'), 'w-size right') : null,
    cols.includes('mtime') ? th('mtime', t('common.modified'), 'w-date right') : null,
    cols.includes('atime') ? th('atime', t('common.opened'), 'w-date right') : null,
    h('th', { class: 'w-act' })));
  let generation = 0;
  async function loadMore() {
    const mine = generation;
    const d = await fetchPage({ sort: st.sort, dir: st.dir, offset: st.offset, limit: pageSize });
    if (mine !== generation) return;
    st.total = d.total;
    onTotals && onTotals(d);
    d.rows.forEach((r) => tbody.append(fileRow(r, { selected, onToggle: toggle, cols })));
    st.offset += d.rows.length;
    clear(more);
    if (!d.total) tbody.append(h('tr', null, h('td', { colspan: 8, class: 'empty' }, t('common.nothingFound'))));
    if (st.offset < d.total) more.append(h('button', { class: 'btn', onclick: () => loadMore().catch(fail) }, t('files.loadMore', { n: fmtNum(d.total - st.offset) })));
  }
  async function reload() {
    generation++;
    st.offset = 0;
    drawHead();
    clear(tbody);
    await loadMore();
  }
  drawHead();
  const el = h('div', null, h('table', { class: 'tbl' }, thead, tbody), more, bulk);
  return { el, reload, selected };
}

/* ---------- Grosse Dateien / Suche ---------- */
const fromSize = (size) => t('files.fromSize', { size });
const minSizes = () => [[0, t('files.anySize')], [1 << 20, fromSize('1 MB')], [10 << 20, fromSize('10 MB')], [100 << 20, fromSize('100 MB')], [1 << 30, fromSize('1 GB')], [10 * 2 ** 30, fromSize('10 GB')]];
const ageChoices = () => [[0, t('files.anyAge')], ...[1, 2, 3, 5, 10].map((y) => [y, tn('files.olderThan', y)])];

export async function viewFiles(root, S, _id, params = {}) {
  const f = S.fileFilter = { q: params.q ?? '', cat: params.cat ?? null, ext: params.ext ?? '', min: 0, age: 0, kind: params.kind ?? (params.q ? 'all' : 'file') };
  const info = h('span', { class: 'muted num' });
  const queryObj = () => ({
    q: f.q, ext: f.ext, cat: f.cat ?? '', min: f.min || '', kind: f.kind,
    mb: f.age ? now() - f.age * YEAR : '',
  });
  const table = fileTable({
    cols: ['alloc', 'mtime', 'atime'], sort: 'alloc', dir: 'desc',
    fetchPage: (p) => api('files?' + qs({ ...queryObj(), ...p })),
    onTotals: (d) => { info.textContent = tn('files.hits', d.total, { size: fmtBytes(d.bytes) }); },
  });
  const search = h('input', { class: 'field', style: { flex: '1 1 240px' }, placeholder: t('files.searchPlaceholder'), value: f.q, 'aria-label': t('top.searchLabel'), oninput: debounce((e) => { f.q = e.target.value; table.reload().catch(fail); }, 220) });
  const chips = h('div', { class: 'row wrap' });
  const drawChips = () => fill(chips, ...[...Array(CAT_COUNT).keys()].map((i) => h('button', {
    class: 'chipbtn' + (f.cat === i ? ' on' : ''),
    onclick: () => { f.cat = f.cat === i ? null : i; f.ext = ''; drawChips(); extTag(); table.reload().catch(fail); },
  }, h('i', { class: 'swatch', style: { background: catVar(i) } }), catName(i))));
  drawChips();
  const extBox = h('span');
  const extTag = () => fill(extBox, ...(f.ext ? [h('button', { class: 'chipbtn on', onclick: () => { f.ext = ''; extTag(); table.reload().catch(fail); } }, f.ext === '(ohne)' ? t('files.noExt') : `.${f.ext}`, icon('x'))] : []));
  extTag();
  const sel = (opts, val, set, label) => {
    const s = h('select', { class: 'field', 'aria-label': label, onchange: () => { set(Number(s.value)); table.reload().catch(fail); } }, opts.map(([v, l]) => h('option', { value: v }, l)));
    s.value = val;
    return s;
  };
  const kindSeg = h('div', { class: 'seg', role: 'group', 'aria-label': t('files.kind') });
  const drawKind = () => fill(kindSeg, ...[['file', t('common.files')], ['dir', t('common.folders')], ['all', t('files.both')]].map(([k, l]) => h('button', { class: f.kind === k ? 'on' : '', onclick: () => { f.kind = k; drawKind(); table.reload().catch(fail); } }, l)));
  drawKind();
  root.append(
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.files')), h('p', null, t('files.intro'))),
      h('div', { class: 'actions' }, h('a', {
        class: 'btn', href: '#', onclick: (e) => { e.preventDefault(); downloadCsv({ what: 'files', ...queryObj() }); },
      }, icon('export'), t('common.asCsv')))),
    h('div', { class: 'panel' },
      h('div', { class: 'filters' }, search, kindSeg,
        sel(minSizes(), f.min, (v) => { f.min = v; }, t('files.minSize')),
        sel(ageChoices(), f.age, (v) => { f.age = v; }, t('files.age')), extBox, info),
      h('div', { class: 'filters' }, chips),
      table.el));
  await table.reload();
  search.focus();
  return { destroy: () => {}, refresh: () => table.reload().catch(fail) };
}

/* ---------- Dateitypen ---------- */
export async function viewTypes(root, S) {
  const [s, types] = await Promise.all([api('summary'), api('types')]);
  S.summary = s;
  const go = App.go;
  root.append(
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.types')), h('p', null, t('types.intro')))),
    h('div', { class: 'grid' },
      h('div', { class: 'panel span-12' }, h('h2', null, t('types.shares')), segBar(catParts(s.cats), { onClick: (p) => go('files', null, { cat: p.id }) })),
      h('div', { class: 'panel span-5' }, h('h2', null, t('types.categories')),
        h('table', { class: 'tbl' },
          h('thead', null, h('tr', null, h('th', null, t('types.category')), h('th', { class: 'w-num right' }, t('common.files')), h('th', { class: 'w-size right' }, t('common.used')), h('th', { class: 'w-num right' }, t('common.share')))),
          h('tbody', null, [...s.cats].sort((a, b) => b.alloc - a.alloc).map((c) => h('tr', { class: 'rowlink', onclick: () => go('files', null, { cat: c.id }) },
            h('td', null, h('div', { class: 'cell-name' }, h('i', { class: 'swatch', style: { background: catVar(c.id) } }), h('span', { class: 'wrapname' }, c.name))),
            h('td', { class: 'right num muted' }, fmtNum(c.files)),
            h('td', { class: 'right num', style: { fontWeight: 600 } }, fmtBytes(c.alloc)),
            h('td', { class: 'right num muted' }, fmtPct(c.alloc / Math.max(1, s.scanned)))))))),
      h('div', { class: 'panel span-7' }, h('h2', null, t('types.extensions')),
        h('table', { class: 'tbl' },
          h('thead', null, h('tr', null, h('th', null, t('types.extension')), h('th', { class: 'w-bar' }, t('common.share')), h('th', { class: 'w-num right' }, t('common.files')), h('th', { class: 'w-size right' }, t('types.largest')), h('th', { class: 'w-size right' }, t('common.used')))),
          h('tbody', null, types.exts.slice(0, 60).map((e) => h('tr', { class: 'rowlink', onclick: () => go('files', null, { ext: e.ext || '(ohne)' }) },
            h('td', null, h('div', { class: 'cell-name' }, h('i', { class: 'swatch', style: { background: catVar(e.cat) }, title: catName(e.cat) }), h('span', { class: 'nm mono' }, e.ext ? `.${e.ext}` : t('types.none')))),
            h('td', null, h('div', { class: 'tbar' }, h('i', { style: { width: `${(e.alloc / Math.max(1, types.exts[0].alloc)) * 100}%`, background: catVar(e.cat) } }))),
            h('td', { class: 'right num muted' }, fmtNum(e.files)),
            h('td', { class: 'right num muted' }, fmtBytes(e.largest)),
            h('td', { class: 'right num', style: { fontWeight: 600 } }, fmtBytes(e.alloc)))))))));
  return () => {};
}

/* ---------- Duplikate ---------- */
export async function viewDups(root, S) {
  const params = S.dupParams = S.dupParams || { minSize: 1 << 20, quick: false, includeSystem: false, tab: 'files' };
  const outer = h('div');
  const head = h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.dups')),
    h('p', null, t('dups.intro'))));
  const tabs = h('div', { class: 'seg', role: 'group', style: { marginBottom: '10px' } });
  const drawTabs = () => fill(tabs, ...[['files', t('dups.tabFiles')], ['folders', t('dups.tabFolders')]].map(([k, l]) => h('button', { class: params.tab === k ? 'on' : '', onclick: () => { params.tab = k; drawTabs(); draw().catch(fail); } }, l)));
  drawTabs();
  root.append(head, tabs, outer);

  let progEl = null;
  async function draw() {
    // Eigener Behaelter je Durchlauf: Ein ueberholter Durchlauf schreibt so ins Leere.
    const body = h('div');
    fill(outer, body);
    progEl = null;
    if (params.tab === 'folders') return drawFolders(body);
    const st = S.status?.dups;
    body.append(controls(st));
    if (st?.running) {
      const fill = h('i', { style: { width: '0%' } });
      const txt = h('div', { class: 'muted num' });
      progEl = { fill, txt };
      body.append(h('div', { class: 'panel', style: { marginTop: '12px' } }, h('div', { class: 'label' }, t('dups.running')), h('div', { class: 'pbar' }, fill), txt));
      updateProgress(st);
      return;
    }
    if (st?.done) await results(body);
    else body.append(h('div', { class: 'panel empty', style: { marginTop: '12px' } }, t('dups.startHint')));
  }
  function controls(st) {
    const min = h('select', { class: 'field', 'aria-label': t('files.minSize'), onchange: () => { params.minSize = Number(min.value); } },
      [[1 << 16, '64 KB'], [1 << 20, '1 MB'], [10 << 20, '10 MB'], [100 << 20, '100 MB'], [1 << 30, '1 GB']].map(([v, l]) => h('option', { value: v }, fromSize(l))));
    min.value = params.minSize;
    return h('div', { class: 'panel' }, h('div', { class: 'filters', style: { marginBottom: 0 } },
      st?.running
        ? h('button', { class: 'btn', onclick: () => api('dups/cancel', {}).catch(fail) }, icon('x'), t('common.cancel'))
        : h('button', { class: 'btn primary', onclick: () => api('dups/start', params).then(() => App.poll()).then(() => draw()).catch(fail) }, icon('search'), st?.done ? t('dups.again') : t('dups.start')),
      min,
      h('label', { class: 'row', title: t('dups.quickTitle') }, h('input', { type: 'checkbox', checked: params.quick, onchange: (e) => { params.quick = e.target.checked; } }), t('dups.quick')),
      h('label', { class: 'row', title: t('dups.sysTitle') }, h('input', { type: 'checkbox', checked: params.includeSystem, onchange: (e) => { params.includeSystem = e.target.checked; } }), t('dups.sys')),
      st?.done ? h('a', { class: 'btn', style: { marginLeft: 'auto' }, href: '#', onclick: (e) => { e.preventDefault(); downloadCsv({ what: 'dups' }); } }, icon('export'), t('common.asCsv')) : null));
  }
  function updateProgress(st) {
    if (!progEl || !st) return;
    const frac = st.totalBytes ? st.doneBytes / st.totalBytes : 0;
    progEl.fill.style.width = `${frac * 100}%`;
    const phase = st.phase === 'full' ? t('dups.phaseFull') : st.phase === 'collect' ? t('dups.phaseCollect') : t('dups.phasePartial');
    progEl.txt.textContent = t('dups.progress', { phase, done: fmtBytes(st.doneBytes), total: fmtBytes(st.totalBytes), fdone: fmtNum(st.doneFiles), ftotal: fmtNum(st.totalFiles), groups: fmtNum(st.groups) });
  }
  async function results(body) {
    let offset = 0;
    const d = await api('dups?' + qs({ limit: 40 }));
    const groupsEl = h('div');
    const chosen = new Map();
    const info = h('span', { class: 'grow' });
    const drawInfo = () => {
      const sum = [...chosen.values()].reduce((a, r) => a + r.alloc, 0);
      info.textContent = chosen.size ? tn('dups.chosen', chosen.size, { size: fmtBytes(sum) }) : t('dups.hint');
    };
    drawInfo();
    const groups = [];
    const addGroups = (rows) => rows.forEach((g) => { const el = groupEl(g, chosen, drawInfo); groups.push({ g, el }); groupsEl.append(el.root); });
    addGroups(d.rows);
    offset += d.rows.length;
    const auto = (pick) => {
      groups.forEach(({ g, el }) => {
        const keep = pick(g.files);
        g.files.forEach((f) => {
          const off = f.id !== keep.id && f.guard?.level !== 'block';
          if (off) chosen.set(f.id, f); else chosen.delete(f.id);
          el.set(f.id, off);
        });
      });
      drawInfo();
    };
    const more = h('div', { class: 'loadmore' });
    const drawMore = () => {
      clear(more);
      if (offset < d.total) more.append(h('button', { class: 'btn', onclick: async () => {
        const n = await api('dups?' + qs({ offset, limit: 40 }));
        addGroups(n.rows); offset += n.rows.length; drawMore();
      } }, t('dups.moreGroups', { n: fmtNum(d.total - offset) })));
    };
    drawMore();
    body.append(
      h('div', { class: 'panel', style: { marginTop: '12px' } },
        h('div', { class: 'stats' },
          statTile(fmtBytes(d.wasted), t('dups.wasted'), t('dups.wastedSub')),
          statTile(fmtNum(d.total), t('dups.groups')),
          statTile(fmtNum(d.files), t('dups.affected')),
          S.status?.dups?.skipped ? statTile(fmtNum(S.status.dups.skipped), t('dups.skipped'), t('dups.skippedSub')) : null)),
      d.total ? h('div', { class: 'panel', style: { marginTop: '12px' } },
        h('div', { class: 'filters' },
          h('span', { class: 'label' }, t('dups.auto')),
          h('button', { class: 'btn small', onclick: () => auto((fs) => [...fs].sort((a, b) => b.mtime - a.mtime)[0]) }, t('dups.keepNew')),
          h('button', { class: 'btn small', onclick: () => auto((fs) => [...fs].sort((a, b) => a.mtime - b.mtime)[0]) }, t('dups.keepOld')),
          h('button', { class: 'btn small', onclick: () => auto((fs) => [...fs].sort((a, b) => a.path.length - b.path.length)[0]) }, t('dups.keepShort')),
          h('button', { class: 'btn small ghost', onclick: () => { groups.forEach(({ g, el }) => g.files.forEach((f) => { chosen.delete(f.id); el.set(f.id, false); })); drawInfo(); } }, t('dups.none'))),
        groupsEl, more,
        h('div', { class: 'bulk' }, info, h('button', {
          class: 'btn small primary', onclick: async () => {
            if (!chosen.size) { toast(t('dups.nothingSel')); return; }
            await App.addToCart([...chosen.keys()]);
            chosen.clear(); drawInfo(); draw().catch(fail);
          },
        }, icon('plus'), t('common.addToList')))) : h('div', { class: 'panel empty', style: { marginTop: '12px' } }, t('dups.noneFound')));
  }
  function groupEl(g, chosen, onChange) {
    const boxes = new Map();
    const rows = g.files.map((f) => {
      const blocked = f.guard?.level === 'block';
      const cb = h('input', { type: 'checkbox', disabled: blocked, 'aria-label': t('common.removeName', { name: f.path }), title: blocked ? f.guard.reason : t('dups.removeTitle'), onchange: (e) => { if (e.target.checked) chosen.set(f.id, f); else chosen.delete(f.id); onChange(); } });
      boxes.set(f.id, cb);
      return h('div', { class: 'dupfile', oncontextmenu: (e) => { e.preventDefault(); rowMenu(f, e.clientX, e.clientY); } },
        cb,
        h('button', { class: 'mono ellipsis link', style: { textAlign: 'left', textDecoration: 'none' }, title: f.path, onclick: () => App.select(f.id) }, f.path),
        h('span', { class: 'muted num' }, fmtDate(f.mtime)),
        h('button', { class: 'btn ghost small icon-only', 'aria-label': t('common.showInFm'), title: t('common.showInFm'), onclick: () => App.reveal(f.id) }, icon('explorer')));
    });
    return {
      root: h('div', { class: 'dupgroup' },
        h('div', { class: 'row', style: { marginBottom: '3px' } },
          h('b', null, g.files[0].name),
          h('span', { class: 'muted' }, `${g.files.length} × ${fmtBytes(g.alloc)}`),
          !g.verified ? h('span', { class: 'tag', title: t('dups.quickOnly') }, t('dups.quick')) : null,
          h('span', { style: { marginLeft: 'auto', fontWeight: 600 }, class: 'num' }, t('dups.needless', { size: fmtBytes(g.wasted) }))),
        rows),
      set: (id, on) => { const c = boxes.get(id); if (c) c.checked = on; },
    };
  }
  async function drawFolders(body) {
    const d = await api('dupfolders');
    body.append(
      h('div', { class: 'notice info' }, icon('info'), h('div', null, tp('dups.folderNotice', { not: h('b', null, t('dups.not')) }))),
      d.total ? h('div', { class: 'panel', style: { marginTop: '12px' } },
        h('div', { class: 'stats', style: { marginBottom: '10px' } }, statTile(fmtBytes(d.wasted), t('dups.possibleFree')), statTile(fmtNum(d.total), t('dups.groups'))),
        d.rows.map((g) => h('div', { class: 'dupgroup' },
          h('div', { class: 'row', style: { marginBottom: '3px' } },
            h('b', null, g.folders[0].name), h('span', { class: 'muted' }, t('dups.folderRow', { count: g.folders.length, size: fmtBytes(g.alloc), files: fmtNum(g.fileCount) })),
            h('span', { style: { marginLeft: 'auto', fontWeight: 600 }, class: 'num' }, t('dups.needless', { size: fmtBytes(g.wasted) }))),
          g.folders.map((f) => h('div', { class: 'dupfile', oncontextmenu: (e) => { e.preventDefault(); rowMenu(f, e.clientX, e.clientY); } },
            h('button', { class: 'btn ghost small icon-only', 'aria-label': t('common.addToList'), title: t('common.addToList'), disabled: f.guard?.level === 'block', onclick: () => App.addToCart([f.id]) }, icon('plus')),
            h('button', { class: 'mono ellipsis link', style: { textAlign: 'left', textDecoration: 'none' }, title: f.path, onclick: () => App.select(f.id) }, f.path),
            h('span', { class: 'muted num' }, fmtDate(f.newest)),
            h('button', { class: 'btn ghost small icon-only', 'aria-label': t('common.showInFm'), title: t('common.showInFm'), onclick: () => App.reveal(f.id) }, icon('explorer'))))))) : h('div', { class: 'panel empty', style: { marginTop: '12px' } }, t('dups.noFolders')));
  }
  await draw();
  let wasRunning = S.status?.dups?.running;
  return {
    destroy: () => {},
    tick: (st) => {
      if (wasRunning && !st.dups.running) { wasRunning = false; draw().catch(fail); return; }
      wasRunning = st.dups.running;
      updateProgress(st.dups);
    },
    refresh: () => draw().catch(fail),
  };
}

/* ---------- Alt & ungenutzt ---------- */
export async function viewOld(root, S) {
  const [s, settings] = await Promise.all([api('summary'), api('settings')]);
  S.summary = s;
  const st = S.oldState = S.oldState || { years: 3, mode: 'mtime' };
  const cutoff = () => now() - st.years * YEAR;
  const seg = h('div', { class: 'seg' });
  const modeSeg = h('div', { class: 'seg' });
  const chartEl = h('div');
  const staleEl = h('div');
  const infoFiles = h('span', { class: 'muted num' });
  const table = fileTable({
    cols: ['alloc', 'mtime', 'atime'], sort: 'alloc', dir: 'desc',
    fetchPage: (p) => api('files?' + qs({ ...(st.mode === 'mtime' ? { mb: cutoff() } : { ab: cutoff() }), min: 1 << 16, ...p })),
    onTotals: (d) => { infoFiles.textContent = `${tn('unit.files', d.total)} · ${fmtBytes(d.bytes)}`; },
  });
  const drawChart = () => {
    const src = st.mode === 'mtime' ? s.ages : s.accessAges;
    fill(chartEl, columns(src, { color: (r, i) => ageColor(i, src.length), label: (r) => r.label, sub: (r) => tn('unit.files', r.files), height: 180 }));
  };
  const drawSegs = () => {
    fill(seg, ...[1, 2, 3, 5, 10].map((y) => h('button', { class: st.years === y ? 'on' : '', onclick: () => { st.years = y; drawSegs(); reload(); } }, t('old.years', { n: y }))));
    fill(modeSeg, ...[['mtime', t('old.notChanged')], ['atime', t('old.notOpened')]].map(([k, l]) => h('button', { class: st.mode === k ? 'on' : '', onclick: () => { st.mode = k; drawSegs(); drawChart(); reload(); } }, l)));
  };
  async function reload() {
    const d = await api('stale?' + qs({ cutoff: cutoff(), limit: 40, min: 10 << 20 }));
    fill(staleEl, 
      h('div', { class: 'row', style: { marginBottom: '8px' } }, h('div', { class: 'panel-title', style: { margin: 0 } }, t('old.forgotten')), h('span', { class: 'muted num', style: { marginLeft: 'auto' } }, `${tn('unit.folders', d.total)} · ${fmtBytes(d.bytes)}`)),
      h('p', { class: 'muted', style: { marginBottom: '8px' } }, tn('old.staleText', st.years)),
      d.rows.length ? h('table', { class: 'tbl' }, h('tbody', null, d.rows.map((r) => fileRow(r, { cols: ['files', 'alloc', 'mtime'] })))) : h('div', { class: 'empty' }, t('common.noneFound')));
    await table.reload();
  }
  drawSegs(); drawChart();
  add(root,
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.old')), h('p', null, t('old.intro'))),
      h('div', { class: 'actions' }, modeSeg, seg)),
    settings.lastAccessTracking === false ? h('div', { class: 'notice warn', style: { marginBottom: '12px' } }, icon('warn'), h('div', null, tp('old.accessWarn', { a: h('b', null, t('old.quotedOpened')), b: h('b', null, t('old.quotedChanged')) }))) : null,
    h('div', { class: 'grid' },
      h('div', { class: 'panel span-5' }, h('h2', null, t('old.byAge')), chartEl),
      h('div', { class: 'panel span-7', style: { maxHeight: '420px', overflow: 'auto' } }, staleEl),
      h('div', { class: 'panel span-12' }, h('div', { class: 'row', style: { marginBottom: '8px' } }, h('div', { class: 'panel-title', style: { margin: 0 } }, t('old.oldFiles')), h('span', { style: { marginLeft: 'auto' } }, infoFiles)), table.el)));
  await reload();
  return { destroy: () => {}, refresh: () => reload().catch(fail) };
}

/* ---------- Aufraeumen ---------- */
export async function viewClean(root, S, _id, params = {}) {
  const content = h('div');
  root.append(
    h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.clean')), h('p', null, t('clean.intro')))),
    content);
  const openRules = new Set(params.rule ? [params.rule] : []);
  async function draw() {
    const j = await api('junk');
    clear(content);
    const safeRules = j.rules.filter((r) => r.risk === 'safe' && r.action === 'none');
    const sumBy = (risk) => j.rules.filter((r) => r.risk === risk).reduce((a, r) => a + r.alloc, 0);
    content.append(h('div', { class: 'panel', style: { marginBottom: '12px' } },
      h('div', { class: 'row wrap', style: { gap: '20px' } },
        h('div', { class: 'stats grow' },
          statTile(fmtBytes(sumBy('safe')), t('risk.safe'), t('clean.safeSub')),
          statTile(fmtBytes(sumBy('caution')), t('risk.caution'), t('clean.cautionSub')),
          statTile(fmtBytes(sumBy('review')), t('risk.review'), t('clean.reviewSub')),
          j.recycleBin ? statTile(fmtBytes(j.recycleBin.bytes), t('clean.inBin'), tn('unit.entries', j.recycleBin.items)) : null),
        h('div', { class: 'row wrap' },
          j.recycleBin && j.recycleBin.bytes > 0 ? h('button', { class: 'btn', onclick: () => App.emptyBin(j.recycleBin.bytes) }, icon('trash'), t('bin.emptyMenu')) : null,
          safeRules.length ? h('button', { class: 'btn primary', onclick: async () => { const r = await api('cart/add', { safeRules: true }).catch(fail); if (r) { App.afterCart(r); draw().catch(fail); } } }, icon('plus'), t('clean.addSafe'))  : null))));
    const groups = [...new Set(j.rules.map((r) => r.group))];
    groups.forEach((g) => {
      const rules = j.rules.filter((r) => r.group === g);
      const total = rules.reduce((a, r) => a + r.alloc, 0);
      content.append(h('div', { class: 'panel', style: { marginBottom: '12px' } },
        h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, g), h('span', { class: 'more num soft' }, fmtBytes(total))),
        rules.map((r) => ruleEl(r))));
    });
    if (j.emptyDirs) {
      content.append(h('div', { class: 'panel', style: { marginBottom: '12px' } },
        h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('clean.tidy'))),
        ruleEl({ id: 'empty', title: t('clean.emptyTitle'), why: t('clean.emptyWhy'), risk: 'safe', action: 'none', alloc: 0, files: 0, items: j.emptyDirs })));
    }
    if (!j.rules.length && !j.emptyDirs) content.append(h('div', { class: 'panel empty' }, t('clean.noHits')));
  }
  function ruleEl(r) {
    const bodyEl = h('div', { class: 'rule-body', hidden: true });
    const exp = h('button', { class: 'expander', 'aria-label': t('clean.showItems'), onclick: () => toggle() }, icon('chevron'));
    const toggle = async () => {
      if (!bodyEl.hidden) { bodyEl.hidden = true; exp.classList.remove('open'); openRules.delete(r.id); return; }
      bodyEl.hidden = false; exp.classList.add('open'); openRules.add(r.id);
      if (bodyEl.childElementCount) return;
      await drawItems(bodyEl, r);
    };
    let action = null;
    if (r.action === 'emptyrecyclebin') action = h('button', { class: 'btn small', onclick: () => App.emptyBin(r.alloc) }, icon('trash'), t('clean.empty'));
    else if (r.action === 'diskcleanup') action = h('button', { class: 'btn small', onclick: () => api('tool/cleanmgr', {}).then((x) => toast(x.simulated ? t('common.simulated') : t('clean.cleanmgrOpened'))).catch(fail) }, t('clean.diskCleanup'));
    else if (r.risk !== 'info') action = h('button', { class: 'btn small', onclick: async () => { const x = await api('cart/add', { rule: r.id }).catch(fail); if (x) { App.afterCart(x); } } }, icon('plus'), t('clean.addAll'));
    const el = h('div', { class: 'rule' },
      h('div', { class: 'rule-head' }, exp, riskBadge(r.risk),
        h('div', { style: { minWidth: 0 } }, h('div', { class: 't' }, r.title), h('div', { class: 'w' }, r.why)),
        h('div', { class: 'num right', style: { fontWeight: 600, minWidth: '74px' } }, r.alloc ? fmtBytes(r.alloc) : `${fmtNum(r.items)}`),
        action || h('span')),
      bodyEl);
    if (openRules.has(r.id)) { toggle(); setTimeout(() => el.scrollIntoView({ block: 'center' }), 60); }
    return el;
  }
  async function drawItems(el, r) {
    const d = await api('junk/items?' + qs({ rule: r.id, limit: 60 }));
    clear(el);
    el.append(h('table', { class: 'tbl' }, h('tbody', null, d.rows.map((x) => fileRow(x, { cols: ['alloc', 'mtime'] })))));
    if (d.total > d.rows.length) el.append(h('div', { class: 'muted', style: { padding: '6px 8px' } }, t('clean.more', { n: fmtNum(d.total - d.rows.length) })));
  }
  await draw();
  return { destroy: () => {}, refresh: () => draw().catch(fail) };
}

/* ---------- Verlauf ---------- */
function fmtSpan(days) {
  if (days > 3650) return t('hist.over10');
  if (days > 730) return tn('unit.years', Math.round(days / 365));
  if (days > 60) return tn('unit.months', Math.round(days / 30.4));
  return tn('unit.days', Math.max(1, Math.round(days)));
}

export async function viewHistory(root, S) {
  const hist = await api('history');
  const entries = hist.entries;
  const isDrive = S.status?.index?.isDrive;
  const series = entries.map((e) => ({ x: e.ts, y: isDrive ? e.total - e.free : e.scanned, sub: tn('unit.files', e.files) }));
  const diffEl = h('div');
  const newFiles = h('div');
  const head = h('div', { class: 'page-head' }, h('div', null, h('h1', null, t('nav.history')), h('p', null, t('hist.intro'))));
  root.append(head);
  if (entries.length < 2) {
    root.append(h('div', { class: 'panel empty' }, t('hist.tooFew')));
    root.append(scheduleHint(S));
    return () => {};
  }
  const a = series[0], b = series[series.length - 1];
  const days = (b.x - a.x) / 86400 || 1;
  const perDay = (b.y - a.y) / days;
  const last = entries[entries.length - 1];
  const free = S.status?.index?.free ?? last.free;
  const forecast = isDrive && perDay > 0 ? free / perDay : null;
  const sel = h('select', { class: 'field', 'aria-label': t('hist.compareWith'), onchange: () => loadDiff(Number(sel.value)) },
    entries.slice(0, -1).reverse().map((e) => h('option', { value: e.ts }, `${fmtDateTime(e.ts)} (${fmtAgo(e.ts)})`)));
  async function loadDiff(ts) {
    try {
      const d = await api('diff?' + qs({ ts }));
      const delta = d.afterTotal - d.beforeTotal;
      fill(diffEl, 
        h('div', { class: 'row', style: { marginBottom: '8px' } },
          h('span', { class: 'soft' }, t('hist.total')),
          h('b', { class: 'num' }, `${delta >= 0 ? '+' : '−'}${fmtBytes(Math.abs(delta))}`),
          h('span', { class: 'muted', style: { marginLeft: 'auto' } }, t('hist.legend'))),
        d.rows.length ? divergingRows(d.rows, {
          delta: (r) => r.delta,
          name: (r) => h('div', { class: 'cell-name' },
            r.status === 'new' ? h('span', { class: 'tag' }, t('hist.new')) : r.status === 'gone' ? h('span', { class: 'tag' }, t('hist.gone')) : null,
            h('div', { class: 'pt mono grow', style: { color: 'var(--ink)' }, title: r.path }, '\u200E' + r.path)),
        }) : h('div', { class: 'empty' }, t('hist.noChanges')));
      const files = await api('files?' + qs({ ma: ts, min: 50 << 20, limit: 25 }));
      fill(newFiles, 
        h('div', { class: 'row', style: { marginBottom: '8px' } }, h('div', { class: 'panel-title', style: { margin: 0 } }, t('hist.bigNew')), h('span', { class: 'muted num', style: { marginLeft: 'auto' } }, `${fmtNum(files.total)} · ${fmtBytes(files.bytes)}`)),
        files.rows.length ? h('table', { class: 'tbl' }, h('tbody', null, files.rows.map((r) => fileRow(r, { cols: ['alloc', 'mtime'] })))) : h('div', { class: 'empty' }, t('hist.noneBig')));
    } catch (e) { fill(diffEl, h('div', { class: 'empty' }, e.message)); }
  }
  root.append(h('div', { class: 'grid' },
    h('div', { class: 'panel span-8' }, h('h2', null, isDrive ? t('over.usedSpace') : t('hist.folderSize')),
      lineChart(series, { fmtX: (x) => fmtDate(x), limit: isDrive ? last.total : undefined, limitLabel: t('chart.capacityOf', { size: fmtBytes(last.total) }) })),
    h('div', { class: 'panel span-4' }, h('h2', null, t('hist.trend')),
      h('div', { class: 'stats' },
        statTile(`${perDay >= 0 ? '+' : '−'}${fmtBytes(Math.abs(perDay))}`, t('hist.perDay'), tn('hist.overDays', Math.round(days))),
        statTile(`${perDay >= 0 ? '+' : '−'}${fmtBytes(Math.abs(perDay * 30))}`, t('hist.perMonth')),
        forecast !== null ? statTile(fmtSpan(forecast), t('hist.untilFull'), t('hist.ifContinues')) : null,
        statTile(fmtNum(entries.length), t('hist.scans'))),
      h('div', { style: { marginTop: '14px' } }, scheduleHint(S))),
    h('div', { class: 'panel span-12' },
      h('div', { class: 'filters' }, h('span', { class: 'panel-title', style: { margin: 0 } }, t('hist.whatChanged')), sel), diffEl),
    h('div', { class: 'panel span-12' }, newFiles)));
  await loadDiff(Number(sel.value));
  return () => {};
}

function scheduleHint(S) {
  const r = S.status?.index?.root || (App.isWindows() ? 'C:\\' : '/');
  const exe = S.status?.exe || 'storage-map';
  const cmd = App.isWindows()
    ? `schtasks /Create /SC WEEKLY /D MO /ST 12:00 /TN "Storage Map Scan" /TR "\\"${exe}\\" --scan ${r}"`
    : `(crontab -l 2>/dev/null; echo '0 12 * * 1 "${exe}" --scan "${r}"') | crontab -`;
  return h('div', { class: 'notice info' }, icon('info'), h('div', null,
    t('hist.schedule'),
    h('div', { class: 'mono', style: { marginTop: '6px', wordBreak: 'break-all' } }, cmd),
    h('button', { class: 'btn small', style: { marginTop: '6px' }, onclick: () => navigator.clipboard.writeText(cmd).then(() => toast(t('hist.cmdCopied'))).catch(fail) }, icon('copy'), t('common.copy'))));
}

