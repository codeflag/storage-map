// KI-Zugriff: Rechte des MCP-Servers, Einrichtung und Protokoll.

import { api, h, icon, clear, fill, App, fmtDateTime, fmtNum, fail, toast, dialog } from './util.js';
import { t, tp, toolText } from './i18n.js';

const LEVELS = ['off', 'read', 'write', 'delete'];
const RANK = { off: 0, read: 1, write: 2, delete: 3 };
const NEEDS = ['always', 'read', 'scan', 'write', 'delete', 'permanent'];

function save(body) {
  return api('mcp/config', body).then(() => App.poll());
}

/// Fragt vor mehr Rechten nach. Das Bestätigungswort prüft auch das Backend.
function confirmRaise({ title, risks, word, label, body, done }) {
  const input = h('input', {
    class: 'field', placeholder: word, 'aria-label': t('common.confirmWordLabel'), style: { width: '100%' }, autocomplete: 'off',
    oninput: () => { document.getElementById('mcp-go').disabled = input.value !== word; },
  });
  dialog({
    title,
    body: [
      h('div', { class: 'notice block' }, icon('warn'), h('div', null, h('b', null, t('raise.warnBold')), t('raise.warnRest'))),
      h('ul', { class: 'plain-list' }, risks.map((r) => h('li', null, r))),
      h('p', { class: 'soft' }, t('raise.foot')),
      h('label', null, tp('common.confirmType', { word: h('b', null, word) })),
      input,
    ],
    actions: [
      { label: t('common.cancel') },
      { label, kind: 'danger', id: 'mcp-go', disabled: true, run: () => save({ ...body, confirm: input.value }).then(done).catch(fail) },
    ],
  });
}

function askLevel(id, current, done) {
  if (RANK[id] <= RANK[current]) { save({ level: id }).then(done).catch(fail); return; }
  if (id === 'read') { save({ level: id }).then(done).catch(fail); return; }
  if (id === 'write') {
    confirmRaise({
      title: t('raise.writeTitle'),
      word: t('common.wordWrite'),
      label: t('raise.writeLabel'),
      body: { level: 'write' },
      done,
      risks: [t('raise.writeRisk1'), t('raise.writeRisk2')],
    });
    return;
  }
  confirmRaise({
    title: t('raise.deleteTitle'),
    word: t('common.wordDelete'),
    label: t('raise.deleteLabel'),
    body: { level: 'delete' },
    done,
    risks: [t('raise.deleteRisk1'), t('raise.deleteRisk2'), t('raise.deleteRisk3')],
  });
}

function codeBlock(text) {
  return h('div', { class: 'code-block' },
    h('pre', { class: 'mono' }, text),
    h('button', { class: 'btn small', onclick: () => navigator.clipboard.writeText(text).then(() => toast(t('mcp.copied'))).catch(fail) }, icon('copy'), t('common.copy')));
}

function quoteArg(a) {
  return /[\s"]/.test(a) ? `"${a.replaceAll('"', '\\"')}"` : a;
}

function auditTable(rows) {
  if (!rows.length) return h('div', { class: 'empty' }, t('mcp.noCalls'));
  const label = { ok: t('mcp.outcomeOk'), denied: t('mcp.outcomeDenied'), error: t('mcp.outcomeError') };
  return h('div', { style: { overflow: 'auto', maxHeight: '420px' } }, h('table', { class: 'tbl' },
    h('thead', null, h('tr', null, h('th', null, t('mcp.time')), h('th', null, t('mcp.client')), h('th', null, t('mcp.tool')), h('th', null, t('mcp.args')), h('th', null, t('mcp.result')))),
    h('tbody', null, rows.map((r) => h('tr', null,
      h('td', { class: 'num', style: { whiteSpace: 'nowrap' } }, fmtDateTime(r.ts)),
      h('td', { class: 'ellipsis', style: { maxWidth: '140px' } }, r.client),
      h('td', { class: 'mono' }, r.tool),
      h('td', { class: 'mono ellipsis', style: { maxWidth: '360px' }, title: r.args }, r.args === '{}' ? '' : r.args),
      h('td', { title: r.detail || undefined }, h('span', { class: 'tag' + (r.outcome === 'ok' ? '' : ' tag-warn') }, label[r.outcome] || r.outcome), r.detail && r.outcome === 'ok' ? h('span', { class: 'muted', style: { marginLeft: '6px' } }, r.detail) : null))))));
}

export async function viewMcp(root, S) {
  const body = h('div');
  const audit = h('div');
  root.append(h('div', { class: 'mcp' },
    h('div', { class: 'row', style: { alignItems: 'baseline' } },
      h('h1', { style: { margin: 0 } }, t('nav.mcp')),
      S.status?.index ? null : h('button', { class: 'link more', style: { marginLeft: 'auto' }, onclick: () => { location.hash = ''; App.poll().then(() => App.home()); } }, t('mcp.back'))),
    h('p', { class: 'lead' }, t('mcp.lead')),
    body,
    h('div', { class: 'panel', style: { marginTop: '14px' } }, h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, t('mcp.log')),
      h('button', { class: 'link more', onclick: () => loadAudit() }, t('mcp.refresh'))), audit)));

  async function loadAudit() {
    const a = await api('mcp/audit?limit=200').catch(() => null);
    if (a && audit.isConnected) fill(audit, auditTable(a.rows));
  }

  async function draw() {
    const [info, drives] = await Promise.all([api('mcp'), api('drives')]);
    const c = info.config;
    clear(body);
    if (info.error) {
      body.append(h('div', { class: 'notice block', style: { marginBottom: '12px' } }, icon('warn'), h('div', null,
        h('b', null, t('mcp.damaged')), t('mcp.damagedRest', { error: info.error }))));
    }

    body.append(h('div', { class: 'levels', role: 'radiogroup', 'aria-label': t('mcp.levelLabel') }, LEVELS.map((l) => h('button', {
      class: 'level' + (c.level === l ? ' on' : '') + (RANK[l] >= 2 ? ' risky' : ''),
      role: 'radio', 'aria-checked': c.level === l ? 'true' : 'false',
      onclick: () => { if (c.level !== l) askLevel(l, c.level, draw); },
    }, h('b', null, t(`level.${l}`)), h('span', { class: 'soft' }, t(`levelText.${l}`))))));

    if (c.level !== 'off') {
      body.append(h('div', { class: 'notice info', style: { marginTop: '12px' } }, icon('info'), h('div', null,
        t('mcp.privacy'))));
    }

    const scanBox = h('input', { type: 'checkbox', checked: c.allow_scan, disabled: c.level === 'off', onchange: (e) => save({ allowScan: e.target.checked }).then(draw).catch(fail) });
    const permBox = h('input', {
      type: 'checkbox', checked: c.allow_permanent_delete, disabled: c.level !== 'delete',
      onchange: (e) => {
        if (!e.target.checked) { save({ allowPermanent: false }).then(draw).catch(fail); return; }
        e.target.checked = false;
        confirmRaise({
          title: t('raise.permTitle'),
          word: t('common.wordDelete'),
          label: t('raise.permLabel'),
          body: { allowPermanent: true },
          done: draw,
          risks: [t('raise.permRisk1'), t('raise.permRisk2')],
        });
      },
    });
    const known = drives.drives.map((d) => d.root).concat(drives.folders.map((f) => f.root));
    const allShown = c.roots.length === 0;
    const setRoots = (roots) => save({ roots }).then(draw).catch(fail);
    body.append(h('div', { class: 'grid', style: { marginTop: '14px' } },
      h('div', { class: 'panel span-6' }, h('h2', null, t('mcp.extra')),
        h('label', { class: 'check' }, scanBox, h('span', null, h('b', null, t('mcp.scanBox')), h('span', { class: 'muted block-line' }, t('mcp.scanText')))),
        h('label', { class: 'check' }, permBox, h('span', null, h('b', null, t('mcp.permBox')), h('span', { class: 'muted block-line' }, c.level === 'delete' ? t('mcp.permOn') : t('mcp.permOff'))))),
      h('div', { class: 'panel span-6' }, h('h2', null, t('mcp.visible')),
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: allShown, onchange: (e) => setRoots(e.target.checked ? [] : known) }), h('span', null, h('b', null, t('common.all')))),
        allShown ? null : known.map((r) => h('label', { class: 'check' },
          h('input', { type: 'checkbox', checked: c.roots.includes(r), onchange: (e) => setRoots(e.target.checked ? [...c.roots, r] : c.roots.filter((x) => x !== r)) }),
          h('span', { class: 'mono' }, r))))));

    const args = info.args;
    const json = JSON.stringify({ mcpServers: { 'storage-map': { command: info.command, args } } }, null, 2);
    const vscode = JSON.stringify({ servers: { 'storage-map': { type: 'stdio', command: info.command, args } } }, null, 2);
    const cli = `claude mcp add storage-map -- ${[info.command, ...args].map(quoteArg).join(' ')}`;
    const groups = NEEDS.map((need) => [need, info.tools.filter((x) => x.need === need)]);
    const active = (need) => ({
      always: true,
      read: RANK[c.level] >= 1,
      scan: RANK[c.level] >= 1 && c.allow_scan,
      write: RANK[c.level] >= 2,
      delete: RANK[c.level] >= 3,
      permanent: RANK[c.level] >= 3 && c.allow_permanent_delete,
    }[need]);
    body.append(h('div', { class: 'grid', style: { marginTop: '14px' } },
      h('div', { class: 'panel span-7' }, h('h2', null, t('mcp.setup')),
        h('p', { class: 'soft' }, t('mcp.setupText')),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, t('mcp.clients')),
        codeBlock(json),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, 'Claude Code'),
        codeBlock(cli),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, 'VS Code (.vscode/mcp.json)'),
        codeBlock(vscode),
        h('p', { class: 'muted', style: { marginTop: '10px' } }, t('mcp.settingsPath', { path: info.configPath })),
        h('p', { class: 'muted' }, t('mcp.auditPath', { path: info.auditPath }))),
      h('div', { class: 'panel span-5' }, h('h2', null, t('mcp.tools', { n: fmtNum(info.tools.length) })),
        groups.map(([need, tools]) => (tools.length ? h('div', { style: { marginBottom: '10px' } },
          h('div', { class: 'row' }, h('span', { class: 'label' }, t(`need.${need}`)), h('span', { class: 'tag' + (active(need) ? ' tag-on' : ''), style: { marginLeft: 'auto' } }, active(need) ? t('mcp.active') : t('mcp.inactive'))),
          tools.map((x) => h('div', { class: 'mono soft', style: { marginTop: '3px' }, title: toolText(x) }, x.name))) : null)))));
  }

  await draw();
  await loadAudit();
  const timer = setInterval(loadAudit, 4000);
  return () => clearInterval(timer);
}
