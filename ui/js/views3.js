// KI-Zugriff: Rechte des MCP-Servers, Einrichtung und Protokoll.

import { api, h, icon, clear, fill, App, fmtDateTime, fmtNum, fail, toast, dialog } from './util.js';

const LEVELS = [
  { id: 'off', name: 'Aus', text: 'Der Assistent bekommt keine Daten.' },
  { id: 'read', name: 'Lesen', text: 'Auswertungen abrufen: Laufwerke, Ordner, Dateinamen, Größen, Duplikate, Aufräum-Vorschläge. Nichts wird verändert.' },
  { id: 'write', name: 'Schreiben', text: 'Zusätzlich: die Aufräumliste bearbeiten und Dateien verschieben. Gelöscht wird nichts.' },
  { id: 'delete', name: 'Löschen', text: 'Zusätzlich: Dateien in den Papierkorb legen.' },
];
const RANK = { off: 0, read: 1, write: 2, delete: 3 };
export const LEVEL_NAME = Object.fromEntries(LEVELS.map((l) => [l.id, l.name]));

const NEED = {
  always: 'Immer verfügbar',
  read: 'Lesen',
  scan: 'Scans starten (eigene Freigabe)',
  write: 'Schreiben',
  delete: 'Löschen',
  permanent: 'Endgültig löschen (eigene Freigabe)',
};

function save(body) {
  return api('mcp/config', body).then(() => App.poll());
}

/// Fragt vor mehr Rechten nach. Das Bestätigungswort prüft auch das Backend.
function confirmRaise({ title, risks, word, label, body, done }) {
  const input = h('input', {
    class: 'field', placeholder: word, 'aria-label': 'Bestätigungswort', style: { width: '100%' }, autocomplete: 'off',
    oninput: () => { document.getElementById('mcp-go').disabled = input.value !== word; },
  });
  dialog({
    title,
    body: [
      h('div', { class: 'notice block' }, icon('warn'), h('div', null, h('b', null, 'Ein KI-Assistent kann sich irren. '), 'Er kann auch durch Dateinamen in die Irre geführt werden, denn die liest er als Text.')),
      h('ul', { class: 'plain-list' }, risks.map((r) => h('li', null, r))),
      h('p', { class: 'soft' }, 'Systemordner bleiben auf jeder Stufe gesperrt, und seit dem Scan geänderte Dateien bleiben liegen. Jeder Aufruf steht im Protokoll.'),
      h('label', null, 'Tippe zur Bestätigung ', h('b', null, word), ':'),
      input,
    ],
    actions: [
      { label: 'Abbrechen' },
      { label, kind: 'danger', id: 'mcp-go', disabled: true, run: () => save({ ...body, confirm: input.value }).then(done).catch(fail) },
    ],
  });
}

function askLevel(id, current, done) {
  if (RANK[id] <= RANK[current]) { save({ level: id }).then(done).catch(fail); return; }
  if (id === 'read') { save({ level: id }).then(done).catch(fail); return; }
  if (id === 'write') {
    confirmRaise({
      title: 'Schreibrechte erlauben?',
      word: 'SCHREIBEN',
      label: 'Schreiben erlauben',
      body: { level: 'write' },
      done,
      risks: ['Der Assistent kann die Aufräumliste ändern, die du hier siehst.', 'Er kann Dateien in andere Ordner verschieben. Programme finden sie dann nicht mehr. Überschrieben wird nichts.'],
    });
    return;
  }
  confirmRaise({
    title: 'Löschrechte erlauben?',
    word: 'LÖSCHEN',
    label: 'Löschen erlauben',
    body: { level: 'delete' },
    done,
    risks: ['Der Assistent kann Dateien und ganze Ordner in den Papierkorb legen, ohne dass du jeden einzelnen Schritt siehst.', 'Aus dem Papierkorb lässt sich alles wiederherstellen, solange er nicht geleert wurde.', 'Netzlaufwerke haben keinen Papierkorb. Dort schlägt das Löschen fehl.'],
  });
}

function codeBlock(text) {
  return h('div', { class: 'code-block' },
    h('pre', { class: 'mono' }, text),
    h('button', { class: 'btn small', onclick: () => navigator.clipboard.writeText(text).then(() => toast('Kopiert.')).catch(fail) }, icon('copy'), 'Kopieren'));
}

function quoteArg(a) {
  return /[\s"]/.test(a) ? `"${a.replaceAll('"', '\\"')}"` : a;
}

function auditTable(rows) {
  if (!rows.length) return h('div', { class: 'empty' }, 'Noch keine Aufrufe.');
  const label = { ok: 'erledigt', denied: 'verweigert', error: 'Fehler' };
  return h('div', { style: { overflow: 'auto', maxHeight: '420px' } }, h('table', { class: 'tbl' },
    h('thead', null, h('tr', null, h('th', null, 'Zeit'), h('th', null, 'Client'), h('th', null, 'Werkzeug'), h('th', null, 'Angaben'), h('th', null, 'Ergebnis'))),
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
      h('h1', { style: { margin: 0 } }, 'KI-Zugriff'),
      S.status?.index ? null : h('button', { class: 'link more', style: { marginLeft: 'auto' }, onclick: () => { location.hash = ''; App.poll().then(() => App.home()); } }, 'Zurück zur Laufwerkswahl')),
    h('p', { class: 'lead' }, 'Über das Model Context Protocol (MCP) kann ein KI-Assistent die Auswertungen von Storage Map nutzen. Hier legst du fest, was er darf. Das gilt sofort, und der Assistent kann es nicht selbst ändern.'),
    body,
    h('div', { class: 'panel', style: { marginTop: '14px' } }, h('div', { class: 'panel-head' }, h('div', { class: 'panel-title' }, 'Protokoll'),
      h('button', { class: 'link more', onclick: () => loadAudit() }, 'Aktualisieren')), audit)));

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
        h('b', null, 'Die Einstellungsdatei ist beschädigt, deshalb ist der Zugriff aus. '), `Speichern überschreibt sie. Fehler: ${info.error}`)));
    }

    body.append(h('div', { class: 'levels', role: 'radiogroup', 'aria-label': 'Zugriffsstufe' }, LEVELS.map((l) => h('button', {
      class: 'level' + (c.level === l.id ? ' on' : '') + (RANK[l.id] >= 2 ? ' risky' : ''),
      role: 'radio', 'aria-checked': c.level === l.id ? 'true' : 'false',
      onclick: () => { if (c.level !== l.id) askLevel(l.id, c.level, draw); },
    }, h('b', null, l.name), h('span', { class: 'soft' }, l.text)))));

    if (c.level !== 'off') {
      body.append(h('div', { class: 'notice info', style: { marginTop: '12px' } }, icon('info'), h('div', null,
        'Dateinamen und Pfade gehen an den Assistenten und damit in der Regel an dessen Anbieter. Dateiinhalte gibt Storage Map nie heraus.')));
    }

    const scanBox = h('input', { type: 'checkbox', checked: c.allow_scan, disabled: c.level === 'off', onchange: (e) => save({ allowScan: e.target.checked }).then(draw).catch(fail) });
    const permBox = h('input', {
      type: 'checkbox', checked: c.allow_permanent_delete, disabled: c.level !== 'delete',
      onchange: (e) => {
        if (!e.target.checked) { save({ allowPermanent: false }).then(draw).catch(fail); return; }
        e.target.checked = false;
        confirmRaise({
          title: 'Endgültiges Löschen erlauben?',
          word: 'LÖSCHEN',
          label: 'Endgültig löschen erlauben',
          body: { allowPermanent: true },
          done: draw,
          risks: ['Der Assistent kann Dateien sofort und ohne Papierkorb entfernen.', 'Das lässt sich nicht rückgängig machen.'],
        });
      },
    });
    const known = drives.drives.map((d) => d.root).concat(drives.folders.map((f) => f.root));
    const allShown = c.roots.length === 0;
    const setRoots = (roots) => save({ roots }).then(draw).catch(fail);
    body.append(h('div', { class: 'grid', style: { marginTop: '14px' } },
      h('div', { class: 'panel span-6' }, h('h2', null, 'Zusätzliche Freigaben'),
        h('label', { class: 'check' }, scanBox, h('span', null, h('b', null, 'Scans und Duplikat-Suche starten'), h('span', { class: 'muted block-line' }, 'Sonst sieht der Assistent nur, was du selbst gescannt hast. Die Duplikat-Suche liest Dateien, um sie zu vergleichen. Ihr Inhalt verlässt das Programm nicht.'))),
        h('label', { class: 'check' }, permBox, h('span', null, h('b', null, 'Endgültig löschen'), h('span', { class: 'muted block-line' }, c.level === 'delete' ? 'Ohne Papierkorb. Nicht umkehrbar.' : 'Nur auf der Stufe „Löschen“ möglich.')))),
      h('div', { class: 'panel span-6' }, h('h2', null, 'Sichtbare Laufwerke'),
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: allShown, onchange: (e) => setRoots(e.target.checked ? [] : known) }), h('span', null, h('b', null, 'Alle'))),
        allShown ? null : known.map((r) => h('label', { class: 'check' },
          h('input', { type: 'checkbox', checked: c.roots.includes(r), onchange: (e) => setRoots(e.target.checked ? [...c.roots, r] : c.roots.filter((x) => x !== r)) }),
          h('span', { class: 'mono' }, r))))));

    const args = info.args;
    const json = JSON.stringify({ mcpServers: { 'storage-map': { command: info.command, args } } }, null, 2);
    const vscode = JSON.stringify({ servers: { 'storage-map': { type: 'stdio', command: info.command, args } } }, null, 2);
    const cli = `claude mcp add storage-map -- ${[info.command, ...args].map(quoteArg).join(' ')}`;
    const groups = Object.keys(NEED).map((need) => [need, info.tools.filter((t) => t.need === need)]);
    const active = (need) => ({
      always: true,
      read: RANK[c.level] >= 1,
      scan: RANK[c.level] >= 1 && c.allow_scan,
      write: RANK[c.level] >= 2,
      delete: RANK[c.level] >= 3,
      permanent: RANK[c.level] >= 3 && c.allow_permanent_delete,
    }[need]);
    body.append(h('div', { class: 'grid', style: { marginTop: '14px' } },
      h('div', { class: 'panel span-7' }, h('h2', null, 'Einrichten'),
        h('p', { class: 'soft' }, 'Der Assistent startet Storage Map selbst, das Fenster muss dafür nicht offen sein. Trage den Server in seine Einstellungen ein:'),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, 'Claude Desktop, Cursor und ähnliche'),
        codeBlock(json),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, 'Claude Code'),
        codeBlock(cli),
        h('div', { class: 'label', style: { margin: '10px 0 4px' } }, 'VS Code (.vscode/mcp.json)'),
        codeBlock(vscode),
        h('p', { class: 'muted', style: { marginTop: '10px' } }, `Einstellungen: ${info.configPath}`),
        h('p', { class: 'muted' }, `Protokoll: ${info.auditPath}`)),
      h('div', { class: 'panel span-5' }, h('h2', null, `Werkzeuge (${fmtNum(info.tools.length)})`),
        groups.map(([need, tools]) => (tools.length ? h('div', { style: { marginBottom: '10px' } },
          h('div', { class: 'row' }, h('span', { class: 'label' }, NEED[need]), h('span', { class: 'tag' + (active(need) ? ' tag-on' : ''), style: { marginLeft: 'auto' } }, active(need) ? 'aktiv' : 'aus')),
          tools.map((t) => h('div', { class: 'mono soft', style: { marginTop: '3px' }, title: t.description }, t.name))) : null)))));
  }

  await draw();
  await loadAudit();
  const timer = setInterval(loadAudit, 4000);
  return () => clearInterval(timer);
}
