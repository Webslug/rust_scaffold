/* Small SVG chart library, no dependencies (25 KB readable, 7 KB gzipped).
 *
 *   <div class="chart" data-chart="column" data-config='{"labels":[...],"series":[...]}'
 *        aria-label="Plain-English summary of the chart"></div>
 *
 * Types: column, stacked, hbar, line, area, pie, donut, ring, heatmap, spark.
 * See docs/COMPONENT-LIBRARY.md for each type's config. Charts redraw to the
 * width of their container. Colours come from CSS tokens (charts.css), so a
 * theme switch recolours them with no redraw.
 *
 * Built to the dataviz rules: series colours in a fixed order, thin marks
 * with 4px rounded data ends, 2px gaps instead of borders, hairline solid
 * gridlines, a legend for two or more series, one tooltip that lists every
 * series, the same readout on keyboard focus (arrow keys), and a table view
 * twin behind every chart's [data-chart-table-toggle] button.
 */
(function () {
  "use strict";

  var NS = "http://www.w3.org/2000/svg";
  var compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
  var full = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
  var CHAR = 6.6; // average glyph width at 12px, used to size label gutters
  var GAP = 2;    // surface gap between touching marks, in px
  var tip;        // the single tooltip element

  /* ---------- helpers ---------- */

  function svg(tag, attrs, parent) {
    var e = document.createElementNS(NS, tag);
    Object.keys(attrs || {}).forEach(function (k) { e.setAttribute(k, attrs[k]); });
    if (parent) { parent.appendChild(e); }
    return e;
  }

  function html(tag, cls, text, parent) {
    var e = document.createElement(tag);
    if (cls) { e.className = cls; }
    if (text != null) { e.textContent = text; }   // labels are data: textContent only
    if (parent) { parent.appendChild(e); }
    return e;
  }

  function fmt(cfg, v, short) {
    return (cfg.prefix || "") + (short ? compact : full).format(v) + (cfg.suffix || "");
  }

  function niceScale(max) {
    if (!(max > 0)) { return { max: 1, step: 0.25 }; }
    var raw = max / 4, mag = Math.pow(10, Math.floor(Math.log10(raw))), n = raw / mag;
    var step = (n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10) * mag;
    return { max: Math.ceil(max / step) * step, step: step };
  }

  // Bar with a 4px rounded data end and a square base.
  function barPath(x, y, w, h, r, dir) {
    r = Math.max(0, Math.min(r, w / 2, h));
    if (dir === "right") {
      return "M" + x + "," + y + "H" + (x + w - r) + "A" + r + "," + r + " 0 0 1 " + (x + w) + "," + (y + r) +
        "V" + (y + h - r) + "A" + r + "," + r + " 0 0 1 " + (x + w - r) + "," + (y + h) + "H" + x + "Z";
    }
    if (dir === "none") { return "M" + x + "," + y + "h" + w + "v" + h + "h" + (-w) + "Z"; }
    return "M" + x + "," + (y + h) + "V" + (y + r) + "A" + r + "," + r + " 0 0 1 " + (x + r) + "," + y +
      "H" + (x + w - r) + "A" + r + "," + r + " 0 0 1 " + (x + w) + "," + (y + r) + "V" + (y + h) + "Z";
  }

  function arc(cx, cy, R, r0, a0, a1) {
    var large = a1 - a0 > Math.PI ? 1 : 0;
    function p(r, a) { return (cx + r * Math.cos(a)).toFixed(2) + "," + (cy + r * Math.sin(a)).toFixed(2); }
    var d = "M" + p(R, a0) + "A" + R + "," + R + " 0 " + large + " 1 " + p(R, a1);
    d += r0 > 0 ? "L" + p(r0, a1) + "A" + r0 + "," + r0 + " 0 " + large + " 0 " + p(r0, a0) + "Z" : "L" + cx + "," + cy + "Z";
    return d;
  }

  /* ---------- tooltip (one for the page) ---------- */

  function ensureTip() {
    if (tip) { return; }
    tip = html("div", "chart-tooltip", null, document.body);
    tip.setAttribute("role", "status");
    tip.hidden = true;
  }

  function showTip(data, x, y) {
    ensureTip();
    tip.textContent = "";
    if (data.title) { html("div", "chart-tip-title", data.title, tip); }
    data.rows.forEach(function (r) {
      var row = html("div", "chart-tip-row", null, tip);
      if (r.key) { html("span", "chart-key chart-key-line key-" + r.key, null, row); }
      html("strong", null, r.value, row);
      if (r.name) { html("span", null, r.name, row); }
    });
    tip.hidden = false;
    var w = tip.offsetWidth, h = tip.offsetHeight;
    var left = Math.min(Math.max(8, x - w / 2), window.innerWidth - w - 8);
    var top = y - h - 12;
    if (top < 8) { top = y + 16; }
    tip.style.left = left + "px";
    tip.style.top = top + "px";
  }

  function hideTip() { if (tip) { tip.hidden = true; } }

  /* ---------- chart context: svg, hit areas, keyboard ---------- */

  function Ctx(box, W, H) {
    this.box = box; this.W = W; this.H = H; this.hits = []; this.active = -1;
    this.svg = svg("svg", { viewBox: "0 0 " + W + " " + H, width: W, height: H, "aria-hidden": "true" });
  }

  // elem = the hit shape; data = tooltip content; ax/ay = anchor in svg px
  // (null = follow the pointer); on/off = optional extra highlight.
  Ctx.prototype.hit = function (elem, data, ax, ay, on, off) {
    var self = this, idx = this.hits.length;
    this.hits.push({ el: elem, data: data, ax: ax, ay: ay, on: on, off: off });
    elem.classList.add("chart-hit");
    elem.addEventListener("pointerenter", function (e) { self.activate(idx, e); });
    elem.addEventListener("pointermove", function (e) { if (ax == null) { self.place(idx, e); } });
    elem.addEventListener("pointerleave", function () { self.deactivate(); });
    return elem;
  };

  Ctx.prototype.place = function (idx, e) {
    var h = this.hits[idx], r = this.svg.getBoundingClientRect(), x, y;
    if (h.ax == null) {
      // follow the pointer; on keyboard focus fall back to the chart's centre
      x = e && e.clientX != null ? e.clientX : r.left + r.width / 2;
      y = e && e.clientY != null ? e.clientY : r.top + r.height / 2;
    } else {
      x = r.left + h.ax * (r.width / this.W);
      y = r.top + h.ay * (r.height / this.H);
    }
    showTip(h.data, x, y);
  };

  Ctx.prototype.activate = function (idx, e) {
    this.deactivate();
    var h = this.hits[idx];
    this.active = idx;
    h.el.classList.add("is-active");
    if (h.on) { h.on(); }
    this.place(idx, e);
  };

  Ctx.prototype.deactivate = function () {
    if (this.active < 0) { return; }
    var h = this.hits[this.active];
    h.el.classList.remove("is-active");
    if (h.off) { h.off(); }
    this.active = -1;
    hideTip();
  };

  /* ---------- plot area with y axis (column, stacked, line, area) ---------- */

  function axisPlot(box, cfg, max, opts) {
    var scale = niceScale(max), W = box.clientWidth, H = cfg.height || 280;
    var labelW = Math.max(String(fmt(cfg, scale.max, true)).length, 3) * CHAR;
    var m = { l: labelW + 14, r: opts && opts.right || 12, t: 14, b: 26 };
    var c = new Ctx(box, W, H);
    var pw = W - m.l - m.r, ph = H - m.t - m.b;
    function y(v) { return m.t + ph - (v / scale.max) * ph; }
    for (var v = 0; v <= scale.max + 1e-9; v += scale.step) {
      svg("line", { x1: m.l, x2: W - m.r, y1: y(v), y2: y(v), "class": v === 0 ? "chart-baseline" : "chart-grid" }, c.svg);
      var t = svg("text", { x: m.l - 8, y: y(v) + 4, "text-anchor": "end", "class": "chart-text" }, c.svg);
      t.textContent = fmt(cfg, v, true);
    }
    var n = cfg.labels.length, band = pw / n;
    // x labels: skip some when crowded
    var every = Math.ceil((String(cfg.labels.reduce(function (a, b) { return String(a).length > String(b).length ? a : b; }, "")).length * CHAR + 8) / band);
    cfg.labels.forEach(function (l, i) {
      if (i % every) { return; }
      var tx = svg("text", { x: m.l + band * (i + 0.5), y: H - 6, "text-anchor": "middle", "class": "chart-text" }, c.svg);
      tx.textContent = l;
    });
    return { c: c, m: m, pw: pw, ph: ph, y: y, band: band, n: n, scale: scale, W: W, H: H };
  }

  function seriesRows(cfg, i, keyed) {
    return cfg.series.map(function (s, k) { return { key: k + 1, name: s.name, value: fmt(cfg, s.values[i]) }; });
  }

  /* ---------- chart types ---------- */

  var types = {};

  types.column = function (box, cfg) {
    var max = Math.max.apply(null, [].concat.apply([], cfg.series.map(function (s) { return s.values; })));
    var p = axisPlot(box, cfg, max), c = p.c, m = cfg.series.length;
    var bw = Math.min(24, (p.band * 0.6 - GAP * (m - 1)) / m), gw = m * bw + GAP * (m - 1);
    var best = { v: -1 };
    cfg.labels.forEach(function (lab, i) {
      var x0 = p.m.l + p.band * i + (p.band - gw) / 2;
      cfg.series.forEach(function (s, k) {
        var v = s.values[i], h = (v / p.scale.max) * p.ph, x = x0 + k * (bw + GAP);
        if (v > best.v) { best = { v: v, x: x + bw / 2, y: p.y(v) }; }
        if (h > 0) { svg("path", { d: barPath(x, p.y(v), bw, h, 4, "top"), "class": "fill-" + (k + 1) }, c.svg); }
      });
      c.hit(svg("rect", { x: p.m.l + p.band * i, y: p.m.t, width: p.band, height: p.ph }, c.svg),
        { title: lab, rows: seriesRows(cfg, i) }, p.m.l + p.band * (i + 0.5), p.y(Math.max.apply(null, cfg.series.map(function (s) { return s.values[i]; }))));
    });
    // label only the extreme, never every bar
    var lt = svg("text", { x: best.x, y: best.y - 6, "text-anchor": "middle", "class": "chart-text-strong" }, c.svg);
    lt.textContent = fmt(cfg, best.v, true);
    return c;
  };

  types.stacked = function (box, cfg) {
    var n = cfg.labels.length, sums = cfg.labels.map(function (_, i) {
      return cfg.series.reduce(function (a, s) { return a + s.values[i]; }, 0);
    });
    var p = axisPlot(box, cfg, Math.max.apply(null, sums)), c = p.c, bw = Math.min(24, p.band * 0.5);
    cfg.labels.forEach(function (lab, i) {
      var x = p.m.l + p.band * (i + 0.5) - bw / 2, base = 0, top = -1;
      cfg.series.forEach(function (s, k) { if (s.values[i] > 0) { top = k; } });
      cfg.series.forEach(function (s, k) {
        var v = s.values[i]; if (v <= 0) { return; }
        var y1 = p.y(base), y2 = p.y(base + v), h = y1 - y2 - (base > 0 ? GAP : 0);
        if (h > 0) { svg("path", { d: barPath(x, y2, bw, h, 4, k === top ? "top" : "none"), "class": "fill-" + (k + 1) }, c.svg); }
        base += v;
      });
      var rows = seriesRows(cfg, i).reverse();
      rows.push({ name: "Total", value: fmt(cfg, sums[i]) });
      c.hit(svg("rect", { x: p.m.l + p.band * i, y: p.m.t, width: p.band, height: p.ph }, c.svg),
        { title: lab, rows: rows }, p.m.l + p.band * (i + 0.5), p.y(sums[i]));
    });
    return c;
  };

  types.hbar = function (box, cfg) {
    var vals = cfg.series[0].values, W = box.clientWidth, rowH = 30;
    var H = cfg.labels.length * rowH + 8, c = new Ctx(box, W, H);
    var lw = Math.min(W * 0.4, Math.max.apply(null, cfg.labels.map(function (l) { return String(l).length; })) * CHAR + 12);
    var vw = String(fmt(cfg, Math.max.apply(null, vals), true)).length * CHAR + 14;
    var max = Math.max.apply(null, vals), pw = W - lw - vw;
    cfg.labels.forEach(function (lab, i) {
      var y = 4 + i * rowH, len = Math.max(0, (vals[i] / max) * pw);
      var t = svg("text", { x: lw - 10, y: y + rowH / 2 + 3, "text-anchor": "end", "class": "chart-text" }, c.svg);
      t.textContent = lab;
      var cls = cfg.highlight == null || cfg.highlight === i ? "fill-1" : "fill-muted";
      if (len > 0) { svg("path", { d: barPath(lw, y + 5, len, rowH - 10, 4, "right"), "class": cls }, c.svg); }
      var vt = svg("text", { x: lw + len + 8, y: y + rowH / 2 + 4, "class": cfg.highlight === i || cfg.highlight == null ? "chart-text-strong" : "chart-text" }, c.svg);
      vt.textContent = fmt(cfg, vals[i], true);
      c.hit(svg("rect", { x: 0, y: y, width: W, height: rowH }, c.svg),
        { title: lab, rows: [{ key: 1, name: cfg.series[0].name, value: fmt(cfg, vals[i]) }] }, lw + len / 2, y + 4);
    });
    svg("line", { x1: lw, x2: lw, y1: 2, y2: H - 2, "class": "chart-baseline" }, c.svg);
    return c;
  };

  function lineChart(box, cfg, area) {
    var max = Math.max.apply(null, [].concat.apply([], cfg.series.map(function (s) { return s.values; })));
    var endLab = cfg.series.length <= 4;
    var right = endLab ? Math.max.apply(null, cfg.series.map(function (s) {
      return String(fmt(cfg, s.values[s.values.length - 1], true)).length;
    })) * CHAR + 18 : 12;
    var p = axisPlot(box, cfg, max, { right: right }), c = p.c;
    function X(i) { return p.m.l + p.band * (i + 0.5); }
    var lastY = [];
    cfg.series.forEach(function (s, k) {
      var d = s.values.map(function (v, i) { return (i ? "L" : "M") + X(i).toFixed(1) + "," + p.y(v).toFixed(1); }).join("");
      if (area) {
        svg("path", { d: d + "L" + X(s.values.length - 1) + "," + p.y(0) + "L" + X(0) + "," + p.y(0) + "Z", "class": "chart-area fill-" + (k + 1) }, c.svg);
      }
      svg("path", { d: d, "class": "chart-line stroke-" + (k + 1) }, c.svg);
      var li = s.values.length - 1, ly = p.y(s.values[li]);
      svg("circle", { cx: X(li), cy: ly, r: 5, "class": "chart-dot fill-" + (k + 1) }, c.svg);
      // end label (the value) unless it would collide with one already placed
      if (endLab && lastY.every(function (o) { return Math.abs(o - ly) > 14; })) {
        lastY.push(ly);
        var t = svg("text", { x: X(li) + 11, y: ly + 4, "class": "chart-text-strong" }, c.svg);
        t.textContent = fmt(cfg, s.values[li], true);
      }
    });
    // crosshair + one tooltip listing every series at that x
    var cross = svg("line", { y1: p.m.t, y2: p.m.t + p.ph, "class": "chart-crosshair" }, c.svg);
    cross.setAttribute("visibility", "hidden");
    var dots = cfg.series.map(function (s, k) {
      var d = svg("circle", { r: 5, "class": "chart-dot fill-" + (k + 1) }, c.svg);
      d.setAttribute("visibility", "hidden");
      return d;
    });
    cfg.labels.forEach(function (lab, i) {
      var x = X(i);
      c.hit(svg("rect", { x: p.m.l + p.band * i, y: p.m.t, width: p.band, height: p.ph }, c.svg),
        { title: lab, rows: seriesRows(cfg, i) }, x, p.y(Math.max.apply(null, cfg.series.map(function (s) { return s.values[i]; }))),
        function () {
          cross.setAttribute("x1", x); cross.setAttribute("x2", x); cross.setAttribute("visibility", "visible");
          dots.forEach(function (d, k) { d.setAttribute("cx", x); d.setAttribute("cy", p.y(cfg.series[k].values[i])); d.setAttribute("visibility", "visible"); });
        },
        function () { cross.setAttribute("visibility", "hidden"); dots.forEach(function (d) { d.setAttribute("visibility", "hidden"); }); });
    });
    return c;
  }
  types.line = function (box, cfg) { return lineChart(box, cfg, false); };
  types.area = function (box, cfg) { return lineChart(box, cfg, true); };

  function pieChart(box, cfg, donut) {
    var W = box.clientWidth, H = Math.min(cfg.height || 220, W), c = new Ctx(box, W, H);
    var R = H / 2 - 6, r0 = donut ? R - 24 : 0, cx = W / 2, cy = H / 2;
    var total = cfg.items.reduce(function (a, i) { return a + i.value; }, 0), a = -Math.PI / 2;
    var gap = cfg.items.length > 1 ? GAP / R : 0;
    cfg.items.forEach(function (it, i) {
      var sweep = (it.value / total) * Math.PI * 2;
      var slot = it.colorSlot || i + 1;
      // An SVG arc whose start and end coincide cannot draw a full circle.
      var path = cfg.items.length === 1 ?
        (donut ? svg("circle", { cx: cx, cy: cy, r: (R + r0) / 2,
          "class": "chart-slice chart-full-ring stroke-" + slot }, c.svg) :
          svg("circle", { cx: cx, cy: cy, r: R, "class": "chart-slice fill-" + slot }, c.svg)) :
        svg("path", { d: arc(cx, cy, R, r0, a + gap / 2, a + sweep - gap / 2), "class": "chart-slice fill-" + slot }, c.svg);
      c.hit(path, { title: it.name, rows: [{ key: slot, name: Math.round(it.value / total * 100) + "%", value: fmt(cfg, it.value) }] }, null, null);
      path.classList.remove("chart-hit"); path.classList.add("chart-slice");
      a += sweep;
    });
    if (donut) {
      var t = svg("text", { x: cx, y: cy + 4, "text-anchor": "middle", "class": "chart-text-hero" }, c.svg);
      t.textContent = fmt(cfg, total, true);
      var l = svg("text", { x: cx, y: cy + 22, "text-anchor": "middle", "class": "chart-text" }, c.svg);
      l.textContent = cfg.centerLabel || "Total";
    }
    return c;
  }
  types.pie = function (box, cfg) { return pieChart(box, cfg, false); };
  types.donut = function (box, cfg) { return pieChart(box, cfg, true); };

  types.ring = function (box, cfg) {
    var S = Math.min(box.clientWidth, cfg.size || 132), c = new Ctx(box, S, S), w = 12, r = S / 2 - w;
    var v = Math.max(0, Math.min(100, cfg.value)), sev = v >= (cfg.critical || 90) ? "critical" : v >= (cfg.warning || 70) ? "warning" : "1";
    svg("circle", { cx: S / 2, cy: S / 2, r: r, "stroke-width": w, "class": "chart-ring-track" }, c.svg);
    svg("circle", { cx: S / 2, cy: S / 2, r: r, "stroke-width": w, pathLength: 100, "stroke-dasharray": v + " " + (100 - v),
      transform: "rotate(-90 " + S / 2 + " " + S / 2 + ")", "class": "chart-ring-value stroke-" + sev }, c.svg);
    var t = svg("text", { x: S / 2, y: S / 2 + 9, "text-anchor": "middle", "class": "chart-text-hero" }, c.svg);
    t.textContent = Math.round(v) + "%";
    var holder = box.closest(".chart").parentNode.querySelector("[data-gauge-status]");
    if (holder) {   // status colour always travels with an icon and a word
      var word = sev === "critical" ? "Critical" : sev === "warning" ? "High" : "Healthy";
      var icon = sev === "critical" ? "bi-x-octagon-fill" : sev === "warning" ? "bi-exclamation-triangle-fill" : "bi-check-circle-fill";
      holder.textContent = "";
      var i = html("i", "bi " + icon + " me-1", null, holder); i.setAttribute("aria-hidden", "true");
      holder.appendChild(document.createTextNode(word));
    }
    return c;
  };

  types.heatmap = function (box, cfg) {
    var W = box.clientWidth, rows = cfg.rows.length, cols = cfg.cols.length, ml = Math.max.apply(null, cfg.rows.map(function (r) { return r.length; })) * CHAR + 12;
    var cw = (W - ml) / cols, ch = Math.min(cw, 26), H = rows * ch + 24, c = new Ctx(box, W, H);
    var max = Math.max.apply(null, [].concat.apply([], cfg.values));
    cfg.rows.forEach(function (r, y) {
      var t = svg("text", { x: ml - 8, y: y * ch + ch / 2 + 4, "text-anchor": "end", "class": "chart-text" }, c.svg);
      t.textContent = r;
      cfg.cols.forEach(function (col, x) {
        var v = cfg.values[y][x], b = Math.min(5, Math.floor((v / max) * 5.999));
        var cell = svg("rect", { x: ml + x * cw + 1, y: y * ch + 1, width: cw - GAP, height: ch - GAP, rx: 3, "class": "chart-cell heat-" + b }, c.svg);
        c.hit(cell, { title: r + ", " + col, rows: [{ value: fmt(cfg, v), name: cfg.unit || "" }] }, null, null);
        cell.classList.remove("chart-hit");
      });
    });
    var every = Math.ceil((String(cfg.cols[0]).length * CHAR + 6) / cw);
    cfg.cols.forEach(function (col, x) {
      if (x % every) { return; }
      var t = svg("text", { x: ml + x * cw + cw / 2, y: H - 6, "text-anchor": "middle", "class": "chart-text" }, c.svg);
      t.textContent = col;
    });
    return c;
  };

  types.spark = function (box, cfg) {
    var W = box.clientWidth, H = 36, c = new Ctx(box, W, H), v = cfg.values;
    var lo = Math.min.apply(null, v), hi = Math.max.apply(null, v), pad = 6;
    function X(i) { return pad + (W - pad * 2) * i / (v.length - 1); }
    function Y(val) { return H - pad - (H - pad * 2) * (val - lo) / ((hi - lo) || 1); }
    svg("path", { d: v.map(function (val, i) { return (i ? "L" : "M") + X(i).toFixed(1) + "," + Y(val).toFixed(1); }).join(""), "class": "chart-line stroke-muted" }, c.svg);
    svg("circle", { cx: X(v.length - 1), cy: Y(v[v.length - 1]), r: 4, "class": "chart-dot fill-1" }, c.svg);
    return c;
  };

  /* ---------- legend and table twin ---------- */

  function legend(box, type, cfg) {
    var items = [];
    if (cfg.series && cfg.series.length > 1 && type !== "hbar") {
      items = cfg.series.map(function (s, k) { return { n: k + 1, name: s.name, line: type === "line" }; });
    } else if (cfg.items) {
      var total = cfg.items.reduce(function (a, i) { return a + i.value; }, 0);
      items = cfg.items.map(function (it, k) { return { n: it.colorSlot || k + 1, name: it.name, value: Math.round(it.value / total * 100) + "%" }; });
    } else if (type === "heatmap") {
      var ul = html("ul", "chart-legend", null, box);
      var li = html("li", null, "Less", ul);
      for (var b = 0; b < 6; b++) { html("span", "chart-key heat-key-" + b, null, li); }
      li.appendChild(document.createTextNode("More"));
      return;
    }
    if (!items.length) { return; }
    var list = html("ul", "chart-legend", null, box);
    items.forEach(function (it) {
      var li2 = html("li", null, null, list);
      html("span", "chart-key" + (it.line ? " chart-key-line" : "") + " key-" + it.n, null, li2);
      li2.appendChild(document.createTextNode(it.name));
      if (it.value) { html("strong", null, it.value, li2); }
    });
  }

  function tableData(type, cfg) {
    if (cfg.items) {
      var total = cfg.items.reduce(function (a, i) { return a + i.value; }, 0);
      return { head: ["Name", "Value", "Share"], rows: cfg.items.map(function (i) { return [i.name, fmt(cfg, i.value), Math.round(i.value / total * 100) + "%"]; }) };
    }
    if (type === "heatmap") {
      return { head: [""].concat(cfg.cols), rows: cfg.rows.map(function (r, y) { return [r].concat(cfg.values[y].map(function (v) { return fmt(cfg, v); })); }) };
    }
    if (type === "ring") { return { head: ["Measure", "Value"], rows: [[cfg.label || "Value", Math.round(cfg.value) + "%"]] }; }
    if (type === "spark") { return { head: ["Point", "Value"], rows: cfg.values.map(function (v, i) { return [String(i + 1), fmt(cfg, v)]; }) }; }
    return { head: [cfg.labelHeading || ""].concat(cfg.series.map(function (s) { return s.name; })),
      rows: cfg.labels.map(function (l, i) { return [l].concat(cfg.series.map(function (s) { return fmt(cfg, s.values[i]); })); }) };
  }

  function buildTable(wrap, type, cfg) {
    var d = tableData(type, cfg);
    wrap.textContent = "";
    var table = html("table", "table table-custom table-striped mb-0", null, wrap);
    var tr = html("tr", null, null, html("thead", null, null, table));
    d.head.forEach(function (h) { var th = html("th", null, h, tr); th.setAttribute("scope", "col"); });
    var body = html("tbody", null, null, table);
    d.rows.forEach(function (row) {
      var r = html("tr", null, null, body);
      row.forEach(function (cell, i) { var e = html(i ? "td" : "th", null, cell, r); if (!i) { e.setAttribute("scope", "row"); } });
    });
  }

  /* ---------- mounting ---------- */

  function draw(box) {
    var st = box._chart;
    if (!st || box.clientWidth < 40) { return; }
    hideTip();
    box.textContent = "";
    var holder = html("div", null, null, box);
    var c = types[st.type](holder, st.cfg);
    holder.appendChild(c.svg);
    legend(box, st.type, st.cfg);
    st.ctx = c;
  }

  function init(box) {
    var type = box.getAttribute("data-chart"), cfg;
    try { cfg = JSON.parse(box.getAttribute("data-config")); } catch (e) { return; }
    if (!types[type] || !cfg) { return; }
    box._chart = { type: type, cfg: cfg };
    var interactive = ["spark", "ring"].indexOf(type) < 0;
    if (interactive) {
      box.setAttribute("tabindex", "0");
      box.setAttribute("role", "group");
      box.setAttribute("aria-roledescription", "chart");
      box.addEventListener("keydown", function (e) {
        var c = box._chart.ctx; if (!c || !c.hits.length) { return; }
        var i = c.active, last = c.hits.length - 1;
        if (e.key === "ArrowRight" || e.key === "ArrowDown") { i = i < last ? i + 1 : 0; }
        else if (e.key === "ArrowLeft" || e.key === "ArrowUp") { i = i > 0 ? i - 1 : last; }
        else if (e.key === "Home") { i = 0; }
        else if (e.key === "End") { i = last; }
        else if (e.key === "Escape") { c.deactivate(); return; }
        else { return; }
        e.preventDefault();
        c.activate(i, null);
      });
      box.addEventListener("blur", function () { if (box._chart.ctx) { box._chart.ctx.deactivate(); } });
    } else {
      box.setAttribute("role", "img");
    }

    var card = box.closest(".chart-card");
    var btn = card && card.querySelector("[data-chart-table-toggle]");
    var wrap = card && card.querySelector(".chart-table-wrap");
    if (btn && wrap) {
      buildTable(wrap, type, cfg);
      btn.addEventListener("click", function () {
        var showTable = wrap.hidden;
        wrap.hidden = !showTable;
        box.hidden = showTable;
        btn.setAttribute("aria-pressed", String(showTable));
        btn.querySelector("span").textContent = showTable ? "Chart" : "Table";
        btn.querySelector("i").className = "bi me-2 " + (showTable ? "bi-bar-chart" : "bi-table");
        if (!showTable) { draw(box); }
      });
    }

    var lastW = 0, raf;
    new ResizeObserver(function () {
      if (Math.abs(box.clientWidth - lastW) < 1) { return; }
      lastW = box.clientWidth;
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(function () { draw(box); });
    }).observe(box);
    draw(box);
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("[data-chart]").forEach(init);
  });

  // Refresh one mounted chart after its local search or filter changes.
  // Reuse its listeners and ResizeObserver instead of mounting it again.
  function update(box, cfg) {
    if (!box._chart) { return; }
    box._chart.cfg = cfg;
    var wrap = box.closest(".chart-card").querySelector(".chart-table-wrap");
    if (wrap) { buildTable(wrap, box._chart.type, cfg); }
    draw(box);
  }

  window.Charts = { init: init, update: update };
})();
