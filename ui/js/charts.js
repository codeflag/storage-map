// Diagramme: duenne Markierungen, feine Linien, Werte im Tooltip. Text traegt nie die Datenfarbe.

import { h, icon, withTip, tipBlock, fmtBytes, fmtPct, showTip, hideTip, catVar, catInk } from './util.js';
import { t, tn } from './i18n.js';

const NS = 'http://www.w3.org/2000/svg';
const svg = (tag, attrs = {}, ...kids) => {
  const el = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  kids.forEach((c) => el.append(c));
  return el;
};

/// Ein Ganzes, aufgeteilt in Teile. Immer mit Legende; zu schmale Teile zeigen ihren Wert nur im Tooltip.
export function segBar(parts, { total, onClick } = {}) {
  const sum = total || parts.reduce((a, p) => a + p.value, 0) || 1;
  const shown = parts.filter((p) => p.value > 0);
  const bar = h('div', { class: 'segbar', role: 'img', 'aria-label': shown.map((p) => `${p.name} ${fmtBytes(p.value)}`).join(', ') },
    shown.map((p) => {
      const share = p.value / sum;
      const el = h('div', {
        class: 'part', tabindex: 0,
        style: { flex: `${p.value} 1 0`, background: p.color, color: p.ink || '#fff' },
        onclick: () => onClick && onClick(p),
      }, share >= 0.07 ? fmtPct(share) : null);
      return withTip(el, () => tipBlock(fmtBytes(p.value), p.name, `${fmtPct(share)}${p.sub ? ' · ' + p.sub : ''}`));
    }));
  const legend = h('div', { class: 'legend' }, shown.map((p) =>
    h(onClick ? 'button' : 'span', { onclick: onClick ? () => onClick(p) : undefined },
      h('i', { class: 'swatch', style: { background: p.color } }), p.name)));
  return h('div', null, bar, legend);
}

export function catParts(cats) {
  return [...cats].sort((a, b) => b.alloc - a.alloc).map((c) => ({
    id: c.id, name: c.name, value: c.alloc, color: catVar(c.id), ink: catInk(c.id),
    sub: tn('unit.files', c.files),
  }));
}

/// Rangliste: Name, Wert, schmaler Balken.
export function rankBars(rows, { name, path, value, color, onClick, onContext, max, fmt = fmtBytes, total }) {
  const top = max || Math.max(...rows.map(value), 1);
  return h('div', { class: 'ranks' }, rows.map((r) => {
    const v = value(r);
    const el = h('button', {
      class: 'rank', onclick: () => onClick && onClick(r),
      oncontextmenu: onContext ? (e) => { e.preventDefault(); onContext(r, e); } : undefined,
    },
      h('div', { class: 'n' }, name(r)),
      h('div', { class: 'val' }, fmt(v)),
      path ? h('div', { class: 'p', style: { gridColumn: '1 / -1' }, title: path(r) }, '‎' + path(r)) : null,
      h('div', { class: 'bar', style: { width: `${Math.max(0.5, (v / top) * 100)}%`, background: color ? color(r) : 'var(--c0)' } }));
    return total ? withTip(el, () => tipBlock(fmt(v), name(r), t('chart.ofTotal', { pct: fmtPct(v / total) }), path ? path(r) : null)) : el;
  }));
}

function niceMax(v) {
  if (v <= 0) return 1;
  const units = 1024 ** Math.floor(Math.log(v) / Math.log(1024));
  const x = v / units;
  const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000];
  const s = steps.find((a) => a >= x) || 1024;
  return s * units;
}

/// Zeichnet in der wirklichen Breite des Behaelters neu, damit die Schrift nicht mitskaliert.
function responsive(draw) {
  const box = h('div', { class: 'chart' });
  let last = 0;
  const ro = new ResizeObserver(() => {
    if (!box.isConnected) { ro.disconnect(); return; }
    const w = Math.round(box.getBoundingClientRect().width);
    if (w && Math.abs(w - last) > 3) { last = w; box.replaceChildren(draw(w)); }
  });
  ro.observe(box);
  return box;
}

/// Saeulen auf einer Grundlinie. Geordnete Klassen (Alter) bekommen die einfarbige Stufenskala, sonst eine Farbe.
export function columns(rows, { value = (r) => r.alloc, label = (r) => r.label, color, sub, onClick, height = 170 }) {
  return responsive((W) => {
    const H = height, L = 50, B = 22, T = 8;
    const top = niceMax(Math.max(...rows.map(value), 1));
    const n = rows.length;
    const band = (W - L) / n;
    const bw = Math.min(24, band * 0.72);
    // Nur so viele Beschriftungen, wie nebeneinander passen.
    const longest = Math.max(...rows.map((r) => String(label(r)).length));
    const every = Math.max(1, Math.ceil((longest * 6.4 + 6) / band));
    const root = svg('svg', { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'img' });
    for (let i = 0; i <= 2; i++) {
      const y = T + ((H - T - B) * i) / 2;
      if (i < 2) root.append(svg('line', { x1: L, x2: W, y1: y, y2: y, class: 'grid-line' }));
      const tick = svg('text', { x: L - 6, y: y + 4, 'text-anchor': 'end' });
      tick.textContent = fmtBytes(top * (1 - i / 2)).replace(/[.,]00(?= )/, '');
      root.append(tick);
    }
    root.append(svg('line', { x1: L, x2: W, y1: H - B, y2: H - B, class: 'axis-line' }));
    rows.forEach((r, i) => {
      const v = value(r);
      const bh = Math.max(v > 0 ? 2 : 0, (v / top) * (H - T - B));
      const x = L + band * i + (band - bw) / 2;
      const y = H - B - bh;
      const rad = Math.min(4, bh, bw / 2);
      const d = `M${x},${H - B}V${y + rad}Q${x},${y} ${x + rad},${y}H${x + bw - rad}Q${x + bw},${y} ${x + bw},${y + rad}V${H - B}Z`;
      const hit = svg('rect', { x: L + band * i, y: T, width: band, height: H - T - B, fill: 'transparent', class: 'col', tabindex: 0 });
      const col = svg('path', { d, fill: color ? color(r, i) : 'var(--c0)', 'pointer-events': 'none' });
      root.append(col, hit);
      const tip = () => tipBlock(fmtBytes(v), r.label || String(label(r)), sub ? sub(r) : null);
      hit.addEventListener('pointermove', (e) => { col.style.filter = 'brightness(1.25)'; showTip(e.clientX, e.clientY, tip()); });
      hit.addEventListener('pointerleave', () => { col.style.filter = ''; hideTip(); });
      hit.addEventListener('focus', () => { const b = hit.getBoundingClientRect(); showTip(b.left, b.top, tip()); });
      hit.addEventListener('blur', hideTip);
      if (onClick) hit.addEventListener('click', () => onClick(r));
      if (i % every === 0) {
        const text = svg('text', { x: L + band * i + band / 2, y: H - 7, 'text-anchor': 'middle' });
        text.textContent = label(r);
        root.append(text);
      }
    });
    return root;
  });
}

/// Stufenskala fuer geordnete Klassen, von "neu" bis "alt".
export function ageColor(i, count) {
  const dark = document.documentElement.dataset.resolvedTheme === 'dark';
  const ramp = dark
    ? ['#104281', '#184f95', '#256abf', '#3987e5', '#5598e7', '#6da7ec', '#86b6ef', '#b7d3f6']
    : ['#86b6ef', '#6da7ec', '#5598e7', '#3987e5', '#2a78d6', '#1c5cab', '#104281', '#0d366b'];
  return ramp[Math.round((i / Math.max(1, count - 1)) * (ramp.length - 1))];
}

/// Eine Linie ueber die Zeit, mit Fadenkreuz, das immer auf den naechsten Messpunkt springt.
export function lineChart(points, { fmtY = fmtBytes, fmtX, height = 220, limit, limitLabel }) {
  return responsive((W) => {
    const H = height, L = 62, R = 12, T = 12, B = 24;
    const xs = points.map((p) => p.x), ys = points.map((p) => p.y);
    const x0 = Math.min(...xs), x1 = Math.max(...xs);
    let y0 = Math.min(...ys), y1 = Math.max(...ys);
    if (limit) y1 = Math.max(y1, limit);
    const pad = Math.max((y1 - y0) * 0.12, 1);
    y0 = Math.max(0, y0 - pad); y1 += limit ? 0 : pad;
    const sx = (v) => L + ((v - x0) / Math.max(1, x1 - x0)) * (W - L - R);
    const sy = (v) => T + (1 - (v - y0) / Math.max(1, y1 - y0)) * (H - T - B);
    const root = svg('svg', { viewBox: `0 0 ${W} ${H}`, width: W, height: H, role: 'img' });
    for (let i = 0; i <= 3; i++) {
      const v = y0 + ((y1 - y0) * i) / 3;
      root.append(svg('line', { x1: L, x2: W - R, y1: sy(v), y2: sy(v), class: i === 0 ? 'axis-line' : 'grid-line' }));
      const tick = svg('text', { x: L - 6, y: sy(v) + 4, 'text-anchor': 'end' });
      tick.textContent = fmtY(v);
      root.append(tick);
    }
    [0, Math.floor((points.length - 1) / 2), points.length - 1].forEach((i) => {
      const tick = svg('text', { x: sx(xs[i]), y: H - 6, 'text-anchor': i === 0 ? 'start' : i === points.length - 1 ? 'end' : 'middle' });
      tick.textContent = fmtX(xs[i]);
      root.append(tick);
    });
    if (limit) {
      root.append(svg('line', { x1: L, x2: W - R, y1: sy(limit), y2: sy(limit), stroke: 'var(--critical)', 'stroke-width': 1 }));
      const text = svg('text', { x: W - R, y: sy(limit) + 14, 'text-anchor': 'end' });
      text.textContent = limitLabel || t('chart.capacity');
      root.append(text);
    }
    const path = points.map((p, i) => `${i ? 'L' : 'M'}${sx(p.x)},${sy(p.y)}`).join('');
    root.append(svg('path', { d: `${path}L${sx(x1)},${sy(y0)}L${sx(x0)},${sy(y0)}Z`, fill: 'var(--c0)', opacity: .1 }));
    root.append(svg('path', { d: path, fill: 'none', stroke: 'var(--c0)', 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }));
    const last = points[points.length - 1];
    root.append(svg('circle', { cx: sx(last.x), cy: sy(last.y), r: 4, fill: 'var(--c0)', stroke: 'var(--surface)', 'stroke-width': 2 }));
    const cross = svg('line', { y1: T, y2: H - B, stroke: 'var(--ink-3)', 'stroke-width': 1, visibility: 'hidden' });
    const dot = svg('circle', { r: 4, fill: 'var(--c0)', stroke: 'var(--surface)', 'stroke-width': 2, visibility: 'hidden' });
    root.append(cross, dot);
    const hit = svg('rect', { x: L, y: T, width: W - L - R, height: H - T - B, fill: 'transparent' });
    root.append(hit);
    hit.addEventListener('pointermove', (e) => {
      const b = root.getBoundingClientRect();
      const vx = e.clientX - b.left;
      let best = points[0];
      points.forEach((p) => { if (Math.abs(sx(p.x) - vx) < Math.abs(sx(best.x) - vx)) best = p; });
      cross.setAttribute('x1', sx(best.x)); cross.setAttribute('x2', sx(best.x)); cross.setAttribute('visibility', 'visible');
      dot.setAttribute('cx', sx(best.x)); dot.setAttribute('cy', sy(best.y)); dot.setAttribute('visibility', 'visible');
      showTip(e.clientX, e.clientY, tipBlock(fmtY(best.y), fmtX(best.x), best.sub));
    });
    hit.addEventListener('pointerleave', () => { cross.setAttribute('visibility', 'hidden'); dot.setAttribute('visibility', 'hidden'); hideTip(); });
    return root;
  });
}

/// Winzige Verlaufslinie ohne Achsen.
export function sparkline(values, { width = 120, height = 28 } = {}) {
  if (values.length < 2) return null;
  const a = Math.min(...values), b = Math.max(...values);
  const p = values.map((v, i) => `${i ? 'L' : 'M'}${(i / (values.length - 1)) * (width - 6) + 1},${(1 - (v - a) / Math.max(1, b - a)) * (height - 6) + 3}`).join('');
  const lastY = (1 - (values[values.length - 1] - a) / Math.max(1, b - a)) * (height - 6) + 3;
  const s = svg('svg', { viewBox: `0 0 ${width} ${height}`, width, height },
    svg('path', { d: p, fill: 'none', stroke: 'var(--ink-3)', 'stroke-width': 1.5, 'stroke-linejoin': 'round' }),
    svg('circle', { cx: width - 5, cy: lastY, r: 3, fill: 'var(--c0)' }));
  return s;
}

/// Plus und Minus um eine neutrale Mitte.
export function divergingRows(rows, { name, delta, onClick }) {
  const top = Math.max(...rows.map((r) => Math.abs(delta(r))), 1);
  return h('div', null, rows.map((r) => {
    const d = delta(r);
    const w = (Math.abs(d) / top) * 50;
    return h('div', { class: 'div-row' },
      name(r),
      h('div', { class: 'div-track' }, h('i', { class: d >= 0 ? 'up' : 'down', style: { width: `${Math.max(w, 0.8)}%` } })),
      h('div', { class: 'num right', style: { fontWeight: 600 } }, (d >= 0 ? '+' : '−') + fmtBytes(Math.abs(d))));
  }));
}

export function severity(share) {
  if (share >= 0.95) return { color: 'var(--critical)', label: t('state.critical'), icon: 'warn' };
  if (share >= 0.9) return { color: 'var(--serious)', label: t('state.veryFull'), icon: 'warn' };
  if (share >= 0.8) return { color: 'var(--warning)', label: t('state.tight'), icon: 'warn' };
  return { color: 'var(--c0)', label: t('state.enough'), icon: 'check' };
}
export function stateBadge(share) {
  const s = severity(share);
  const ic = icon(s.icon);
  ic.style.color = s.color;
  return h('span', { class: 'state' }, ic, s.label);
}
