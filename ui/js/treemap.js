// Die Speicherkarte: jede Flaeche ist genau so gross wie der Platz, den sie belegt.
// Gezeichnet wird einmal in einen Puffer; Hover malt nur noch die Markierung darueber.

import { h, fmtBytes, fmtNum, fmtPct, fmtDate, cssVar, showTip, hideTip, tipBlock, catName } from './util.js';
import { ageColor } from './charts.js';
import { t, tn } from './i18n.js';

const K_FILE = 0, K_DIR = 1, K_REST = 2, K_UNKNOWN = 3, K_FREE = 4;
const HEADER = 17;

export function buildTree(data, { showFree = false } = {}) {
  const nodes = data.nodes.map((a) => ({
    id: a[0], parent: a[1], kind: a[2], alloc: a[3], count: a[4], cat: a[5], mtime: a[6], name: a[7], kids: [], up: null,
  }));
  nodes.forEach((n) => { if (n.parent >= 0) { nodes[n.parent].kids.push(n); n.up = nodes[n.parent]; } });
  const root = nodes[0];
  // Ketten aus Ordnern mit genau einem Unterordner werden eine Flaeche: "Docker › wsl › disk" statt drei Kopfzeilen.
  for (let i = nodes.length - 1; i > 0; i--) {
    const n = nodes[i];
    if (n.kind !== K_DIR) continue;
    while (n.kids.length === 1 && n.kids[0].kind === K_DIR) {
      const c = n.kids[0];
      n.name = `${n.name} › ${c.name}`;
      n.id = c.id;
      n.kids = c.kids;
      n.kids.forEach((k) => { k.up = n; });
    }
  }
  if (data.unaccounted > 0) root.kids.push({ id: -1, kind: K_UNKNOWN, alloc: data.unaccounted, name: t('map.notCaptured'), kids: [], up: root });
  if (showFree && data.free > 0) root.kids.push({ id: -2, kind: K_FREE, alloc: data.free, name: t('map.free'), kids: [], up: root });
  root.kids.sort((a, b) => b.alloc - a.alloc);
  root.total = root.kids.reduce((s, k) => s + k.alloc, 0) || root.alloc || 1;
  return root;
}

function squarify(kids, x, y, w, h, place) {
  const total = kids.reduce((s, k) => s + k.alloc, 0);
  if (total <= 0 || w <= 0 || h <= 0) return;
  const scale = (w * h) / total;
  let i = 0;
  while (i < kids.length) {
    const short = Math.min(w, h);
    let sum = 0, min = Infinity, max = 0, best = Infinity, j = i;
    while (j < kids.length) {
      const a = kids[j].alloc * scale;
      const s = sum + a, mn = Math.min(min, a), mx = Math.max(max, a);
      const ratio = Math.max((short * short * mx) / (s * s), (s * s) / (short * short * mn));
      if (ratio > best && j > i) break;
      best = ratio; sum = s; min = mn; max = mx; j++;
    }
    const thick = sum / short;
    if (w >= h) {
      let yy = y;
      for (let k = i; k < j; k++) { const hh = (kids[k].alloc * scale) / thick; place(kids[k], x, yy, thick, hh); yy += hh; }
      x += thick; w -= thick;
    } else {
      let xx = x;
      for (let k = i; k < j; k++) { const ww = (kids[k].alloc * scale) / thick; place(kids[k], xx, y, ww, thick); xx += ww; }
      y += thick; h -= thick;
    }
    i = j;
  }
}

export function layout(node, x, y, w, h, depth = 0) {
  node.x = x; node.y = y; node.w = w; node.h = h; node.depth = depth;
  node.header = false;
  if (!node.kids.length || w < 5 || h < 5) { node.laidOut = false; return; }
  node.laidOut = true;
  let ix = x, iy = y, iw = w, ih = h;
  if (depth > 0) {
    node.header = w >= 64 && h >= 34;
    const pad = w >= 24 && h >= 24 ? 1 : 0;
    ix += pad; iw -= pad * 2;
    iy += node.header ? HEADER : pad;
    ih -= (node.header ? HEADER : pad) + pad;
  }
  squarify(node.kids, ix, iy, iw, ih, (k, a, b, c, d) => layout(k, a, b, c, d, depth + 1));
}

const lum = (hex) => {
  const v = parseInt(hex.slice(1), 16);
  const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * f(v >> 16) + 0.7152 * f((v >> 8) & 255) + 0.0722 * f(v & 255);
};
const inkOn = (hex) => (lum(hex) > 0.42 ? '#0b0b0b' : '#ffffff');

function ageBin(mtime) {
  const days = (Date.now() / 1000 - mtime) / 86400;
  const lim = [30, 182, 365, 730, 1095, 1825, 3650];
  const i = lim.findIndex((l) => days < l);
  return i < 0 ? lim.length : i;
}

export class TreeMap {
  constructor(host, opts = {}) {
    this.host = host;
    this.opts = opts;
    this.canvas = h('canvas', { role: 'img', 'aria-label': t('map.aria') });
    host.append(this.canvas);
    this.ctx = this.canvas.getContext('2d');
    this.buf = document.createElement('canvas');
    this.mode = 'type';
    this.root = null;
    this.hover = null;
    this.sel = null;
    this.dimCat = null;
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(host);
    if (!opts.mini) {
      this.canvas.addEventListener('pointermove', (e) => this.onMove(e));
      this.canvas.addEventListener('pointerleave', () => { this.hover = null; hideTip(); this.paint(); });
      this.canvas.addEventListener('click', (e) => this.onClick(e));
      this.canvas.addEventListener('dblclick', (e) => this.onDbl(e));
      this.canvas.addEventListener('contextmenu', (e) => this.onCtx(e));
      this.canvas.addEventListener('wheel', (e) => this.onWheel(e), { passive: false });
    }
  }
  destroy() { this.ro.disconnect(); hideTip(); cancelAnimationFrame(this.raf); }
  readColors() {
    this.colors = {
      cats: [...Array(9).keys()].map((i) => cssVar(`--c${i}`)),
      folder: cssVar('--folder'), folder2: cssVar('--folder-2'), gap: cssVar('--map-gap'),
      ink: cssVar('--ink'), ink2: cssVar('--ink-2'), ink3: cssVar('--ink-3'), accent: cssVar('--accent'), line: cssVar('--line-2'),
      surface: cssVar('--surface'),
    };
    this.font = cssVar('--font');
  }
  setData(data, { anim, focus, showFree } = {}) {
    const prev = this.snapshot();
    this.data = data;
    this.root = buildTree(data, { showFree });
    this.hover = null;
    this.resize(true);
    if (anim && prev && !matchMedia('(prefers-reduced-motion: reduce)').matches) this.animate(prev, anim, focus);
  }
  setMode(m) { this.mode = m; this.render(); this.paint(); }
  setDim(cat) { this.dimCat = cat; this.render(); this.paint(); }
  setSelected(id) { this.sel = id; this.paint(); }
  snapshot() {
    if (!this.root || !this.buf.width) return null;
    const c = document.createElement('canvas');
    c.width = this.buf.width; c.height = this.buf.height;
    c.getContext('2d').drawImage(this.buf, 0, 0);
    return c;
  }
  resize(force) {
    const r = this.host.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const dpr = window.devicePixelRatio || 1;
    const W = Math.round(r.width * dpr), H = Math.round(r.height * dpr);
    if (!force && this.canvas.width === W && this.canvas.height === H) return;
    this.W = r.width; this.H = r.height; this.dpr = dpr;
    this.canvas.width = this.buf.width = W;
    this.canvas.height = this.buf.height = H;
    if (this.root) { layout(this.root, 0, 0, this.W, this.H); this.render(); this.paint(); }
  }
  fill(n) {
    const c = this.colors;
    if (n.kind === K_UNKNOWN) return c.folder2;
    if (n.kind === K_FREE) return c.surface;
    if (this.mode === 'age') {
      if (n.kind === K_REST || !n.mtime) return c.folder2;
      return ageColor(ageBin(n.mtime), 8);
    }
    return c.cats[n.cat ?? 8];
  }
  render() {
    if (!this.root) return;
    this.readColors();
    const g = this.buf.getContext('2d');
    g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    g.fillStyle = this.colors.gap;
    g.fillRect(0, 0, this.W, this.H);
    g.textBaseline = 'middle';
    this.drawNode(g, this.root);
  }
  drawNode(g, n) {
    const x0 = Math.round(n.x), y0 = Math.round(n.y), x1 = Math.round(n.x + n.w), y1 = Math.round(n.y + n.h);
    const w = x1 - x0, ht = y1 - y0;
    if (w <= 0 || ht <= 0) return;
    if (n.kind === K_DIR && n.laidOut) {
      if (n.depth > 0) {
        g.fillStyle = n.depth % 2 ? this.colors.folder : this.colors.folder2;
        g.fillRect(x0, y0, Math.max(1, w - 1), Math.max(1, ht - 1));
        if (n.header) {
          g.font = `600 11.5px ${this.font}`;
          g.fillStyle = this.colors.ink;
          const size = fmtBytes(n.alloc);
          const sw = g.measureText(size).width;
          const room = w - 12;
          g.save();
          g.beginPath(); g.rect(x0, y0, w - 1, HEADER); g.clip();
          if (room > sw + 40) {
            g.fillText(this.fit(g, n.name, room - sw - 8), x0 + 4, y0 + HEADER / 2 + 1);
            g.fillStyle = this.colors.ink2;
            g.font = `400 11px ${this.font}`;
            g.fillText(size, x1 - sw - 6, y0 + HEADER / 2 + 1);
          } else {
            g.fillText(this.fit(g, n.name, room), x0 + 4, y0 + HEADER / 2 + 1);
          }
          g.restore();
        }
      }
      for (const k of n.kids) this.drawNode(g, k);
      return;
    }
    // Blatt: Datei, nicht weiter aufgeklappter Ordner, Sammelblock oder Sonderflaeche
    const color = this.fill(n);
    const dimmed = this.dimCat !== null && n.cat !== this.dimCat;
    g.globalAlpha = dimmed ? 0.14 : n.kind === K_REST ? 0.55 : 1;
    g.fillStyle = color;
    g.fillRect(x0, y0, Math.max(1, w - 1), Math.max(1, ht - 1));
    // Leichte Woelbung, damit benachbarte gleichfarbige Flaechen auseinanderzuhalten sind.
    if (w > 5 && ht > 5 && n.kind < K_UNKNOWN) {
      g.fillStyle = 'rgba(255,255,255,.14)';
      g.fillRect(x0, y0, w - 1, 1);
      g.fillRect(x0, y0, 1, ht - 1);
      g.fillStyle = 'rgba(0,0,0,.16)';
      g.fillRect(x0, y1 - 2, w - 1, 1);
      g.fillRect(x1 - 2, y0, 1, ht - 1);
    }
    if (n.kind === K_UNKNOWN || n.kind === K_FREE) this.hatch(g, x0, y0, w - 1, ht - 1, n.kind);
    g.globalAlpha = 1;
    if (!dimmed && w >= 44 && ht >= 16) this.label(g, n, x0, y0, w - 1, ht - 1, color);
  }
  hatch(g, x, y, w, ht, kind) {
    g.save();
    g.beginPath(); g.rect(x, y, w, ht); g.clip();
    g.strokeStyle = kind === K_FREE ? this.colors.line : this.colors.ink3;
    g.globalAlpha = 0.35;
    g.lineWidth = 1;
    g.beginPath();
    for (let d = -ht; d < w; d += 9) { g.moveTo(x + d, y + ht); g.lineTo(x + d + ht, y); }
    g.stroke();
    g.restore();
  }
  label(g, n, x, y, w, ht, bg) {
    const special = n.kind === K_UNKNOWN || n.kind === K_FREE;
    const ink = special ? this.colors.ink : (n.kind === K_REST && this.mode !== 'age' ? this.colors.ink : inkOn(bg.startsWith('#') ? bg : '#808080'));
    const name = n.kind === K_REST ? t('map.smaller', { n: fmtNum(n.count) }) : n.name;
    g.save();
    g.beginPath(); g.rect(x, y, w, ht); g.clip();
    g.fillStyle = ink;
    const big = w > 150 && ht > 52;
    g.font = `${big ? 600 : 500} ${big ? 13.5 : 11.5}px ${this.font}`;
    const lh = big ? 17 : 14;
    g.fillText(this.fit(g, name, w - 8), x + 4, y + lh / 2 + 3);
    if (ht >= lh + 15) {
      g.font = `400 ${big ? 12 : 11}px ${this.font}`;
      g.globalAlpha = 0.86;
      g.fillText(fmtBytes(n.alloc), x + 4, y + lh + 10);
    }
    g.restore();
  }
  fit(g, s, w) {
    if (g.measureText(s).width <= w) return s;
    let a = 0, b = s.length;
    while (a < b) {
      const m = (a + b + 1) >> 1;
      if (g.measureText(s.slice(0, m) + '…').width <= w) a = m; else b = m - 1;
    }
    return a > 0 ? s.slice(0, a) + '…' : '';
  }
  paint() {
    if (!this.root || !this.buf.width) return;
    const g = this.ctx;
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.drawImage(this.buf, 0, 0);
    g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    if (this.sel !== null && this.sel !== undefined) {
      const n = this.find(this.root, this.sel);
      if (n) this.outline(g, n, this.colors.ink, 2);
    }
    if (this.hover) {
      if (this.hover.up && this.hover.up !== this.root) this.outline(g, this.hover.up, this.colors.ink2, 1);
      this.outline(g, this.hover, this.colors.accent, 2);
    }
  }
  outline(g, n, color, lw) {
    g.strokeStyle = color;
    g.lineWidth = lw;
    g.strokeRect(Math.round(n.x) + lw / 2, Math.round(n.y) + lw / 2, Math.max(1, Math.round(n.w) - lw - 1), Math.max(1, Math.round(n.h) - lw - 1));
  }
  find(n, id) {
    if (n.id === id && n.kind !== K_REST) return n;
    for (const k of n.kids) {
      const r = this.find(k, id);
      if (r) return r;
    }
    return null;
  }
  hit(n, x, y) {
    for (const k of n.kids) {
      if (x >= k.x && x < k.x + k.w && y >= k.y && y < k.y + k.h) {
        if (k.laidOut && !(k.header && y < k.y + HEADER)) return this.hit(k, x, y) || k;
        return k;
      }
    }
    return null;
  }
  pos(e) {
    const r = this.canvas.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }
  onMove(e) {
    if (!this.root) return;
    const [x, y] = this.pos(e);
    const n = this.hit(this.root, x, y);
    if (n !== this.hover) { this.hover = n; this.paint(); }
    if (!n) { hideTip(); this.canvas.style.cursor = ''; return; }
    this.canvas.style.cursor = n.kind === K_DIR ? 'zoom-in' : 'pointer';
    showTip(e.clientX, e.clientY, this.tip(n));
  }
  tip(n) {
    const total = this.root.total;
    const trail = [];
    for (let p = n.up; p && p !== this.root; p = p.up) trail.unshift(p.name);
    if (n.kind === K_UNKNOWN) return tipBlock(fmtBytes(n.alloc), n.name, t('map.unknownTip'));
    if (n.kind === K_FREE) return tipBlock(fmtBytes(n.alloc), n.name, fmtPct(n.alloc / total));
    if (n.kind === K_REST) return tipBlock(fmtBytes(n.alloc), t('map.smallerEntries', { n: fmtNum(n.count) }), t('map.restTip', { pct: fmtPct(n.alloc / total) }), trail.join(' › '));
    const sub = n.kind === K_DIR
      ? t('map.dirTip', { files: tn('unit.files', n.count), pct: fmtPct(n.alloc / total) })
      : t('map.fileTip', { cat: catName(n.cat), date: fmtDate(n.mtime), pct: fmtPct(n.alloc / total) });
    return tipBlock(fmtBytes(n.alloc), n.name, sub, trail.join(' › '));
  }
  onClick(e) {
    const [x, y] = this.pos(e);
    const n = this.root && this.hit(this.root, x, y);
    if (!n) return;
    if (n.kind === K_DIR && n.header && y < n.y + HEADER) { this.zoomTo(n); return; }
    if (n.id >= 0 && n.kind !== K_REST) { this.sel = n.id; this.paint(); this.opts.onSelect?.(n); }
    else if (n.kind === K_REST) { this.opts.onSelect?.(n.up); }
  }
  onDbl(e) {
    const [x, y] = this.pos(e);
    const n = this.root && this.hit(this.root, x, y);
    if (!n) return;
    if (n.kind === K_DIR) this.zoomTo(n);
    else if (n.up && n.up !== this.root) this.zoomTo(n.up);
  }
  onCtx(e) {
    e.preventDefault();
    const [x, y] = this.pos(e);
    const n = this.root && this.hit(this.root, x, y);
    if (n && n.id >= 0) this.opts.onContext?.(n.kind === K_REST ? n.up : n, e);
  }
  onWheel(e) {
    e.preventDefault();
    const now = performance.now();
    if (now - (this.lastWheel || 0) < 260) return;
    this.lastWheel = now;
    if (e.deltaY > 0) { this.opts.onUp?.(); return; }
    const [x, y] = this.pos(e);
    let n = this.hit(this.root, x, y);
    while (n && n.up && n.up !== this.root) n = n.up;
    if (n && n.kind === K_DIR) this.zoomTo(n);
  }
  zoomTo(n) {
    this.focus = { x: n.x, y: n.y, w: n.w, h: n.h };
    this.opts.onZoom?.(n);
  }
  animate(prev, dir, focus) {
    focus = focus || this.focus;
    if (!focus) return;
    const start = performance.now(), dur = 240;
    const { W, H, dpr } = this;
    const g = this.ctx;
    const ease = (t) => 1 - (1 - t) ** 3;
    const step = (now) => {
      const t = Math.min(1, (now - start) / dur), e = ease(t);
      g.setTransform(1, 0, 0, 1, 0, 0);
      if (dir === 'in') {
        // Der angeklickte Ordner waechst aus dem alten Bild heraus.
        const sx = W / focus.w, sy = H / focus.h;
        const k = 1 + (Math.min(sx, sy) - 1) * e;
        const dx = -focus.x * (k - 1), dy = -focus.y * (k - 1);
        g.globalAlpha = 1;
        g.drawImage(prev, dx * dpr, dy * dpr, W * k * dpr, H * k * dpr);
        g.globalAlpha = e;
        g.drawImage(this.buf, 0, 0);
      } else {
        g.globalAlpha = 1;
        g.drawImage(prev, 0, 0);
        g.globalAlpha = e;
        g.drawImage(this.buf, 0, 0);
      }
      g.globalAlpha = 1;
      if (t < 1) this.raf = requestAnimationFrame(step); else this.paint();
    };
    this.raf = requestAnimationFrame(step);
  }
}

export function ageLegend() {
  const stops = [...Array(8).keys()].map((i) => ageColor(i, 8));
  return h('span', { class: 'row' },
    h('span', null, t('map.new')),
    h('i', { class: 'ramp', style: { background: `linear-gradient(90deg, ${stops.join(',')})` } }),
    h('span', null, t('map.old')));
}
