// Ringansicht: von der Mitte nach aussen tiefer in die Ordner. Zeigt auf einen Blick, wie Platz verschachtelt ist.

import { h, fmtBytes, fmtNum, fmtPct, cssVar, showTip, hideTip, tipBlock } from './util.js';
import { buildTree } from './treemap.js';

const TAU = Math.PI * 2;

export class Sunburst {
  constructor(host, opts = {}) {
    this.host = host;
    this.opts = opts;
    this.canvas = h('canvas', { role: 'img', 'aria-label': 'Ringansicht der Ordner. Die gleichen Daten stehen in der Ordner-Ansicht als Tabelle.' });
    host.append(this.canvas);
    this.ctx = this.canvas.getContext('2d');
    this.arcs = [];
    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(host);
    this.canvas.addEventListener('pointermove', (e) => this.onMove(e));
    this.canvas.addEventListener('pointerleave', () => { this.hover = null; hideTip(); this.paint(); });
    this.canvas.addEventListener('click', (e) => this.onClick(e));
    this.canvas.addEventListener('contextmenu', (e) => this.onCtx(e));
  }
  destroy() { this.ro.disconnect(); hideTip(); }
  setData(data) {
    this.data = data;
    this.root = buildTree(data);
    this.rootName = data.ancestors[data.ancestors.length - 1]?.name || '';
    this.resize(true);
  }
  resize(force) {
    const r = this.host.getBoundingClientRect();
    if (!r.width || !r.height) return;
    const dpr = window.devicePixelRatio || 1;
    const W = Math.round(r.width * dpr), H = Math.round(r.height * dpr);
    if (!force && this.canvas.width === W) return;
    this.W = r.width; this.H = r.height; this.dpr = dpr;
    this.canvas.width = W; this.canvas.height = H;
    this.layout();
    this.paint();
  }
  layout() {
    if (!this.root) return;
    this.cx = this.W / 2;
    this.cy = this.H / 2;
    this.R = Math.min(this.W, this.H) / 2 - 10;
    this.r0 = Math.max(60, this.R * 0.19);
    this.rings = this.opts.rings || 6;
    this.thick = (this.R - this.r0) / this.rings;
    this.arcs = [];
    const walk = (n, a0, a1, d) => {
      if (d >= this.rings) return;
      let a = a0;
      const tot = n.kids.reduce((s, k) => s + k.alloc, 0) || 1;
      // Bei Ordnern fuellen die Kinder den Bogen des Ordners vollstaendig aus.
      for (const k of n.kids) {
        const da = ((a1 - a0) * k.alloc) / tot;
        if (da > 0.002) {
          this.arcs.push({ n: k, a0: a, a1: a + da, d });
          walk(k, a, a + da, d + 1);
        }
        a += da;
      }
    };
    walk(this.root, -Math.PI / 2, -Math.PI / 2 + TAU, 0);
  }
  paint() {
    if (!this.root) return;
    const g = this.ctx;
    g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    g.clearRect(0, 0, this.W, this.H);
    const cats = [...Array(9).keys()].map((i) => cssVar(`--c${i}`));
    const surface = cssVar('--map-gap');
    const unknown = cssVar('--folder-2');
    for (const a of this.arcs) {
      const dim = this.hover && this.hover !== a && !this.isAncestor(a.n, this.hover.n);
      g.globalAlpha = dim ? 0.38 : (a.n.kind === 2 ? 0.5 : 1);
      g.beginPath();
      const r1 = this.r0 + a.d * this.thick, r2 = r1 + this.thick - 1.5;
      g.arc(this.cx, this.cy, r2, a.a0, a.a1);
      g.arc(this.cx, this.cy, r1, a.a1, a.a0, true);
      g.closePath();
      g.fillStyle = a.n.kind === 3 ? unknown : cats[a.n.cat ?? 8];
      g.fill();
      g.strokeStyle = surface;
      g.lineWidth = 1;
      g.stroke();
    }
    g.globalAlpha = 1;
    // Mitte
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    const font = cssVar('--font');
    g.fillStyle = cssVar('--ink');
    const show = this.hover?.n || null;
    const title = show ? (show.kind === 2 ? `${fmtNum(show.count)} kleinere` : show.name) : this.rootName;
    const val = show ? show.alloc : this.data.alloc;
    g.font = `600 ${Math.max(18, this.r0 * 0.34)}px ${font}`;
    g.fillText(fmtBytes(val), this.cx, this.cy - 6);
    g.font = `400 12px ${font}`;
    g.fillStyle = cssVar('--ink-2');
    g.fillText(this.fit(g, title, this.r0 * 1.7), this.cx, this.cy + this.r0 * 0.28);
    if (!show && this.data.ancestors.length > 1) {
      g.fillStyle = cssVar('--ink-3');
      g.font = `400 11px ${font}`;
      g.fillText('Mitte: eine Ebene höher', this.cx, this.cy + this.r0 * 0.56);
    }
    g.textAlign = 'start';
  }
  fit(g, s, w) {
    if (g.measureText(s).width <= w) return s;
    let t = s;
    while (t.length > 2 && g.measureText(t + '…').width > w) t = t.slice(0, -1);
    return t + '…';
  }
  isAncestor(n, of) {
    for (let p = of; p; p = p.up) if (p === n) return true;
    return false;
  }
  hitTest(e) {
    const r = this.canvas.getBoundingClientRect();
    const x = e.clientX - r.left - this.cx, y = e.clientY - r.top - this.cy;
    const dist = Math.hypot(x, y);
    if (dist < this.r0) return 'center';
    const d = Math.floor((dist - this.r0) / this.thick);
    let ang = Math.atan2(y, x);
    if (ang < -Math.PI / 2) ang += TAU;
    return this.arcs.find((a) => a.d === d && ang >= a.a0 && ang < a.a1) || null;
  }
  onMove(e) {
    const a = this.hitTest(e);
    const next = a && a !== 'center' ? a : null;
    if (next !== this.hover) { this.hover = next; this.paint(); }
    this.canvas.style.cursor = a === 'center' ? (this.data.ancestors.length > 1 ? 'zoom-out' : '') : next && next.n.kind === 1 ? 'zoom-in' : next ? 'pointer' : '';
    if (!next) { hideTip(); return; }
    const n = next.n;
    const trail = [];
    for (let p = n.up; p && p !== this.root; p = p.up) trail.unshift(p.name);
    showTip(e.clientX, e.clientY, tipBlock(fmtBytes(n.alloc), n.kind === 2 ? `${fmtNum(n.count)} kleinere Einträge` : n.name,
      `${fmtPct(n.alloc / this.root.total)}`, trail.join(' › ')));
  }
  onClick(e) {
    const a = this.hitTest(e);
    if (a === 'center') { this.opts.onUp?.(); return; }
    if (!a) return;
    if (a.n.kind === 1 && a.n.id >= 0) this.opts.onZoom?.(a.n);
    else if (a.n.kind === 0) this.opts.onSelect?.(a.n);
  }
  onCtx(e) {
    e.preventDefault();
    const a = this.hitTest(e);
    if (a && a !== 'center' && a.n.id >= 0) this.opts.onContext?.(a.n.kind === 2 ? a.n.up : a.n, e);
  }
}
