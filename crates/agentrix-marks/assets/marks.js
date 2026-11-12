/* The WebAgent living mark (ADR-031): the browser half of agentrix-marks.
   The generator below is the Rust crate's, operation for operation (src/num.rs, src/rng.rs,
   src/svg.rs); the golden tests render the same cases through both and compare them byte for
   byte, so a mark drawn here for a live preview is the mark the signed release carries.
   The <wa-mark> element only changes data-state when a presence frame says so, settles motion
   after 5 seconds, and follows the real voice level while speaking. It never invents a state. */

/* ---------- arithmetic that gives the same bits as Rust ---------- */
const PI = Math.PI;
const PIO2_1 = 1.5707963267341256;
const PIO2_1T = 6.077100506506192e-11;
const INVPIO2 = 0.6366197723675814;
const S1 = -0.16666666666666632;
const S2 = 0.00833333333332249;
const S3 = -0.0001984126982985795;
const S4 = 2.7557313707070068e-6;
const S5 = -2.5050760253406863e-8;
const S6 = 1.58969099521155e-10;
const C1 = 0.0416666666666666;
const C2 = -0.001388888888887411;
const C3 = 2.480158728947673e-5;
const C4 = -2.7557314351390663e-7;
const C5 = 2.087572321298175e-9;
const C6 = -1.1359647557788195e-11;
const LN2_HI = 0.6931471803691238;
const LN2_LO = 1.9082149292705877e-10;
const INV_LN2 = 1.4426950408889634;
const SQRT2 = 1.4142135623730951;
const SQRT1_2 = 0.7071067811865476;

function ksin(x) { const z = x * x, v = z * x, r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6))); return x + v * (S1 + z * r); }
function kcos(x) { const z = x * x, r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6))))); return 1 - (0.5 * z - z * r); }
function reduce(x) { const q = Math.floor(x * INVPIO2 + 0.5), r = (x - q * PIO2_1) - q * PIO2_1T; return [((q % 4) + 4) % 4, r]; }
export function sin(x) { const [n, r] = reduce(x); return n === 0 ? ksin(r) : n === 1 ? kcos(r) : n === 2 ? -ksin(r) : -kcos(r); }
export function cos(x) { const [n, r] = reduce(x); return n === 0 ? kcos(r) : n === 1 ? -ksin(r) : n === 2 ? -kcos(r) : ksin(r); }
function ln(x) {
  let m = x, k = 0;
  while (m > SQRT2) { m *= 0.5; k += 1; }
  while (m < SQRT1_2) { m *= 2; k -= 1; }
  const s = (m - 1) / (m + 1), s2 = s * s;
  let acc = 1 / 25;
  for (let j = 11; j >= 0; j -= 1) acc = 1 / (2 * j + 1) + s2 * acc;
  return k * LN2_HI + (k * LN2_LO + 2 * s * acc);
}
function exp(y) {
  const k = Math.floor(y * INV_LN2 + 0.5), r = (y - k * LN2_HI) - k * LN2_LO;
  let acc = 1;
  for (let n = 18; n >= 1; n -= 1) acc = 1 + acc * r / n;
  if (k > 0) for (let i = 0; i < k; i += 1) acc *= 2;
  else for (let i = 0; i < -k; i += 1) acc *= 0.5;
  return acc;
}
export function pow(x, y) { if (x === 0) return 0; if (x === 1) return 1; return exp(y * ln(x)); }
function sign(x) { return x > 0 ? 1 : x < 0 ? -1 : x; }
export const f = v => { const r = Math.round(v * 100) / 100; return r === 0 ? "0" : String(r); };

/* ---------- the seeded generator ---------- */
export function seedFrom(str) { let h = 1779033703 ^ str.length; for (let i = 0; i < str.length; i++) { h = Math.imul(h ^ str.charCodeAt(i), 3432918353); h = (h << 13) | (h >>> 19); } h = Math.imul(h ^ (h >>> 16), 2246822507); h = Math.imul(h ^ (h >>> 13), 3266489909); return (h ^= h >>> 16) >>> 0; }
export function rngFor(key) { let a = seedFrom(key); return () => { a |= 0; a = (a + 0x6D2B79F5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }

/* ---------- colour and contrast ---------- */
export const ON_DARK = "#ffffff", ON_LIGHT = "#1b1726", WAIT = "#b26b00";
export const SIG_ALPHA = [0, 0.22, 0.3];
export function hexToRgb(h) { h = h.replace("#", ""); if (h.length === 3) h = h.split("").map(c => c + c).join(""); const n = parseInt(h, 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; }
export function rgbToHex(r, g, b) { return "#" + [r, g, b].map(v => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0")).join(""); }
export function normalize(h) { if (typeof h !== "string" || !/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.test(h)) return null; return rgbToHex(...hexToRgb(h)); }
export function luminance(h) { const w = [0.2126, 0.7152, 0.0722]; let a = 0; hexToRgb(h).forEach((c, i) => { const v = c / 255; a += w[i] * (v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)); }); return a; }
export function contrast(a, b) { const A = luminance(a), B = luminance(b); return (Math.max(A, B) + 0.05) / (Math.min(A, B) + 0.05); }
export function onColor(fill) { return contrast(fill, ON_DARK) >= contrast(fill, ON_LIGHT) ? ON_DARK : ON_LIGHT; }
export function mix(a, b, t) { const A = hexToRgb(a), B = hexToRgb(b); return rgbToHex(...A.map((v, i) => v + (B[i] - v) * t)); }
export function glyphContrast(fill) { const on = onColor(fill); return contrast(on, mix(fill, on, SIG_ALPHA[2])); }
export function passes(fill, ground) { return contrast(fill, ground) >= 3 && glyphContrast(fill) >= 3; }
export function nearestPassing(fill, ground) {
  const dirs = luminance(ground) > 0.4 ? ["#000000", "#ffffff"] : ["#ffffff", "#000000"];
  for (const towards of dirs) for (let t = 0; t <= 1.0001; t += 0.02) { const c = mix(fill, towards, t); if (passes(c, ground)) return c; }
  return dirs[0];
}

/* ---------- glyphs (24 px line icons) ---------- */
export const GLYPHS = {
  cake: '<path d="M4 20h16"/><path d="M5.5 20v-6.5h13V20"/><path d="M5.5 15.5c1.6 1.2 3.4 1.2 5 0 1.6 1.2 3.4 1.2 5 0 1 .8 2 .9 3 .5"/><path d="M12 13.5V10"/><path d="M12 7.6c-1-.9-1-2.1 0-3.4 1 1.3 1 2.5 0 3.4z"/>',
  loaf: '<path d="M6 12.2A3.6 3.6 0 0 1 6.8 5h10.4a3.6 3.6 0 0 1 .8 7.2V19a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1z"/><path d="M9.5 9.5l1.5 1.8"/><path d="M13 8.6l1.5 1.8"/>',
  bag: '<path d="M5.5 8.5h13l-1.1 11.5H6.6z"/><path d="M9 8.5V7a3 3 0 0 1 6 0v1.5"/>',
  pen: '<path d="M4.5 19.5l1-4L16 5l3 3L8.5 18.5z"/><path d="M14 7l3 3"/>',
  shield: '<path d="M12 3.5l7 2.8v5.4c0 4.2-2.9 7.3-7 8.8-4.1-1.5-7-4.6-7-8.8V6.3z"/><path d="M9 12l2.1 2.1L15.2 10"/>',
  gift: '<path d="M4 9.5h16v3.5H4z"/><path d="M5.5 13v7h13v-7"/><path d="M12 9.5V20"/><path d="M12 9.5C10.8 6.8 7.5 6 7.5 8c0 1.2 1.8 1.5 4.5 1.5z"/><path d="M12 9.5c1.2-2.7 4.5-3.5 4.5-1.5 0 1.2-1.8 1.5-4.5 1.5z"/>',
  perfume: '<path d="M9.5 3.5h5v3h-5z"/><path d="M10.5 6.5v2M13.5 6.5v2"/><path d="M6.5 11.5a3 3 0 0 1 3-3h5a3 3 0 0 1 3 3v7a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2z"/><path d="M9.5 14.5h5"/>',
  star: '<path d="M12 4l2.5 5.1 5.6.8-4 3.9.9 5.6L12 16.8l-5 2.6.9-5.6-4-3.9 5.6-.8z"/>',
  coin: '<circle cx="12" cy="12" r="8"/><path d="M12 7.5v9"/><path d="M14.4 9.7c-.4-.8-1.3-1.3-2.4-1.3-1.3 0-2.4.7-2.4 1.8s1.1 1.4 2.4 1.7 2.4.7 2.4 1.8-1.1 1.8-2.4 1.8c-1.1 0-2-.5-2.4-1.3"/>',
  news: '<path d="M4 5.5h12.5V19H6a2 2 0 0 1-2-2z"/><path d="M16.5 9H20v8a2 2 0 0 1-2 2h-1.5"/><path d="M7 9h6.5M7 12.5h6.5M7 16h4"/>',
  calendar: '<rect x="4" y="5.5" width="16" height="14.5" rx="2"/><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4"/><path d="M8 14h3"/>',
  key: '<circle cx="8.5" cy="15.5" r="4"/><path d="M11.3 12.7L19.5 4.5"/><path d="M16.5 7.5l2.2 2.2M14.5 9.5l1.8 1.8"/>',
  boxes: '<path d="M3.5 7.5L12 3.5l8.5 4L12 11.5z"/><path d="M3.5 7.5v9l8.5 4 8.5-4v-9"/><path d="M12 11.5v9"/>',
  chat: '<path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 16h-6.5L8 19.5V16H5a1.5 1.5 0 0 1-1.5-1.5v-8A1.5 1.5 0 0 1 5 5z"/>',
  pin: '<path d="M12 21s-6-5.5-6-11a6 6 0 0 1 12 0c0 5.5-6 11-6 11z"/><circle cx="12" cy="10" r="2"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="M20 20l-4-4"/>',
  bottle: '<path d="M10 3h4v3l2 2v11a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2V8l2-2z"/><path d="M8 12h8"/>',
  book: '<path d="M5 4h9a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z"/><path d="M5 17a3 3 0 0 1 3-3h9"/>',
  archive: '<path d="M4 5h16v4H4z"/><path d="M5 9v10h14V9"/><path d="M10 13h4"/>',
  ticket: '<path d="M4 7h16v3a2 2 0 0 0 0 4v3H4v-3a2 2 0 0 0 0-4z"/><path d="M14 7v10"/>',
  train: '<rect x="6" y="3" width="12" height="13" rx="3"/><path d="M6 10h12"/><path d="M9 20l-2 1M15 20l2 1"/><circle cx="9" cy="13" r=".8"/><circle cx="15" cy="13" r=".8"/>',
  leaf: '<path d="M5 19c0-8 5-13 14-14-1 9-6 14-14 14z"/><path d="M5 19l7-7"/>'
};
export const GLYPH_NAMES = { cake: "Cake", loaf: "Loaf", bag: "Bag", pen: "Pen", shield: "Shield", gift: "Gift", perfume: "Perfume", star: "Star", coin: "Coin", news: "Newspaper", calendar: "Calendar", key: "Key", boxes: "Boxes", chat: "Speech bubble", pin: "Pin", search: "Search", bottle: "Bottle", book: "Book", archive: "Archive box", ticket: "Ticket", train: "Train", leaf: "Leaf" };
export const FAMILIES = [["orbits", "Orbits"], ["grain", "Grain"], ["strata", "Strata"], ["weave", "Weave"], ["facets", "Facets"]];
export const STATES = ["idle", "listening", "working", "answering", "speaking", "waiting", "resting", "asleep", "paused", "frozen"];
export const SQUARENESS = [3.2, 8];

/* ---------- geometry ---------- */
export const esc = s => String(s).replace(/[&<>"']/g, ch => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch]));
const VB = 'viewBox="-12 -12 124 134"';
const dims = size => `width="${f(size * 1.24)}" height="${f(size * 1.34)}"`;
export function squircle(n, r) {
  const steps = 96, cx = 50, cy = 50;
  let d = "";
  for (let i = 0; i < steps; i += 1) {
    const t = (i / steps) * PI * 2, c = cos(t), s = sin(t);
    const x = cx + r * sign(c) * pow(Math.abs(c), 2 / n);
    const y = cy + r * sign(s) * pow(Math.abs(s), 2 / n);
    d += (i === 0 ? "M" : "L") + f(x) + " " + f(y);
  }
  return d + "Z";
}
function arc(r, a0, a1, color, w, op) {
  const p = a => [50 + r * cos(a * PI / 180), 50 + r * sin(a * PI / 180)];
  const [x0, y0] = p(a0), [x1, y1] = p(a1), large = (a1 - a0) > 180 ? 1 : 0;
  return `<path d="M${f(x0)} ${f(y0)}A${f(r)} ${f(r)} 0 ${large} 1 ${f(x1)} ${f(y1)}" fill="none" stroke="${color}" stroke-width="${f(w)}" stroke-linecap="round" opacity="${f(op)}"/>`;
}
function wave(x0, x1, y0, amp, lam, ph) {
  let d = "", first = true;
  for (let x = x0; x <= x1; x += 2.5) { d += (first ? "M" : "L") + f(x) + " " + f(y0 + amp * sin((x + ph) / lam * PI * 2)); first = false; }
  return d;
}

/* ---------- signature families ---------- */
const SIG = {
  orbits(r, c) {
    const k = 3 + Math.floor(r() * 2); let s = "";
    for (let i = 0; i < k; i += 1) {
      const rad = 16 + i * (34 / (k - 1)) + (r() - 0.5) * 3, sweep = 150 + r() * 170, start = r() * 360, w = 3.2 + r() * 2.6;
      s += arc(rad, start, start + sweep, c, w, 0.8);
    }
    const ma = r() * PI * 2, cx = f(50 + 31.7 * cos(ma)), cy = f(50 + 31.7 * sin(ma)), rr = f(4 + r() * 2);
    s += `<circle cx="${cx}" cy="${cy}" r="${rr}" fill="${c}"/>`;
    return { inner: s, motion: "spin" };
  },
  grain(r, c) {
    const n = 34 + Math.floor(r() * 24), sc = 5.6 + r() * 1.6, rot = r() * 360, ox = (r() - 0.5) * 14, oy = (r() - 0.5) * 14;
    let s = "";
    for (let i = 1; i <= n; i += 1) {
      const a = (i * 137.508 + rot) * PI / 180, d = sc * Math.sqrt(i);
      s += `<circle cx="${f(50 + ox + d * cos(a))}" cy="${f(50 + oy + d * sin(a))}" r="${f(1.1 + (i / n) * 2.6)}"/>`;
    }
    return { inner: `<g fill="${c}">${s}</g>`, motion: "spin" };
  },
  strata(r, c) {
    const bands = 4 + Math.floor(r() * 3), lam = 34 + r() * 30, amp = 3 + r() * 6, ang = -30 + r() * 60, gap = 150 / bands;
    let s = "";
    for (let i = 0; i < bands; i += 1) {
      const y0 = -25 + gap * (i + 0.5), ph = r() * lam;
      const d = wave(-50 - lam, 150 + lam, y0, amp, lam, ph), w = f(gap * (0.3 + r() * 0.22)), op = i % 2 !== 0 ? "1" : "0.6";
      s += `<path d="${d}" fill="none" stroke="${c}" stroke-width="${w}" stroke-linecap="round" opacity="${op}"/>`;
    }
    return { inner: `<g transform="rotate(${f(ang)} 50 50)"><g class="wa-drift" data-p="${f(-lam)}">${s}</g></g>`, motion: "drift" };
  },
  weave(r, c) {
    const sp = 9 + r() * 6, a1 = r() * 180, a2 = a1 + 55 + r() * 50, sw = 1.6 + r() * 1.4;
    const set = ang => { let s = ""; for (let x = -90; x <= 190; x += sp) { const fx = f(x); s += `<line x1="${fx}" y1="-70" x2="${fx}" y2="170"/>`; } return `<g transform="rotate(${f(ang)} 50 50)"><g class="wa-drift" data-p="${f(-sp)}">${s}</g></g>`; };
    return { inner: `<g stroke="${c}" stroke-width="${f(sw)}">${set(a1)}${set(a2)}</g>`, motion: "drift" };
  },
  facets(r, c) {
    const px = 50 + (r() - 0.5) * 36, py = 50 + (r() - 0.5) * 36, k = 7 + Math.floor(r() * 5);
    const angs = []; let a = r() * 360;
    for (let i = 0; i < k; i += 1) { angs.push(a); a += (360 / k) * (0.6 + r() * 0.8); }
    let tri = "", lines = "";
    for (let i = 0; i < angs.length; i += 1) {
      const b1 = angs[i] * PI / 180, b2 = angs[(i + 1) % angs.length] * PI / 180, R = 92;
      const p1 = [50 + R * cos(b1), 50 + R * sin(b1)], p2 = [50 + R * cos(b2), 50 + R * sin(b2)];
      if (i % 2 === 0) tri += `<path d="M${f(px)} ${f(py)}L${f(p1[0])} ${f(p1[1])}L${f(p2[0])} ${f(p2[1])}Z" fill="${c}" fill-opacity="${f(0.4 + r() * 0.45)}"/>`;
      lines += `<line x1="${f(px)}" y1="${f(py)}" x2="${f(p1[0])}" y2="${f(p1[1])}"/>`;
    }
    return { inner: `${tri}<g stroke="${c}" stroke-width="1.4">${lines}</g>`, motion: "spin" };
  }
};

/* ---------- the mark ---------- */
export const detailFor = size => size <= 24 ? 0 : size <= 44 ? 1 : 2;
const seedKey = spec => `${spec.site}:${spec.webagent}:${spec.signature.seed}`;
const units = s => String(s).length; /* UTF-16 code units, as Rust's encode_utf16().count() */

function glyphLayer(spec, detail, font, color) {
  const g = spec.glyph;
  if (g.kind === "monogram") {
    const fs = units(g.letters) > 1 ? "40" : detail === 0 ? "60" : "52";
    return `<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="${esc(font)}" font-size="${fs}" fill="${color}">${esc(g.letters)}</text>`;
  }
  const [gs, gx, sw] = detail === 0 ? ["60", "20", "2.7"] : detail === 1 ? ["50", "25", "2.3"] : ["44", "28", "2"];
  return `<svg x="${gx}" y="${gx}" width="${gs}" height="${gs}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round">${GLYPHS[g.id] || ""}</svg>`;
}

/* The layers on their own, for the Workshop's exploded view. sigColor and sigOpacity override
   the signature's colour and strength there; the published mark never uses them. */
export function layers(spec, size, { font = "", sigColor = null, sigOpacity = null } = {}) {
  const detail = detailFor(size), fill = spec.color.fill, on = onColor(fill), n = spec.silhouette.squareness;
  let sig = "", motion = "";
  if (detail > 0) {
    const p = SIG[spec.signature.family](rngFor(seedKey(spec)), sigColor || on);
    sig = `<g opacity="${f(sigOpacity != null ? sigOpacity : SIG_ALPHA[detail])}">${p.inner}</g>`;
    motion = p.motion;
  }
  return { detail, on, body: squircle(n, 50), halo: squircle(n, 58), sig, motion, glyphFor: color => glyphLayer(spec, detail, font, color) };
}

/* One WebAgent's mark. Same signature and output as agentrix_marks::render. */
export function markSVG(spec, { size = 40, state = "idle", font = "", uid = "1" } = {}) {
  const id = "wm" + uid, p = layers(spec, size, { font }), fillHex = spec.color.fill;
  const fill = p.detail === 2 ? `url(#${id}g)` : fillHex;
  return `<svg class="wa" data-state="${state}" data-motion="${p.motion}" ${dims(size)} ${VB} aria-hidden="true" focusable="false">` +
    `<defs><clipPath id="${id}c"><path d="${p.body}"/></clipPath><linearGradient id="${id}g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${mix(fillHex, "#ffffff", 0.16)}"/><stop offset="1" stop-color="${fillHex}"/></linearGradient></defs>` +
    `<path class="wa-halo" d="${p.halo}" fill="none" stroke="${fillHex}" stroke-width="4"/>` +
    `<g class="wa-core"><path d="${p.body}" fill="${fill}"/>` +
    (p.sig ? `<g clip-path="url(#${id}c)"><g class="wa-sig"><circle cx="50" cy="50" r="80" fill="none"/>${p.sig}</g></g>` : "") +
    p.glyphFor(p.on) + `</g>` +
    `<line class="wa-trace" x1="8" y1="117" x2="92" y2="117" stroke="${fillHex}" stroke-width="4" stroke-linecap="round"/>` +
    `<circle class="wa-dot" cx="96" cy="4" r="11" fill="${WAIT}" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/>` +
    `</svg>`;
}

/* The Chief WebAgent's team mark. Same output as agentrix_marks::render_team. */
export function teamSVG(team, { size = 44, state = "idle", busy = [], font = "" } = {}) {
  const ink = team.ink, on = onColor(ink), body = squircle(team.squareness, 50), halo = squircle(team.squareness, 58), k = team.members.length;
  let dots = `<circle cx="50" cy="50" r="33" fill="none" stroke="${on}" stroke-width="1.5" opacity=".25"/>`;
  team.members.forEach((m, i) => {
    const ang = (-90 + i * 360 / k) * PI / 180;
    dots += `<circle class="chief-dot${busy.includes(m.webagent) ? " busy" : ""}" data-webagent="${esc(m.webagent)}" cx="${f(50 + 33 * cos(ang))}" cy="${f(50 + 33 * sin(ang))}" r="7" fill="${esc(m.fill)}" stroke="${on}" stroke-width="2.4"/>`;
  });
  return `<svg class="wa chief" data-state="${state}" ${dims(size)} ${VB} aria-hidden="true" focusable="false">` +
    `<path class="wa-halo" d="${halo}" fill="none" stroke="${esc(ink)}" stroke-width="4"/>` +
    `<g class="wa-core"><path d="${body}" fill="${esc(ink)}"/>${dots}` +
    `<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="${esc(font)}" font-size="34" fill="${on}">${esc(team.letters)}</text></g>` +
    `<circle class="wa-dot" cx="96" cy="4" r="11" fill="${WAIT}" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/></svg>`;
}

/* ---------- live behaviour: states only from presence frames ---------- */
const SETTLE_MS = 5000;
const timers = new WeakMap();

/* Copies each drift group's distance into the CSS variable its keyframes read. */
function applyDrift(svg) { svg.querySelectorAll(".wa-drift[data-p]").forEach(g => g.style.setProperty("--p", g.getAttribute("data-p") + "px")); }

/* Sets a mark's state and restarts its 5-second settle. Speaking lasts as long as the voice. */
export function setState(svg, state) {
  if (!svg || !STATES.includes(state)) return;
  svg.setAttribute("data-state", state);
  restartSettle(svg);
}

/* Motion runs again from now and settles in 5 seconds, unless the mark is speaking. */
function restartSettle(svg) {
  svg.removeAttribute("data-settled");
  clearTimeout(timers.get(svg));
  if (svg.getAttribute("data-state") !== "speaking") timers.set(svg, setTimeout(() => svg.setAttribute("data-settled", ""), SETTLE_MS));
}

/* The voice level, 0 to 1, from the TTS stream (never simulated). */
export function setLevel(svg, level) { if (svg) svg.style.setProperty("--lvl", String(Math.max(0, Math.min(1, Number(level) || 0)))); }

/* Drives a mark's speaking ring from audio that is really playing (06 §4.3: the level is
   measured from the audio the browser plays, never simulated, so muting or stopping stills the
   ring). `media` is the <audio> or <video> element playing the voice. The mark speaks while the
   audio plays and goes back to the state it had when the audio pauses or ends. Returns a
   function that stops following. */
let voiceContext = null;
export function followVoice(svg, media) {
  const AC = typeof window !== "undefined" && (window.AudioContext || window.webkitAudioContext);
  if (!svg || !media || !AC) return () => {};
  voiceContext = voiceContext || new AC();
  const ctx = voiceContext;
  // A media element can be routed through Web Audio once; keep its analyser on the element.
  if (!media.__waAnalyser) {
    const source = ctx.createMediaElementSource(media);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 1024;
    source.connect(analyser);
    analyser.connect(ctx.destination);
    media.__waAnalyser = analyser;
  }
  const analyser = media.__waAnalyser, data = new Float32Array(analyser.fftSize);
  let prev = "idle", speaking = false, raf = 0, level = 0;
  const tick = () => {
    analyser.getFloatTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i += 1) sum += data[i] * data[i];
    const rms = Math.sqrt(sum / data.length);
    level = level * 0.6 + Math.min(1, rms * 5) * 0.4;
    setLevel(svg, media.muted ? 0 : level);
    raf = requestAnimationFrame(tick);
  };
  const start = () => {
    if (ctx.state === "suspended") ctx.resume();
    if (!speaking) { prev = svg.getAttribute("data-state") || "idle"; speaking = true; setState(svg, "speaking"); }
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(tick);
  };
  const stop = () => {
    cancelAnimationFrame(raf);
    if (!speaking) return;
    speaking = false;
    setLevel(svg, 0);
    setState(svg, prev === "speaking" ? "idle" : prev);
  };
  media.addEventListener("playing", start);
  media.addEventListener("pause", stop);
  media.addEventListener("ended", stop);
  if (!media.paused) start();
  return () => { stop(); media.removeEventListener("playing", start); media.removeEventListener("pause", stop); media.removeEventListener("ended", stop); };
}

/* On the team mark: a member's dot pulses while that WebAgent works or answers. */
export function setMember(teamSvg, webagent, state) {
  if (!teamSvg) return;
  const dot = [...teamSvg.querySelectorAll(".chief-dot")].find(d => d.getAttribute("data-webagent") === webagent);
  if (!dot) return;
  dot.classList.toggle("busy", state === "working" || state === "answering");
  restartSettle(teamSvg);
}

/* <wa-mark webagent="cakes" state="idle">: wraps a server-rendered SVG, or renders one from a
   `spec` attribute (MarkSpec JSON) with `size` and `font`. Presence frames set `state`; a
   team mark (attribute `team`) takes `member-<id>` updates through setMember. */
if (typeof customElements !== "undefined" && !customElements.get("wa-mark")) {
  let uid = 0;
  class WaMark extends HTMLElement {
    static get observedAttributes() { return ["state", "spec", "level"]; }
    get svg() { return this.querySelector("svg.wa"); }
    connectedCallback() {
      if (!this.svg && this.hasAttribute("spec")) this.draw();
      if (!this.svg) return;
      applyDrift(this.svg);
      if (this.hasAttribute("state")) setState(this.svg, this.getAttribute("state"));
      else restartSettle(this.svg);
    }
    draw() {
      let spec;
      try { spec = JSON.parse(this.getAttribute("spec")); } catch (_) { return; }
      const size = Number(this.getAttribute("size") || 40), state = this.getAttribute("state") || "idle";
      this.innerHTML = markSVG(spec, { size, state, font: this.getAttribute("font") || "", uid: "c" + (++uid) });
      applyDrift(this.svg);
      setState(this.svg, state);
    }
    attributeChangedCallback(name, old, value) {
      if (old === value || !this.isConnected) return;
      if (name === "spec") this.draw();
      else if (name === "state") setState(this.svg, value);
      else if (name === "level") setLevel(this.svg, value);
    }
  }
  customElements.define("wa-mark", WaMark);
}

/* Applies one presence frame to every mark on the page for that WebAgent or desk:
   { webagent, state, label?, level?, words? }. The Chief's team mark (<wa-mark team>) takes the
   Chief's own state ("coordinator") and pulses the dot of any member working or answering.
   Status lines marked data-presence-words="{webagent}" get the frame's words. */
export function applyPresence(frame, root = document) {
  if (!frame || !frame.webagent) return;
  root.querySelectorAll("wa-mark").forEach(el => {
    if (el.hasAttribute("team")) {
      if (frame.webagent === "coordinator") setState(el.svg, frame.state);
      else setMember(el.svg, frame.webagent, frame.state);
      return;
    }
    if (el.getAttribute("webagent") !== frame.webagent) return;
    if (frame.state) el.setAttribute("state", frame.state);
    if (frame.level != null) el.setAttribute("level", String(frame.level));
  });
  if (typeof frame.words === "string") {
    root.querySelectorAll("[data-presence-words]").forEach(el => {
      if (el.getAttribute("data-presence-words") === frame.webagent) el.textContent = frame.words;
    });
  }
}

/* Opens a presence stream (the console's /api/webagents/{site}/presence) and applies every
   frame. The first event is a snapshot of every member; then one frame per real change.
   Returns the EventSource, or null where there is none. */
export function connectPresence(url, root = document) {
  if (typeof EventSource === "undefined" || !url) return null;
  const es = new EventSource(url, { withCredentials: true });
  es.addEventListener("snapshot", e => { try { (JSON.parse(e.data).frames || []).forEach(f => applyPresence(f, root)); } catch (_) { /* a bad frame changes nothing */ } });
  es.addEventListener("presence", e => { try { applyPresence(JSON.parse(e.data), root); } catch (_) { /* a bad frame changes nothing */ } });
  return es;
}

/* The sample sites' first mark script (agentrix-themes/static/marks.js) drew elements with class
   "wa-living" and data-site, data-id, data-color, data-n, data-glyph, data-mono, data-family,
   data-seed, data-font, data-size and data-state, behind window.WAMarks. The same markup and API
   work here, drawn by this generator, so the site and the console show the same mark. */
export function specFromData(d) {
  const fill = normalize(d.color || "") || "#38356d";
  return {
    markSpec: 1, site: d.site || "", webagent: d.id || "",
    silhouette: { shape: "squircle", squareness: Math.min(SQUARENESS[1], Math.max(SQUARENESS[0], Number(d.n || 4.6))) },
    color: { fill, glyph: onColor(fill) },
    glyph: !d.glyph || d.glyph === "monogram" ? { kind: "monogram", letters: (d.mono || (d.id || "A").slice(0, 1).toUpperCase()).slice(0, 2), face: "site display" } : { kind: "role", id: d.glyph },
    signature: { family: d.family || "grain", seed: Number(d.seed || 1) }
  };
}
let livingUid = 0;
export function drawLiving(root = document) {
  root.querySelectorAll(".wa-living:not([data-drawn])").forEach(el => {
    const d = el.dataset, size = Number(d.size || 28), state = d.state || "idle";
    el.innerHTML = markSVG(specFromData(d), { size, state, font: d.font || "", uid: "l" + (++livingUid) });
    el.setAttribute("data-drawn", "");
    const svg = el.querySelector("svg.wa");
    applyDrift(svg);
    setState(svg, state);
  });
}
if (typeof window !== "undefined" && typeof document !== "undefined") {
  const svgOf = el => el && (el.matches("svg.wa") ? el : el.querySelector("svg.wa"));
  window.WAMarks = {
    draw: drawLiving,
    setState: (el, state) => setState(svgOf(el), state),
    specOf: el => specFromData(el.dataset),
    markSVG: (m, size, state) => markSVG(specFromData({ site: m.site, id: m.id, color: m.color, n: m.n, glyph: m.glyph, mono: m.mono, family: m.family, seed: m.seed }), { size, state, font: m.font || "", uid: "l" + (++livingUid) })
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => drawLiving(), { once: true }); else drawLiving();
}

/* A page opts in with <meta name="agentrix-presence" content="/api/webagents/{site}/presence">,
   so no inline script is needed. */
if (typeof document !== "undefined") {
  const start = () => { const m = document.querySelector('meta[name="agentrix-presence"]'); if (m) connectPresence(m.getAttribute("content")); };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start, { once: true }); else start();
}
