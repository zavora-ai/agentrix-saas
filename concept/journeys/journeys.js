/* Agentrix WebAgent Platform: the dual journeys. Shared by both lanes.
   - J.LANES: the step registry for the owner and customer lanes (step strip and pager).
   - Living marks (ADR-031): every <span class="j-mark" data-agent=… data-size=… data-state=…>
     becomes a generated mark; data-chief draws the team mark.
   - J.setState(el, state): change a mark's state; motion settles after 5 s (06 §4.3).
   - Tabs: [role=tablist] buttons with aria-controls switch their panels.
   The mark generator is copied from concept/identity/marks.html (the reference). */
(() => {
  "use strict";

  /* ---------- lanes ---------- */
  const LANES = {
    customer: [
      { file: "customer-browse.html", title: "Arrive and browse" },
      { file: "customer-account.html", title: "An account when needed" },
      { file: "customer-order.html", title: "Place an order", meet: "meet-order" },
      { file: "customer-pay.html", title: "Pay with PayPal", meet: "meet-payment" },
      { file: "customer-after.html", title: "After the order", meet: "meet-refund" },
      { file: "customer-agent.html", title: "Her AI agent orders" },
      { file: "customer-security.html", title: "Security moments" }
    ],
    owner: [
      { file: "owner-discover.html", title: "Discover and assess" },
      { file: "owner-account.html", title: "Keep it" },
      { file: "owner-plan.html", title: "Plan and pay" },
      { file: "owner-domain.html", title: "Claim the domain" },
      { file: "owner-move.html", title: "The Move" },
      { file: "owner-golive.html", title: "Go live" },
      { file: "owner-day1.html", title: "The first brief" },
      { file: "owner-order.html", title: "The first order", meet: "meet-order" },
      { file: "owner-billing.html", title: "Billing and ownership" }
    ]
  };
  const LANE_NAME = { customer: "Customer journey", owner: "Owner journey" };

  /* ---------- glyphs (24 px line icons) ---------- */
  const GLYPHS = {
    cake: '<path d="M4 20h16"/><path d="M5.5 20v-6.5h13V20"/><path d="M5.5 15.5c1.6 1.2 3.4 1.2 5 0 1.6 1.2 3.4 1.2 5 0 1 .8 2 .9 3 .5"/><path d="M12 13.5V10"/><path d="M12 7.6c-1-.9-1-2.1 0-3.4 1 1.3 1 2.5 0 3.4z"/>',
    loaf: '<path d="M6 12.2A3.6 3.6 0 0 1 6.8 5h10.4a3.6 3.6 0 0 1 .8 7.2V19a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1z"/><path d="M9.5 9.5l1.5 1.8"/><path d="M13 8.6l1.5 1.8"/>',
    bag: '<path d="M5.5 8.5h13l-1.1 11.5H6.6z"/><path d="M9 8.5V7a3 3 0 0 1 6 0v1.5"/>',
    pen: '<path d="M4.5 19.5l1-4L16 5l3 3L8.5 18.5z"/><path d="M14 7l3 3"/>',
    shield: '<path d="M12 3.5l7 2.8v5.4c0 4.2-2.9 7.3-7 8.8-4.1-1.5-7-4.6-7-8.8V6.3z"/><path d="M9 12l2.1 2.1L15.2 10"/>',
    coin: '<circle cx="12" cy="12" r="8"/><path d="M12 7.5v9"/><path d="M14.4 9.7c-.4-.8-1.3-1.3-2.4-1.3-1.3 0-2.4.7-2.4 1.8s1.1 1.4 2.4 1.7 2.4.7 2.4 1.8-1.1 1.8-2.4 1.8c-1.1 0-2-.5-2.4-1.3"/>',
    chat: '<path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 16h-6.5L8 19.5V16H5a1.5 1.5 0 0 1-1.5-1.5v-8A1.5 1.5 0 0 1 5 5z"/>',
    boxes: '<path d="M3.5 7.5L12 3.5l8.5 4L12 11.5z"/><path d="M3.5 7.5v9l8.5 4 8.5-4v-9"/><path d="M12 11.5v9"/>',
    key: '<circle cx="8.5" cy="15.5" r="4"/><path d="M11.3 12.7L19.5 4.5"/><path d="M16.5 7.5l2.2 2.2M14.5 9.5l1.8 1.8"/>',
    star: '<path d="M12 4l2.5 5.1 5.6.8-4 3.9.9 5.6L12 16.8l-5 2.6.9-5.6-4-3.9 5.6-.8z"/>'
  };

  /* ---------- Kilele Bakery (fictional): one team, one family, one seed each ---------- */
  const SITE = {
    key: "kilele", name: "Kilele Bakery", owner: "Wanjiru",
    font: "'Fraunces', Georgia, serif", ink: "#3b2416", family: "grain", n: 4.6,
    agents: [
      { id: "cakes", name: "Cakes", glyph: "cake", color: "#8e3b46", seed: 7 },
      { id: "bread", name: "Bread", glyph: "loaf", color: "#9c5725", seed: 3 },
      { id: "orders", name: "Orders", glyph: "bag", color: "#3b2416", seed: 11 },
      { id: "cashier", name: "Cashier", glyph: "coin", color: "#11734a", seed: 23 },
      { id: "mover", name: "Mover", glyph: "boxes", color: "#2f55c4", seed: 17 },
      { id: "caretaker", name: "Caretaker", glyph: "key", color: "#3d5a1f", seed: 29 },
      { id: "editor", name: "Editor", glyph: "pen", color: "#577035", seed: 5 },
      { id: "guardian", name: "Guardian", glyph: "shield", color: "#56636f", seed: 2 }
    ]
  };
  const AGENT = Object.fromEntries(SITE.agents.map(a => [a.id, a]));

  /* ---------- colour ---------- */
  function hexToRgb(h) { h = h.replace("#", ""); if (h.length === 3) h = h.split("").map(c => c + c).join(""); const n = parseInt(h, 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
  function rgbToHex(r, g, b) { return "#" + [r, g, b].map(v => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0")).join(""); }
  function lum(h) { const w = [.2126, .7152, .0722]; return hexToRgb(h).reduce((a, v, i) => { v /= 255; return a + w[i] * (v <= .03928 ? v / 12.92 : Math.pow((v + .055) / 1.055, 2.4)); }, 0); }
  function contrast(a, b) { const A = lum(a), B = lum(b); return (Math.max(A, B) + .05) / (Math.min(A, B) + .05); }
  const ON_DARK = "#ffffff", ON_LIGHT = "#1b1726";
  function onColor(fill) { return contrast(fill, ON_DARK) >= contrast(fill, ON_LIGHT) ? ON_DARK : ON_LIGHT; }
  function mix(a, b, t) { const A = hexToRgb(a), B = hexToRgb(b); return rgbToHex(...A.map((v, i) => v + (B[i] - v) * t)); }

  /* ---------- seeded generator ---------- */
  function seedFrom(str) { let h = 1779033703 ^ str.length; for (let i = 0; i < str.length; i++) { h = Math.imul(h ^ str.charCodeAt(i), 3432918353); h = (h << 13) | (h >>> 19); } h = Math.imul(h ^ (h >>> 16), 2246822507); h = Math.imul(h ^ (h >>> 13), 3266489909); return (h ^= h >>> 16) >>> 0; }
  function rngFor(key) { let a = seedFrom(key); return () => { a |= 0; a = (a + 0x6D2B79F5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }

  /* ---------- geometry ---------- */
  const f = v => (Math.round(v * 100) / 100).toString();
  function squircle(n, r, cx = 50, cy = 50, steps = 96) {
    let d = "";
    for (let i = 0; i < steps; i++) {
      const t = (i / steps) * Math.PI * 2, c = Math.cos(t), s = Math.sin(t);
      d += (i ? "L" : "M") + f(cx + r * Math.sign(c) * Math.pow(Math.abs(c), 2 / n)) + " " + f(cy + r * Math.sign(s) * Math.pow(Math.abs(s), 2 / n));
    }
    return d + "Z";
  }
  function arc(r, a0, a1, color, w, op) {
    const p = a => [50 + r * Math.cos(a * Math.PI / 180), 50 + r * Math.sin(a * Math.PI / 180)];
    const [x0, y0] = p(a0), [x1, y1] = p(a1), large = (a1 - a0) > 180 ? 1 : 0;
    return `<path d="M${f(x0)} ${f(y0)}A${f(r)} ${f(r)} 0 ${large} 1 ${f(x1)} ${f(y1)}" fill="none" stroke="${color}" stroke-width="${f(w)}" stroke-linecap="round" opacity="${f(op)}"/>`;
  }
  function wave(x0, x1, y0, amp, lam, ph) {
    let d = "";
    for (let x = x0, i = 0; x <= x1; x += 2.5, i++) d += (i ? "L" : "M") + f(x) + " " + f(y0 + amp * Math.sin((x + ph) / lam * Math.PI * 2));
    return d;
  }

  /* ---------- signature families (from marks.html at ba30748) ----------
     The pattern is drawn in the glyph's colour inside one group whose opacity is the ceiling
     (SIG_ALPHA by detail: none below 28 px, 22% at 28-44 px, 30% above), so overlaps can't stack
     above it. The glyph is checked against the fill with the pattern at its strongest (06 §4.3). */
  const SIG_ALPHA = [0, .22, .3];
  function glyphContrast(fill) { const on = onColor(fill); return contrast(on, mix(fill, on, SIG_ALPHA[2])); }
  const SIG = {
    orbits(r, c) {
      const k = 3 + Math.floor(r() * 2); let s = "";
      for (let i = 0; i < k; i++) { const rad = 16 + i * (34 / (k - 1)) + (r() - .5) * 3, sweep = 150 + r() * 170, start = r() * 360; s += arc(rad, start, start + sweep, c, 3.2 + r() * 2.6, .8); }
      const ma = r() * Math.PI * 2;
      s += `<circle cx="${f(50 + 31.7 * Math.cos(ma))}" cy="${f(50 + 31.7 * Math.sin(ma))}" r="${f(4 + r() * 2)}" fill="${c}"/>`;
      return { inner: s, motion: "spin" };
    },
    grain(r, c) {
      const n = 34 + Math.floor(r() * 24), sc = 5.6 + r() * 1.6, rot = r() * 360, ox = (r() - .5) * 14, oy = (r() - .5) * 14;
      let s = "";
      for (let i = 1; i <= n; i++) { const a = (i * 137.508 + rot) * Math.PI / 180, d = sc * Math.sqrt(i); s += `<circle cx="${f(50 + ox + d * Math.cos(a))}" cy="${f(50 + oy + d * Math.sin(a))}" r="${f(1.1 + (i / n) * 2.6)}"/>`; }
      return { inner: `<g fill="${c}">${s}</g>`, motion: "spin" };
    },
    strata(r, c) {
      const bands = 4 + Math.floor(r() * 3), lam = 34 + r() * 30, amp = 3 + r() * 6, ang = -30 + r() * 60, gap = 150 / bands;
      let s = "";
      for (let i = 0; i < bands; i++) { const y0 = -25 + gap * (i + .5), ph = r() * lam; s += `<path d="${wave(-50 - lam, 150 + lam, y0, amp, lam, ph)}" fill="none" stroke="${c}" stroke-width="${f(gap * (.3 + r() * .22))}" stroke-linecap="round" opacity="${i % 2 ? 1 : .6}"/>`; }
      return { inner: `<g transform="rotate(${f(ang)} 50 50)"><g class="wa-drift" style="--p:${f(-lam)}px">${s}</g></g>`, motion: "drift" };
    },
    weave(r, c) {
      const sp = 9 + r() * 6, a1 = r() * 180, a2 = a1 + 55 + r() * 50, sw = 1.6 + r() * 1.4;
      const set = ang => { let s = ""; for (let x = -90; x <= 190; x += sp) s += `<line x1="${f(x)}" y1="-70" x2="${f(x)}" y2="170"/>`; return `<g transform="rotate(${f(ang)} 50 50)"><g class="wa-drift" style="--p:${f(-sp)}px">${s}</g></g>`; };
      return { inner: `<g stroke="${c}" stroke-width="${f(sw)}">${set(a1)}${set(a2)}</g>`, motion: "drift" };
    },
    facets(r, c) {
      const px = 50 + (r() - .5) * 36, py = 50 + (r() - .5) * 36, k = 7 + Math.floor(r() * 5), angs = []; let a = r() * 360;
      for (let i = 0; i < k; i++) { angs.push(a); a += (360 / k) * (.6 + r() * .8); }
      let tri = "", lines = "";
      for (let i = 0; i < k; i++) {
        const b1 = angs[i] * Math.PI / 180, b2 = angs[(i + 1) % k] * Math.PI / 180, R = 92, p1 = [50 + R * Math.cos(b1), 50 + R * Math.sin(b1)], p2 = [50 + R * Math.cos(b2), 50 + R * Math.sin(b2)];
        if (i % 2 === 0) tri += `<path d="M${f(px)} ${f(py)}L${f(p1[0])} ${f(p1[1])}L${f(p2[0])} ${f(p2[1])}Z" fill="${c}" fill-opacity="${f(.4 + r() * .45)}"/>`;
        lines += `<line x1="${f(px)}" y1="${f(py)}" x2="${f(p1[0])}" y2="${f(p1[1])}"/>`;
      }
      return { inner: `${tri}<g stroke="${c}" stroke-width="1.4">${lines}</g>`, motion: "spin" };
    }
  };

  /* ---------- marks ---------- */
  let UID = 0;
  const esc = s => String(s).replace(/[&<>"']/g, ch => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch]));
  const VB = 'viewBox="-12 -12 124 134"';
  const dims = size => `width="${f(size * 1.24)}" height="${f(size * 1.34)}"`;

  function markSVG(m, size, state = "idle") {
    const id = "jm" + (++UID), detail = size <= 24 ? 0 : size <= 44 ? 1 : 2, on = onColor(m.color);
    const body = squircle(m.n, 50), halo = squircle(m.n, 58);
    let sig = "", motion = "";
    if (detail > 0) { const out = SIG[m.family](rngFor(`${m.site}:${m.id}:${m.seed}`), on); sig = `<g opacity="${f(SIG_ALPHA[detail])}">${out.inner}</g>`; motion = out.motion; }
    const gs = detail === 0 ? 60 : detail === 1 ? 50 : 44, gx = 50 - gs / 2, sw = detail === 0 ? 2.7 : detail === 1 ? 2.3 : 2;
    const glyph = m.glyph === "monogram"
      ? `<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="${esc(m.font)}" font-size="52" fill="${on}">${esc(m.mono || "")}</text>`
      : `<svg x="${gx}" y="${gx}" width="${gs}" height="${gs}" viewBox="0 0 24 24" fill="none" stroke="${on}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round">${GLYPHS[m.glyph] || ""}</svg>`;
    const fill = detail === 2 ? `url(#${id}g)` : m.color;
    return `<svg class="wa" data-state="${state}" data-motion="${motion}" ${dims(size)} ${VB} aria-hidden="true" focusable="false">` +
      `<defs><clipPath id="${id}c"><path d="${body}"/></clipPath><linearGradient id="${id}g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${mix(m.color, "#ffffff", .16)}"/><stop offset="1" stop-color="${m.color}"/></linearGradient></defs>` +
      `<path class="wa-halo" d="${halo}" fill="none" stroke="${m.color}" stroke-width="4"/>` +
      `<g class="wa-core"><path d="${body}" fill="${fill}"/>` +
      (sig ? `<g clip-path="url(#${id}c)"><g class="wa-sig"><circle cx="50" cy="50" r="80" fill="none"/>${sig}</g></g>` : "") +
      glyph + `</g>` +
      `<line class="wa-trace" x1="8" y1="117" x2="92" y2="117" stroke="${m.color}" stroke-width="4" stroke-linecap="round"/>` +
      `<circle class="wa-dot" cx="96" cy="4" r="11" fill="#b26b00" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/></svg>`;
  }

  function chiefSVG(size, busy = [], waiting = false) {
    const on = onColor(SITE.ink), body = squircle(SITE.n, 50), halo = squircle(SITE.n, 58), team = SITE.agents, k = team.length;
    let dots = `<circle cx="50" cy="50" r="33" fill="none" stroke="${on}" stroke-width="1.5" opacity=".25"/>`;
    team.forEach((a, i) => {
      const ang = (-90 + i * 360 / k) * Math.PI / 180;
      dots += `<circle class="chief-dot${busy.includes(a.id) ? " busy" : ""}" cx="${f(50 + 33 * Math.cos(ang))}" cy="${f(50 + 33 * Math.sin(ang))}" r="7" fill="${a.color}" stroke="${on}" stroke-width="2.4"/>`;
    });
    return `<svg class="wa chief" data-state="${waiting ? "waiting" : "idle"}" ${dims(size)} ${VB} aria-hidden="true" focusable="false">` +
      `<path class="wa-halo" d="${halo}" fill="none" stroke="${SITE.ink}" stroke-width="4"/>` +
      `<g class="wa-core"><path d="${body}" fill="${SITE.ink}"/>${dots}` +
      `<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="${esc(SITE.font)}" font-size="34" fill="${on}">${esc(SITE.name[0])}</text></g>` +
      `<circle class="wa-dot" cx="96" cy="4" r="11" fill="#b26b00" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/></svg>`;
  }

  const specFor = id => { const a = AGENT[id] || AGENT.cakes; return { site: SITE.key, id: a.id, color: a.color, n: SITE.n, glyph: a.glyph, mono: a.name[0], family: SITE.family, seed: a.seed, font: SITE.font }; };

  /* Motion settles after 5 s in one state (06 §4.3); speaking follows the audio instead. */
  const timers = new WeakMap();
  function setState(el, state) {
    const svg = el && (el.matches(".wa") ? el : el.querySelector(".wa"));
    if (!svg) return;
    svg.setAttribute("data-state", state);
    svg.removeAttribute("data-settled");
    clearTimeout(timers.get(svg));
    if (state === "listening" || state === "working") timers.set(svg, setTimeout(() => svg.setAttribute("data-settled", ""), 5000));
  }

  function drawMarks(root = document) {
    root.querySelectorAll(".j-mark:not([data-drawn])").forEach(el => {
      const size = Number(el.dataset.size || 28), state = el.dataset.state || "idle";
      el.innerHTML = el.hasAttribute("data-chief")
        ? chiefSVG(size, (el.dataset.busy || "").split(",").filter(Boolean), el.dataset.state === "waiting")
        : markSVG(specFor(el.dataset.agent), size, state);
      el.setAttribute("data-drawn", "");
      if (!el.hasAttribute("data-chief")) setState(el, state);
    });
  }

  /* ---------- step strip and pager ---------- */
  function drawNav() {
    const lane = document.body.dataset.lane, step = Number(document.body.dataset.step || 0), steps = LANES[lane];
    if (!steps) return;
    const strip = document.querySelector("[data-steps]");
    if (strip) {
      strip.innerHTML = `<ol>${steps.map((s, i) => `<li><a href="${s.file}"${i + 1 === step ? ' aria-current="step"' : ""}><span class="n">${i + 1}</span>${esc(s.title)}${s.meet ? '<span class="meet-dot" title="Meets the other lane"></span>' : ""}</a></li>`).join("")}</ol>`;
      const cur = strip.querySelector('[aria-current="step"]');
      if (cur) strip.querySelector("ol").scrollLeft = Math.max(0, cur.offsetLeft - 24);
    }
    const pager = document.querySelector("[data-pager]");
    if (pager) {
      const prev = steps[step - 2], next = steps[step];
      pager.innerHTML =
        (prev ? `<a class="prev" href="${prev.file}"><span>← Step ${step - 1}</span><b>${esc(prev.title)}</b></a>` : `<a class="prev" href="index.html"><span>← ${esc(LANE_NAME[lane])}</span><b>The journey map</b></a>`) +
        (next ? `<a class="next" href="${next.file}"><span>Step ${step + 1} →</span><b>${esc(next.title)}</b></a>` : `<a class="next" href="index.html"><span>End of the lane →</span><b>Back to the map</b></a>`);
    }
  }

  /* ---------- tabs ---------- */
  function wireTabs(root = document) {
    root.querySelectorAll('[role="tablist"]:not([data-wired])').forEach(list => {
      const tabs = [...list.querySelectorAll('[role="tab"]')];
      const select = t => tabs.forEach(x => {
        const on = x === t, panel = document.getElementById(x.getAttribute("aria-controls"));
        x.setAttribute("aria-selected", on ? "true" : "false");
        x.tabIndex = on ? 0 : -1;
        if (panel) panel.hidden = !on;
      });
      tabs.forEach((t, i) => {
        t.addEventListener("click", () => select(t));
        t.addEventListener("keydown", e => {
          const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
          if (d) { e.preventDefault(); const n = tabs[(i + d + tabs.length) % tabs.length]; select(n); n.focus(); }
        });
      });
      select(tabs.find(t => t.getAttribute("aria-selected") === "true") || tabs[0]);
      list.setAttribute("data-wired", "");
    });
  }

  window.J = { LANES, SITE, AGENT, SIG_ALPHA, glyphContrast, contrast, markSVG, chiefSVG, specFor, setState, drawMarks, wireTabs };
  const init = () => { drawNav(); drawMarks(); wireTabs(); };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init); else init();
})();
