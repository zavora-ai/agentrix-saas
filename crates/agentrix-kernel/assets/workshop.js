/* The Workshop (ADR-031; 01b C13; concept/identity/marks.html "Workshop"): the owner tunes one
   WebAgent's or desk's mark from its card in the console.
     <wa-workshop site="kilele" webagent="editor"></wa-workshop>
   Colour (from the site's palette or any colour, with the two 3:1 checks and the nearest shade
   that passes), corners, glyph or monogram, the site's signature family and a fresh seed, a
   preview of every state with replay, and reset. The preview is drawn by marks.js, the same
   generator the server uses, so what the owner sees is what is saved. Nothing is saved until
   the owner presses Save. Text from the server is set with textContent. */
import { markSVG, onColor, contrast, glyphContrast, nearestPassing, normalize, GLYPHS, GLYPH_NAMES, FAMILIES, STATES, SQUARENESS, esc } from "./marks.js";

const SIZES = [20, 28, 40, 64, 96];
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
const clone = o => JSON.parse(JSON.stringify(o));
let uid = 0;
const mark = (spec, size, state, font) => `<wa-mark webagent="workshop" state="${esc(state)}">${markSVG(spec, { size, state, font, uid: "w" + (++uid) })}</wa-mark>`;

class Workshop extends HTMLElement {
  async connectedCallback() {
    if (this.dataset.ready) return;
    this.dataset.ready = "1";
    this.site = this.getAttribute("site");
    this.api = (this.getAttribute("api") || "/api/webagents") + "/" + encodeURIComponent(this.site);
    this.textContent = "Loading the Workshop…";
    try {
      const res = await fetch(this.api, { credentials: "same-origin", headers: { accept: "application/json" } });
      if (!res.ok) throw new Error((await res.json().catch(() => ({}))).error || "We couldn't load your WebAgents.");
      this.data = await res.json();
    } catch (e) {
      this.textContent = (e && e.message) || "We couldn't load your WebAgents. Try again.";
      return;
    }
    this.pick(this.getAttribute("webagent") || "editor");
  }

  /* Everyone the owner can tune: WebAgents on duty (the Concierge through its desks), then desks. */
  options() {
    const out = [];
    for (const m of this.data.members) if (m.status === "on-duty" && m.id !== "concierge") out.push({ id: m.id, name: m.spec.name, mandate: m.spec.mandate, mark: m.spec.mark, def: m.default_mark });
    for (const d of this.data.desks) out.push({ id: d.id, name: d.name, mandate: d.mandate, mark: d.mark, def: d.default_mark });
    return out;
  }

  pick(id) {
    const all = this.options();
    this.who = all.find(o => o.id === id) || all[0];
    this.saved = clone(this.who.mark);
    this.spec = clone(this.who.mark);
    this.state = "working";
    this.build();
  }

  build() {
    const look = this.data.look;
    this.textContent = "";
    this.classList.add("wsk");
    const form = el("form", "wsk-controls");
    form.setAttribute("autocomplete", "off");
    form.addEventListener("submit", e => e.preventDefault());

    const pickRow = el("div");
    const lbl = el("label", "wsk-lbl", "WebAgent");
    lbl.htmlFor = "wsk-who";
    const sel = el("select");
    sel.id = "wsk-who";
    for (const o of this.options()) { const opt = el("option", null, o.name); opt.value = o.id; sel.append(opt); }
    sel.value = this.who.id;
    sel.addEventListener("change", () => this.pick(sel.value));
    pickRow.append(lbl, sel);

    const colour = this.fieldset("Colour");
    const sw = el("div", "wsk-swatches");
    for (const c of look.palette) {
      const b = el("button", "wsk-swatch");
      b.type = "button"; b.dataset.color = c; b.title = c; b.setAttribute("aria-label", "Colour " + c);
      b.style.background = c;
      b.addEventListener("click", () => { this.spec.color.fill = c; this.update(); });
      sw.append(b);
    }
    const any = el("input");
    any.type = "color"; any.setAttribute("aria-label", "Any colour");
    any.addEventListener("input", () => { const c = normalize(any.value); if (c) { this.spec.color.fill = c; this.update(); } });
    sw.append(any);
    this.anyColour = any;
    this.fix = el("div", "wsk-fix");
    colour.append(sw, this.fix);

    const corners = el("div");
    const cl = el("label", "wsk-lbl", "Corners");
    cl.htmlFor = "wsk-corners";
    const range = el("input");
    range.type = "range"; range.id = "wsk-corners"; range.min = String(SQUARENESS[0]); range.max = String(SQUARENESS[1]); range.step = "0.1";
    range.addEventListener("input", () => { this.spec.silhouette.squareness = Math.round(Number(range.value) * 10) / 10; this.update(); });
    const ends = el("div", "wsk-ends");
    ends.append(el("span", null, "Softer"), el("span", null, "Squarer"));
    corners.append(cl, range, ends);
    this.range = range;

    const glyph = this.fieldset("Glyph");
    const tiles = el("div", "wsk-glyphs");
    for (const g of Object.keys(GLYPHS)) {
      const b = el("button", "wsk-tile");
      b.type = "button"; b.dataset.glyph = g; b.title = GLYPH_NAMES[g]; b.setAttribute("aria-label", GLYPH_NAMES[g]);
      b.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${GLYPHS[g]}</svg>`;
      b.addEventListener("click", () => { this.spec.glyph = { kind: "role", id: g }; this.update(); });
      tiles.append(b);
    }
    const mono = el("div", "wsk-mono");
    const monoBtn = el("button", "wsk-tile wsk-aa", "Aa");
    monoBtn.type = "button"; monoBtn.dataset.glyph = "monogram"; monoBtn.setAttribute("aria-label", "Monogram");
    const letters = el("input");
    letters.type = "text"; letters.maxLength = 2; letters.setAttribute("aria-label", "Monogram letters");
    const useMono = () => { const t = letters.value.trim().slice(0, 2); if (t) { this.spec.glyph = { kind: "monogram", letters: t, face: "site display" }; this.update(); } };
    monoBtn.addEventListener("click", () => { if (!letters.value.trim()) letters.value = this.who.name.slice(0, 1); useMono(); });
    letters.addEventListener("input", useMono);
    mono.append(monoBtn, letters);
    glyph.append(tiles, mono);
    this.letters = letters;

    const sig = this.fieldset("Signature");
    this.families = el("div", "wsk-families");
    sig.append(this.families);
    const note = el("p", "wsk-note", "The family is your whole site's, so your WebAgents read as one team. The seed is this one's.");
    const row = el("div", "wsk-row");
    const shuffle = el("button", "btn btn-sm", "Shuffle");
    shuffle.type = "button";
    shuffle.addEventListener("click", () => { this.spec.signature.seed = 1 + Math.floor(Math.random() * 9999); this.update(); });
    this.seedOut = el("span", "wsk-seed");
    row.append(shuffle, this.seedOut);
    sig.append(note, row);

    const st = this.fieldset("Preview a state");
    const seg = el("div", "wsk-seg");
    for (const s of STATES) {
      const b = el("button", "wsk-chip", this.data.words[s] ? this.data.words[s].name : s);
      b.type = "button"; b.dataset.state = s;
      b.addEventListener("click", () => { this.state = s; this.update(); });
      seg.append(b);
    }
    const replay = el("button", "btn btn-sm", "Replay the motion");
    replay.type = "button";
    replay.addEventListener("click", () => this.update());
    st.append(seg, replay, el("p", "wsk-note", "Motion settles after 5 seconds in one state; the words stay. Speaking lasts as long as the voice."));

    const reset = el("button", "btn btn-sm", "Reset to this WebAgent's defaults");
    reset.type = "button";
    reset.addEventListener("click", () => { if (this.who.def) { this.spec = clone(this.who.def); this.update(); } });

    form.append(pickRow, colour, corners, glyph, sig, st, reset);

    const preview = el("div", "wsk-preview");
    this.stage = el("div", "wsk-stage");
    this.sizes = el("div", "wsk-sizes");
    this.sizes.setAttribute("aria-label", "The same mark at five sizes");
    this.contexts = el("div", "wsk-contexts");
    this.checks = el("div", "wsk-checks");
    const actions = el("div", "wsk-actions");
    this.save = el("button", "btn btn-primary", "Save this look");
    this.save.type = "button";
    this.save.addEventListener("click", () => this.persist());
    this.discard = el("button", "btn", "Discard changes");
    this.discard.type = "button";
    this.discard.addEventListener("click", () => { this.spec = clone(this.saved); this.update(); });
    this.status = el("p", "wsk-status");
    this.status.setAttribute("role", "status");
    actions.append(this.save, this.discard, this.status);
    preview.append(this.stage, this.sizes, this.contexts, this.checks, actions);

    for (const [k, v] of [["--sbg", look.bg], ["--sink", look.ink], ["--sfont", look.font], ["--wa-ground", look.bg]]) this.style.setProperty(k, v);
    const grid = el("div", "wsk-grid");
    grid.append(form, preview);
    this.append(grid);
    this.update();
  }

  fieldset(name) { const f = el("fieldset"); f.append(el("legend", "wsk-lbl", name)); return f; }

  update() {
    const look = this.data.look, s = this.spec, font = look.font;
    s.color.glyph = onColor(s.color.fill);
    // Controls follow the spec.
    this.querySelectorAll(".wsk-swatch").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.color.toLowerCase() === s.color.fill.toLowerCase())));
    this.anyColour.value = s.color.fill;
    this.range.value = String(s.silhouette.squareness);
    const g = s.glyph.kind === "monogram" ? "monogram" : s.glyph.id;
    this.querySelectorAll(".wsk-tile").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.glyph === g)));
    if (s.glyph.kind === "monogram") this.letters.value = s.glyph.letters;
    this.querySelectorAll(".wsk-chip").forEach(b => b.setAttribute("aria-pressed", String(b.dataset.state === this.state)));
    this.seedOut.textContent = "Seed " + s.signature.seed;
    this.families.textContent = "";
    for (const [k, label] of FAMILIES) {
      const b = el("button", "wsk-family");
      b.type = "button"; b.setAttribute("aria-pressed", String(k === s.signature.family));
      b.innerHTML = mark(Object.assign(clone(s), { signature: { family: k, seed: s.signature.seed } }), 32, "idle", font);
      b.append(el("span", null, label));
      b.addEventListener("click", () => { s.signature.family = k; this.update(); });
      this.families.append(b);
    }
    // The stage: the mark, its name with the AI label, and the words beside it.
    const w = this.data.words[this.state] || { owner: "", visitor: "" };
    const tool = "The label of the tool it's running";
    const owner = this.state === "working" ? tool : (w.owner || "The mark only");
    const visitor = this.state === "working" ? tool : (w.visitor || "The mark only");
    this.stage.innerHTML = `<div class="wsk-big">${mark(s, 140, this.state, font)}</div>`;
    const cap = el("div", "wsk-cap");
    const nm = el("div", "wsk-name", this.who.name);
    nm.append(el("span", "wsk-ai", "AI"));
    const dl = el("dl");
    for (const [t, d] of [["The owner sees", owner], ["A visitor sees", visitor], ["Mandate", this.who.mandate]]) { const div = el("div"); div.append(el("dt", null, t), el("dd", null, d)); dl.append(div); }
    cap.append(nm, dl);
    this.stage.append(cap);
    this.sizes.innerHTML = SIZES.map(z => `<figure>${mark(s, z, this.state, font)}<figcaption>${z} px</figcaption></figure>`).join("");
    this.contexts.innerHTML =
      `<div class="wsk-ctx wsk-ctx-site"><div class="wsk-ctx-lbl">On the site</div><div class="wsk-peek">${mark(s, 40, this.state, font)}<b></b><span class="wsk-ai">AI</span></div></div>` +
      `<div class="wsk-ctx"><div class="wsk-ctx-lbl">In the console dock</div><div class="wsk-peek">${mark(s, 40, this.state, font)}<b></b></div></div>` +
      `<div class="wsk-ctx"><div class="wsk-ctx-lbl">Round crops</div><div class="wsk-round">${mark(s, 34, "idle", font)}</div></div>`;
    this.contexts.querySelectorAll(".wsk-peek b").forEach(b => { b.textContent = this.who.name; });
    // The two checks the platform keeps (ADR-031).
    const a = contrast(s.color.fill, look.bg), b = glyphContrast(s.color.fill);
    this.checks.textContent = "";
    const pill = (ok, text) => el("span", "chip " + (ok ? "chip-ok" : "chip-wait"), (ok ? "Passes · " : "Too faint · ") + text);
    this.checks.append(pill(a >= 3, `mark on the site ${a.toFixed(1)}:1`), pill(b >= 3, `glyph on the mark, pattern drawn ${b.toFixed(1)}:1`));
    this.fix.textContent = "";
    const passing = a >= 3 && b >= 3;
    if (!passing) {
      const f = el("button", "btn btn-sm", "Use the nearest shade that passes");
      f.type = "button";
      f.addEventListener("click", () => { s.color.fill = nearestPassing(s.color.fill, look.bg); this.update(); });
      this.fix.append(f);
    }
    const changed = JSON.stringify(s) !== JSON.stringify(this.saved);
    this.save.disabled = !changed || !passing;
    this.discard.disabled = !changed;
    const familyNote = s.signature.family !== this.saved.signature.family ? "Saving changes the pattern family for every WebAgent on your site. " : "";
    this.status.textContent = changed ? familyNote + "Not saved yet." : "";
  }

  async persist() {
    this.save.disabled = true;
    this.status.textContent = "Saving…";
    try {
      const res = await fetch(this.api + "/marks", {
        method: "POST", credentials: "same-origin",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ webagent: this.who.id, mark: this.spec })
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        if (body.suggestion) this.spec.color.fill = body.suggestion;
        this.update();
        this.status.textContent = body.error || "We couldn't save that look. Nothing changed. Try again.";
        return;
      }
      this.saved = clone(this.spec);
      this.who.mark = clone(this.spec);
      this.update();
      this.status.textContent = "Saved. The console shows it now; your site shows it from its next release.";
    } catch (_) {
      this.update();
      this.status.textContent = "We couldn't reach the server. Nothing changed. Try again.";
    }
  }
}

if (!customElements.get("wa-workshop")) customElements.define("wa-workshop", Workshop);
