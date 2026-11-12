/* Kilele's cake studio island (K-04). Ported from concept/experience/creator.html.
   Every option, price, day and refusal comes from the page (server-rendered from the live price
   rules, slots and the manifest). Picks are priced by POST /cakes/quote: rules, no model.
   The Cakes desk (ADR-032) talks through POST /cakes/talk, a model behind the gateway whose
   every pick the server checks against the price list; when it can't answer, the page matches
   the words itself and says so, without the AI label. POST /cakes/preview draws an idea of the
   cake with AI; the sketch stays as the instant view and the fallback.
   Server text is set with textContent only; markup comes from fixed parts. */
(() => {
  const $ = (s, r = document) => r.querySelector(s), $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const root = $("#studio");
  if (!root) return;
  const D = (() => { try { return JSON.parse($("#studio-data").textContent); } catch { return null; } })();
  if (!D) return;
  const G = D.groups, OWNER = D.owner || "the owner", ZONE = D.zone;
  const OPT = {};
  Object.entries(G).forEach(([g, list]) => list.forEach(o => { OPT[o.code] = Object.assign({ group: g }, o); }));
  const DAYS = Object.fromEntries(D.days.map(d => [d.date, d]));
  const STEPS = ["size", "sponge", "colour", "finish", "deco", "message", "fulfilment", "date"];
  const S = { size: null, sponge: null, colour: null, finish: null, deco: null, message: "", msgAsked: false, fulfilment: null, date: null, note: "", recipient: "", occasion: "", theme: "" };
  const STORE = "kilele-studio";
  let Q = null, current = "size", busy = false, held = null, refusal = "", key = null, seq = 0;
  // The desk's model: "unknown" until it answers or fails, then "up" or "down". Taps skip it
  // while it's down; typed words always try it, since the server falls back quickly.
  let model = "unknown", aiShown = false;
  const T = [];

  const thread = $("#thread"), body = $("#talkBody"), input = $("#q");
  const studioMark = $("#studioMark"), reqMark = $("#reqMark");
  const presence = (el, state) => { if (el && window.WAMarks) window.WAMarks.setState(el, state); };
  const toast = msg => { if (window.WADesks && window.WADesks.toast) window.WADesks.toast(msg); };
  const lower = s => (s || "").toLowerCase();
  const cap = s => s ? s[0].toUpperCase() + s.slice(1) : s;
  const label = code => (OPT[code] ? OPT[code].label : "");
  const list = (items, joiner = "and") => items.length < 2 ? (items[0] || "") : `${items.slice(0, -1).join(", ")} ${joiner} ${items[items.length - 1]}`;

  /* ---- The thread ---- */
  const scroll = () => { body.scrollTop = body.scrollHeight; };
  const rich = text => {
    const frag = document.createDocumentFragment(), re = /\*\*(.+?)\*\*/g;
    let last = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
      const b = document.createElement("b"); b.textContent = m[1]; frag.appendChild(b);
      last = re.lastIndex;
    }
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    return frag;
  };
  const byLine = $("#thread .wa-a .by");
  const DESK = D.desk_name || "Cakes";
  /* The AI label goes up the moment a model answers, before its words show, and never before. */
  function showAi() {
    if (aiShown) return; aiShown = true;
    [$("#deskName"), $("#deskSub"), $("#disclose")].forEach(el => { if (el && el.dataset.ai) el.textContent = el.dataset.ai; });
    const chip = $("#deskAi"); if (chip) chip.hidden = false;
  }
  function say(phrases, meta, ai) {
    const box = document.createElement("div"); box.className = "wa-a";
    const by = byLine.cloneNode(true);
    by.querySelectorAll(".wa-living").forEach(m => { m.textContent = ""; m.removeAttribute("data-drawn"); });
    const name = by.querySelector(".by-name"); if (name) name.textContent = ai ? DESK : "Cake studio";
    if (ai) { const chip = document.createElement("span"); chip.className = "wa-ai"; chip.textContent = "AI"; by.append(" ", chip); }
    const txt = document.createElement("div"); txt.className = "txt";
    phrases.filter(Boolean).forEach(p => { const el = document.createElement("p"); el.appendChild(rich(p)); txt.appendChild(el); });
    box.append(by, txt);
    if (meta) { const m = document.createElement("div"); m.className = "wa-meta"; m.textContent = meta; box.appendChild(m); }
    thread.appendChild(box);
    if (window.WAMarks) window.WAMarks.draw(box);
    remembered("desk", phrases.filter(Boolean).join(" ").replace(/\*\*/g, ""));
    scroll();
    return box;
  }
  function visitor(text) { const v = document.createElement("div"); v.className = "wa-v"; v.textContent = text; thread.appendChild(v); remembered("visitor", text); scroll(); }
  function remembered(who, text) { T.push({ who, text: String(text).slice(0, 300) }); if (T.length > 10) T.shift(); }

  const QUESTION = {
    size: "How many people will it feed?",
    sponge: `Which sponge? ${D.gf_only || ""}`.trim(),
    colour: "What colour buttercream?",
    finish: "And the finish?",
    deco: "Any decoration on top?",
    message: `What should the cake say? Up to ${D.max_chars} characters. Type it below, or pick one.`,
    fulfilment: ZONE ? `Delivery or collection? Delivery to ${ZONE.areas} is ${ZONE.fee}. Collection is free, from the bakery at ${D.address}.` : `Collection is from the bakery at ${D.address}.`,
    date: `When do you need it? These are the days ${OWNER} has room for.`,
  };

  /* ---- Chips and the calendar ---- */
  const boxes = Object.fromEntries($$(".chips[data-step]").map(c => [c.dataset.step, c]));
  const whyDay = d => { const bySize = d.why[S.size || ""] || d.why[""]; return (bySize && bySize[S.fulfilment || ""]) || ""; };
  function showChips(step) { Object.entries(boxes).forEach(([k, c]) => { c.hidden = k !== step; }); }
  function refreshChips() {
    $$(".chip-opt[data-group]").forEach(b => {
      b.setAttribute("aria-pressed", String(S[b.dataset.group] === b.dataset.code));
      const only = b.dataset.only ? b.dataset.only.split(" ") : null;
      const off = !!(only && S.size && !only.includes(S.size));
      b.disabled = off;
      if (off) b.title = b.dataset.why; else b.removeAttribute("title");
    });
    $$(".chip-opt[data-fulfilment]").forEach(b => b.setAttribute("aria-pressed", String(S.fulfilment === b.dataset.fulfilment)));
    $$(".day").forEach(b => {
      const d = DAYS[b.dataset.date]; if (!d) return;
      const why = whyDay(d);
      if (why) { b.setAttribute("aria-disabled", "true"); b.title = why; } else { b.removeAttribute("aria-disabled"); b.removeAttribute("title"); }
      b.querySelector(".why").textContent = why;
      b.setAttribute("aria-label", why ? `${d.label}: ${why}` : d.label);
      b.setAttribute("aria-pressed", String(S.date === d.date));
    });
  }

  /* ---- The sketch: composed from approved parts, no model ---- */
  const svgEl = $("#cake"), NS = "http://www.w3.org/2000/svg";
  const SPONGE = { "sponge.vanilla": "#f3dcae", "sponge.lemon": "#f2e08a", "sponge.chocolate": "#6b3b22", "sponge.redvelvet": "#9b2b33", "sponge.gf": "#e9cf9a" };
  const hex = c => (/^#[0-9a-f]{3,8}$/i.test(c || "") ? c : null);
  function drawCake() {
    const size = OPT[S.size], tiers = Math.min(Math.max(size ? size.tiers : 1, 1), 3);
    const col = hex(OPT[S.colour] && OPT[S.colour].swatch) || "#fbf1de", sp = SPONGE[S.sponge] || "#f3dcae";
    const fin = (S.finish || "").replace("finish.", ""), deco = (S.deco || "").replace("deco.", "");
    const W = [230, 176, 124], H = [74, 62, 52]; let y = 262;
    let out = `<defs><linearGradient id="st-ombre" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff8ee"/><stop offset="1" stop-color="${col}"/></linearGradient></defs>`;
    out += `<ellipse cx="170" cy="268" rx="150" ry="16" fill="#e8d6bd"/><ellipse cx="170" cy="264" rx="138" ry="12" fill="#fff" stroke="#e2cfb4"/>`;
    const tops = [];
    for (let i = 0; i < tiers; i++) {
      const w = W[i], h = H[i], x = 170 - w / 2; y -= h;
      out += `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10" fill="${fin === "ombre" ? "url(#st-ombre)" : col}" stroke="rgba(59,36,22,.18)"/>`;
      if (fin === "naked") {
        for (let k = 1; k < 3; k++) out += `<rect x="${x + 4}" y="${y + k * h / 3 - 4}" width="${w - 8}" height="7" rx="3" fill="${sp}" opacity=".85"/>`;
        out += `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10" fill="#fff" opacity=".35"/>`;
      }
      tops.push({ x, y, w });
    }
    const top = tops[tops.length - 1];
    if (fin === "drip") {
      let d = `M${top.x + 4} ${top.y + 4}`; const n = 9;
      for (let k = 0; k <= n; k++) { const px = top.x + 4 + k * (top.w - 8) / n; d += ` L${px} ${top.y + 6 + (k % 2 ? 22 : 12)} L${px + (top.w - 8) / n / 2} ${top.y + 8}`; }
      out += `<path d="${d} L${top.x + top.w - 4} ${top.y + 4} Z" fill="#4a2615"/><rect x="${top.x}" y="${top.y - 2}" width="${top.w}" height="10" rx="5" fill="#4a2615"/>`;
    }
    if (deco === "berries") {
      [[-34, -6], [-12, -10], [10, -7], [30, -9], [-22, -14], [20, -15], [0, -17]].forEach(([dx, dy], k) => {
        out += `<circle cx="${170 + dx}" cy="${top.y + dy + 6}" r="${k % 3 ? 7 : 8}" fill="${k % 2 ? "#c2263a" : "#3b2a6b"}"/><circle cx="${170 + dx - 2}" cy="${top.y + dy + 3}" r="2" fill="#fff" opacity=".6"/>`;
      });
      tops.slice(0, -1).forEach(t => { for (let k = 0; k < 5; k++) out += `<circle cx="${t.x + 18 + k * (t.w - 36) / 4}" cy="${t.y + 4}" r="5" fill="#c2263a"/>`; });
    }
    if (deco === "flowers") {
      const fl = (cx, cy, c) => [0, 72, 144, 216, 288].map(a => `<circle cx="${cx + 6 * Math.cos(a * Math.PI / 180)}" cy="${cy + 6 * Math.sin(a * Math.PI / 180)}" r="5" fill="${c}"/>`).join("") + `<circle cx="${cx}" cy="${cy}" r="3.5" fill="#f2c14e"/>`;
      out += fl(140, top.y - 2, "#f7f2f5") + fl(170, top.y - 8, "#e98aa0") + fl(198, top.y - 1, "#f7f2f5");
      tops.slice(0, -1).forEach(t => { out += fl(t.x + 22, t.y + 8, "#e98aa0") + fl(t.x + t.w - 22, t.y + 8, "#f7f2f5"); });
    }
    if (deco === "sprinkles") {
      const cs = ["#e05a6e", "#f2c14e", "#4aa3d8", "#6dbb6a", "#fff"];
      tops.forEach(t => { for (let k = 0; k < 16; k++) { const px = t.x + 10 + ((k * 37) % (t.w - 20)), py = t.y + 10 + ((k * 23) % 40); out += `<rect x="${px}" y="${py}" width="7" height="2.6" rx="1.3" fill="${cs[k % 5]}" transform="rotate(${(k * 47) % 180} ${px} ${py})"/>`; } });
    }
    let plaque = null;
    if (S.message) {
      const pw = Math.min(top.w - 14, 150), px = 170 - pw / 2, py = top.y + (tiers === 1 ? 26 : 20);
      out += `<rect x="${px}" y="${py}" width="${pw}" height="22" rx="11" fill="#fffaf0" stroke="#d9b98f"/>`;
      plaque = py + 15;
    }
    if (S.sponge === "sponge.gf") out += `<rect x="16" y="226" width="58" height="22" rx="11" fill="#fff" stroke="#cfe3c9"/><text x="45" y="241" text-anchor="middle" font-family="DM Sans, sans-serif" font-size="10.5" font-weight="700" fill="#24561f">GF sponge</text>`;
    svgEl.innerHTML = out;
    if (plaque !== null) {
      const t = document.createElementNS(NS, "text");
      Object.entries({ x: 170, y: plaque, "text-anchor": "middle", "font-family": "Fraunces, Georgia, serif", "font-size": 11, "font-weight": 700, fill: "#3b2416" }).forEach(([k, v]) => t.setAttribute(k, v));
      const max = 22; t.textContent = S.message.length > max ? S.message.slice(0, max - 1) + "…" : S.message;
      svgEl.appendChild(t);
    }
    const parts = [size ? size.label : "1 tier", label(S.sponge), label(S.colour), label(S.finish), S.deco && S.deco !== "deco.none" ? label(S.deco) : ""].filter(Boolean).map(lower);
    svgEl.setAttribute("aria-label", `Sketch of your cake: ${parts.join(", ")}${S.message ? `, with a plaque that reads ${S.message}` : ""}`);
    ideaFollow();
  }

  /* ---- An idea of the cake, made with AI from the checked options (POST /cakes/preview) ----
     A picture needs the size, sponge and buttercream colour. An unchosen finish or decoration is
     drawn as Kilele's plain default, and the desk says so. The desk draws it itself in the
     conversation; the button does the same, or says exactly what is missing. Never silent. */
  const ideaFig = $("#idea"), ideaImg = $("#ideaImg"), ideaTag = $("#ideaTag"), ideaBtn = $("#ideaBtn"), viewBtn = $("#viewBtn"), ideaLine = $("#ideaLine"), ideaPlaque = $("#ideaPlaque"), sketchTag = $("#sketchTag");
  const IDEAS = {}, DEF = D.idea_defaults || {};
  let shown = null, viewing = "sketch", drawing = false, autoKey = "";
  const NEEDS = [["size", "the size"], ["sponge", "the sponge"], ["colour", "the buttercream colour"]];
  const ideaMissing = () => NEEDS.filter(([k]) => !S[k]).map(([, w]) => w);
  const ideaReady = () => ideaMissing().length === 0;
  const looks = () => [S.size, S.sponge, S.colour, S.finish || DEF.finish || null, S.deco || DEF.deco || null];
  const designKey = () => JSON.stringify([...looks(), !!S.message]);
  const missingWords = () => `To draw it, pick ${list(ideaMissing())} first. The finish and decoration can wait.`;
  function view(which) {
    viewing = which && shown ? which : "sketch";
    const idea = viewing === "idea";
    ideaFig.hidden = !idea; svgEl.style.display = idea ? "none" : ""; sketchTag.hidden = idea;
    viewBtn.hidden = !shown; viewBtn.textContent = idea ? "Show the sketch" : "Show the idea";
    ideaPlaque.hidden = !(idea && S.message);
    ideaPlaque.textContent = S.message ? `On the plaque: \u201c${S.message}\u201d. The picture leaves it blank; ${OWNER} pipes the words.` : "";
  }
  function ideaFollow() {
    if (!ideaBtn) return;
    const k = designKey();
    if (shown && shown.key !== k) { shown = IDEAS[k] || null; if (shown) showIdea(shown, true); else view("sketch"); }
    ideaBtn.disabled = drawing || held !== null || !!(shown && shown.key === k);
    ideaBtn.textContent = drawing ? "Drawing…" : shown && shown.key === k ? "Idea shown" : "Show me an idea of it";
    if (!drawing) ideaLine.textContent = !ideaReady() ? "Once the size, sponge and buttercream colour are picked, the desk draws an idea of it with AI."
      : shown ? "Drawn from your choices only, not from what you typed."
      : "Draws a picture of these choices with AI. It's an idea, not a photo.";
    if (viewing === "idea") view("idea");
  }
  function showIdea(j, quiet) {
    shown = j;
    ideaImg.src = j.url; ideaImg.alt = j.alt || "An idea of your cake, made with AI";
    ideaTag.textContent = j.label;
    view("idea");
    if (!quiet) ideaLine.textContent = "Drawn from your choices only, not from what you typed.";
  }
  if (ideaImg) ideaImg.addEventListener("error", () => { if (!shown) return; delete IDEAS[shown.key]; shown = null; view("sketch"); ideaFollow(); ideaLine.textContent = "The picture didn't load. The sketch shows your choices."; });
  if (viewBtn) viewBtn.addEventListener("click", () => view(viewing === "idea" ? "sketch" : "idea"));
  /* Draws (or reuses) the idea for the design as it stands. Returns the picture when a new one
     is shown, null otherwise; every outcome is said in the line under the picture. */
  async function drawIdea() {
    if (drawing) return null;
    if (held) { ideaLine.textContent = `This cake is already with ${OWNER}.`; return null; }
    if (!ideaReady()) { ideaLine.textContent = missingWords(); return null; }
    const k = designKey();
    if (IDEAS[k]) { showIdea(IDEAS[k]); ideaFollow(); return null; }
    drawing = true; ideaFollow();
    ideaLine.textContent = "Drawing an idea of your cake with AI. It takes up to a minute; the sketch shows it meanwhile.";
    // The server fills an unchosen finish or decoration itself, so it can say so.
    const { size, sponge, colour, finish, deco } = S;
    const ctl = window.AbortController ? new AbortController() : null;
    const timer = setTimeout(() => { if (ctl) ctl.abort(); }, 90000);
    let r = null, j = {};
    try {
      r = await fetch("/cakes/preview", { method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", signal: ctl ? ctl.signal : undefined,
        body: JSON.stringify({ size, sponge, colour, finish, deco, message: S.message }) });
      j = await r.json().catch(() => ({}));
    } catch { r = null; }
    clearTimeout(timer);
    drawing = false;
    const ok = !!(r && r.ok && j.url);
    if (ok) { j.key = k; IDEAS[k] = j; if (designKey() === k) showIdea(j); }
    ideaFollow();
    if (!ok) ideaLine.textContent = (j && j.words) || (r ? "The picture couldn't be drawn just now. The sketch shows your choices; try again in a minute." : "The picture took too long, so it isn't shown. The sketch shows your choices; try again in a minute.");
    return ok && designKey() === k ? j : null;
  }
  /* The desk shows the idea in the conversation, with its label and what it filled in. */
  function ideaInChat(j) {
    const box = say([j.note || "Here's an idea of it."], "", aiShown);
    const fig = document.createElement("figure"); fig.className = "chat-idea";
    const img = document.createElement("img"); img.src = j.url; img.alt = j.alt || "An idea of your cake, made with AI"; img.width = 768; img.height = 768; img.decoding = "async";
    img.addEventListener("load", scroll);
    const cap = document.createElement("figcaption"); cap.textContent = j.label;
    fig.append(img, cap);
    (box.querySelector(".txt") || box).appendChild(fig);
    scroll();
  }
  /* The desk's picture tool: draws when asked, or when a ready design's look changed. */
  async function autoIdea(asked) {
    if (!ideaReady() || held) return;
    const k = JSON.stringify(looks());
    if (!asked && k === autoKey) return;
    autoKey = k;
    const known = IDEAS[designKey()];
    if (known) { showIdea(known); ideaFollow(); if (asked) ideaInChat(known); return; }
    const j = await drawIdea();
    if (j) ideaInChat(j);
  }
  if (ideaBtn) ideaBtn.addEventListener("click", async () => { const j = await drawIdea(); if (j) ideaInChat(j); });

  /* ---- The cake card, the spec and the request summary ---- */
  const dayLabel = d => (DAYS[d] ? DAYS[d].label : d);
  function spec() {
    const v = {
      size: S.size ? `${label(S.size)} · serves ${OPT[S.size].serves}` : "",
      sponge: label(S.sponge), colour: label(S.colour), finish: label(S.finish), deco: label(S.deco),
      message: S.message ? `“${S.message}”` : (S.msgAsked ? "No message" : ""),
      fulfilment: S.fulfilment === "delivery" && ZONE ? `Delivery, ${ZONE.label} · ${ZONE.fee}` : (S.fulfilment === "collection" ? "Collection from the bakery" : ""),
      date: S.date ? ((Q && Q.slot) || dayLabel(S.date)) : "",
    };
    $$("#spec button").forEach(b => {
      const val = v[b.dataset.row];
      b.classList.toggle("unset", !val);
      b.querySelector("span").textContent = val || "Not chosen yet";
    });
    const occ = (S.occasion ? `${S.occasion} cake` : "Your cake") + (S.theme ? `, ${S.theme} theme` : "");
    $("#cakeTitle").textContent = S.message ? `${occ} · “${S.message}”` : occ;
    const cake = ["size", "sponge", "colour", "finish", "deco"].map(k => lower(label(S[k]))).filter(Boolean);
    $("#rCake").textContent = cake.length ? cap(cake.join(" · ")) : "Not chosen yet";
    $("#rMessage").textContent = S.message ? `“${S.message}”` : "None";
    $("#rWhenLabel").textContent = S.fulfilment === "delivery" ? "Delivery" : S.fulfilment === "collection" ? "Collection" : "When";
    $("#rWhen").textContent = S.date ? ((Q && Q.slot) || dayLabel(S.date)) : "Not chosen yet";
    const row = $("#rRecipientRow"); if (row) row.hidden = S.fulfilment !== "delivery";
  }

  function flags(list) {
    const box = $("#flags"); box.textContent = "";
    list.forEach(f => { const el = document.createElement("div"); el.className = `flag${f.level === "error" ? " error" : ""}`; el.textContent = f.text; box.appendChild(el); });
  }

  const needsRecipient = () => S.fulfilment === "delivery" && !S.recipient.trim();
  function renderQuote() {
    const lines = $("#lines"); lines.textContent = "";
    const li = (cls, a, b, rule) => {
      const el = document.createElement("li"); if (cls) el.className = cls;
      const l = document.createElement("span"); l.textContent = a;
      if (rule) { const r = document.createElement("span"); r.className = "rule"; r.textContent = rule; l.appendChild(r); }
      el.appendChild(l);
      if (b !== null) { const v = document.createElement("span"); v.textContent = b; el.appendChild(v); }
      lines.appendChild(el);
    };
    if (!Q || !Q.has_size) li("empty", "Pick a size to see the price.", null);
    else {
      Q.lines.forEach(l => li("", l.label, l.amount, l.code));
      li("tot", "Total", Q.total);
      li("usd", `At Kilele's rate of ${Q.rate}`, Q.usd);
    }
    $("#cakeTotal").textContent = Q && Q.has_size ? Q.total : "";
    const sv = $("#cakeServes"); sv.hidden = !(Q && Q.serves); sv.textContent = (Q && Q.serves) || "";
    $("#rPrice").textContent = Q && Q.has_size ? `${Q.total}, which is ${Q.usd} at Kilele's rate of ${Q.rate}` : "From the price list once you pick a size";
    $("#nextUsd").textContent = Q && Q.has_size ? `: ${Q.usd} to Kilele Bakery` : "";
    const all = (Q ? Q.flags.slice() : []);
    if (Q && Q.complete && needsRecipient() && ZONE) all.push({ level: "note", text: `Add who receives the cake and where, in ${ZONE.areas}.` });
    if (refusal) all.unshift({ level: "error", text: refusal });
    flags(all);
    const btn = $("#requestBtn");
    if (!held) btn.disabled = !(Q && Q.complete && !needsRecipient());
    $("#shareBtn").disabled = !S.size;
    spec();
  }

  const payload = () => ({ size: S.size, sponge: S.sponge, colour: S.colour, finish: S.finish, deco: S.deco, message: S.message, date: S.date, note: S.note, fulfilment: S.fulfilment, recipient: S.recipient, idea: shown && shown.key === designKey() ? shown.digest : null });
  const post = (url, data) => fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(data), credentials: "same-origin" });

  async function quote() {
    const my = ++seq;
    presence(studioMark, "working");
    try {
      const r = await post("/cakes/quote", payload());
      if (my !== seq) return;
      if (!r.ok) throw new Error(String(r.status));
      const q = await r.json();
      if (my !== seq) return;
      Q = q;
      renderQuote();
    } catch {
      if (my !== seq) return;
      Q = null; renderQuote();
      flags([{ level: "error", text: "The price didn't load, so the request is off for now. Pick again, or use the simple cake form." }]);
    } finally {
      if (my === seq) presence(studioMark, held ? "waiting" : "listening");
    }
  }

  async function update() {
    refusal = ""; key = null;
    drawCake(); spec(); refreshChips(); remember();
    await quote();
  }

  /* ---- One question at a time ---- */
  const isSet = k => (k === "message" ? !!S.message || S.msgAsked : !!S[k]);
  function ask(step) {
    current = step || STEPS.find(k => !isSet(k)) || null;
    input.placeholder = current === "message" ? `Type the message, up to ${D.max_chars} characters` : "Describe it in your own words…";
    if (!current) { showChips(null); return review(); }
    if (current === "message") S.msgAsked = true;
    say([QUESTION[current]]);
    refreshChips(); showChips(current);
  }
  /* After the desk's own question: its chips, or the summary when nothing is missing. */
  function chipsFor(step) {
    current = step && step !== "occasion" ? step : (STEPS.find(k => !isSet(k)) || null);
    input.placeholder = current === "message" ? `Type the message, up to ${D.max_chars} characters` : "Describe it in your own words…";
    if (!step && !current) { showChips(null); return review(); }
    if (current === "message") S.msgAsked = true;
    refreshChips(); showChips(step === "occasion" ? "size" : current);
  }
  function review() {
    const parts = ["sponge", "colour", "finish", "deco"].map(k => lower(label(S[k]))).filter(Boolean);
    const err = Q && Q.flags.find(f => f.level === "error");
    say([
      `Here's your cake: **${lower(label(S.size))}**, ${list(parts)}${S.message ? `, saying “${S.message}”` : ""}.`,
      err ? `One thing to fix first: ${err.text}` : (Q && Q.has_size ? `**${Q.total}** from the price list. This price holds until ${Q.valid_until}.` : ""),
      needsRecipient() ? "Say who receives it and where in the request card, then request it." : `Add a note for ${OWNER} if you like, then request it. Nothing is charged until she confirms.`,
    ], Q ? `Quote from Kilele's price list · valid until ${Q.valid_until}` : "");
  }

  async function pick(k, v, text) {
    if (busy) return;
    if (held) { say([`This cake is already with ${OWNER} as **${held}**. Ask for changes on your order page.`]); return; }
    busy = true;
    visitor(text);
    S[k] = v;
    if (k === "message") S.msgAsked = true;
    const quoting = update();
    const turn = model === "down" ? null : await talk(text, true);
    await quoting;
    if (turn) await answer(turn);
    busy = false;
    if (!turn) ask();
    if (["size", "sponge", "colour", "finish", "deco"].includes(k)) autoIdea(false);
  }

  root.addEventListener("click", e => {
    const chip = e.target.closest(".chip-opt");
    if (chip && !chip.disabled) {
      if (chip.dataset.group) return pick(chip.dataset.group, chip.dataset.code, OPT[chip.dataset.code] ? OPT[chip.dataset.code].label : chip.textContent.trim());
      if (chip.dataset.message !== undefined) return pick("message", chip.dataset.message, chip.dataset.message || "No message");
      if (chip.dataset.fulfilment) return pick("fulfilment", chip.dataset.fulfilment, chip.dataset.fulfilment === "delivery" ? "Delivery" : "Collection");
    }
    const day = e.target.closest(".day");
    if (day) {
      const d = DAYS[day.dataset.date]; if (!d) return;
      const why = whyDay(d);
      if (why) { visitor(d.label); say([`${d.label} can't take this cake: ${why}.`]); return; }
      return pick("date", d.date, d.label);
    }
    const row = e.target.closest("#spec button");
    if (row && !busy) {
      if (held) { say([`This cake is already with ${OWNER} as **${held}**. Ask for changes on your order page.`]); return; }
      ask(row.dataset.row);
    }
  });

  /* ---- Free text → approved options, matched on this page (no model) ---- */
  const WORDS = {
    "size.1": /\b(one|1|single)[- ]tier/, "size.2": /\b(two|2)[- ]tier/, "size.3": /\b(three|3)[- ]tier/,
    "sponge.gf": /gluten|coeliac|celiac/, "sponge.redvelvet": /red velvet/, "sponge.lemon": /lemon|citrus/, "sponge.vanilla": /vanilla/, "sponge.chocolate": /chocolate(?! drip| buttercream| icing| frosting)|\bchoc\b/,
    "colour.forest": /forest|green|jungle|leaf/, "colour.blush": /pink|blush/, "colour.sky": /\bblue\b|\bsky\b/, "colour.chocolate": /chocolate (buttercream|icing|frosting)|brown icing/, "colour.cream": /\bcream\b|\bwhite\b|ivory/,
    "finish.drip": /drip/, "finish.ombre": /ombr[eé]|fade|gradient/, "finish.naked": /naked|rustic/, "finish.smooth": /smooth/,
    "deco.berries": /berr|fruit/, "deco.flowers": /flower|floral|blossom|\broses?\b/, "deco.sprinkles": /sprinkle|rainbow|confetti/, "deco.none": /no decoration|nothing on top|plain top/,
  };
  const NOT = [
    { re: /toadstool|mushroom|unicorn|figur|sculpt|fondant|\b3d\b|character|topper|statue|model of/, item: /figure|sculpt/i },
    { re: /photo|picture|print|edible image|selfie|\blogo\b/, item: /photo|print/i },
    { re: /tomorrow|today|tonight|this evening|in an hour|right now|same day/, item: /notice|sooner/i },
  ];
  // A short label two groups share ("Chocolate" sponge and buttercream) only matches by its own words.
  const SHORTS = {};
  Object.values(OPT).forEach(o => { const k = lower(o.short); SHORTS[k] = (SHORTS[k] || new Set()).add(o.group); });
  const plain = o => o.short.length > 3 && SHORTS[lower(o.short)].size === 1;
  const ordinal = n => { const s = ["th", "st", "nd", "rd"], v = n % 100; return n + (s[(v - 20) % 10] || s[v] || s[0]); };
  function mapWords(raw) {
    const t = lower(raw), got = [], no = [];
    let big = null;
    if (/birthday|turns|bday|turning/.test(t)) S.occasion = "Birthday";
    else if (/wedding/.test(t)) S.occasion = "Wedding";
    else if (/graduat/.test(t)) S.occasion = "Graduation";
    else if (/anniversar/.test(t)) S.occasion = "Anniversary";
    const n = +((t.match(/(\d+)\s*(people|guests|kids|children|persons|pax|of us)/) || [])[1] || 0);
    if (n) {
      const fit = G.size.find(o => +(o.serves.split(/[–-]/).pop()) >= n);
      if (fit) { S.size = fit.code; got.push(`${lower(fit.label)} for about ${n}`); }
      else big = n;
    }
    ["size", "sponge", "colour", "finish", "deco"].forEach(g => {
      if (g === "size" && n) return;
      const hits = G[g].filter(o => (WORDS[o.code] && WORDS[o.code].test(t)) || (plain(o) && t.includes(lower(o.short))));
      const o = hits.find(h => h.only_sizes.length) || hits[0];
      if (!o) return;
      S[g] = o.code; got.push(lower(o.label));
      if (g === "colour" && o.code === "colour.forest" && !S.finish && OPT["finish.ombre"] && !/smooth|naked|drip/.test(t)) { S.finish = "finish.ombre"; got.push(lower(label("finish.ombre"))); }
    });
    NOT.forEach(x => { if (x.re.test(t)) { const item = (D.not_offered || []).find(i => x.item.test(i)); if (item && !no.includes(item)) no.push(item); } });
    const nm = raw.match(/(?:daughter|son|niece|nephew|friend|wife|husband|mum|mom|dad|sister|brother|for)\s+([A-Z][a-z]+)\s+(?:turns|is turning|will be)\s+(\d{1,3})/i);
    if (nm && !S.message) {
      const m = `Happy ${ordinal(+nm[2])}, ${cap(nm[1].toLowerCase())}`;
      if ([...m].length <= D.max_chars) { S.message = m; S.msgAsked = true; got.push(`the message “${m}”`); }
    }
    return { got, no, big };
  }

  async function free(text) {
    if (busy) return;
    if (held) { visitor(text); say([`This cake is already with ${OWNER} as **${held}**. Ask for changes on your order page.`]); return; }
    busy = true;
    visitor(text);
    if (current === "message") {
      const len = [...text].length;
      if (len > D.max_chars) { say([`That's ${len} characters; the cake fits ${D.max_chars}. Try a shorter one.`]); busy = false; return; }
      S.message = text; S.msgAsked = true;
      say([`I've put **“${text}”** on the cake. Change it any time.`]);
      await update(); busy = false; return ask();
    }
    const turn = await talk(text, false);
    if (turn) { await answer(turn); busy = false; return; }
    const { got, no, big } = mapWords(text);
    const ph = [];
    if (got.length) ph.push(`From Kilele's options I've picked ${list(got.map(g => `**${g}**`))}.`);
    if (no.length) ph.push(`Kilele doesn't make ${list(no.map(lower), "or")}, so I won't promise that.`);
    if (big) {
      const top = G.size[G.size.length - 1];
      ph.push(`The largest cake on the price list is **${lower(top.label)}**, for ${top.serves}. For ${big} people ${OWNER} quotes it herself; ask her from the Cakes desk.`);
    }
    if (!ph.length) ph.push("I couldn't match that to Kilele's options. Pick from these, or say it another way.");
    say(ph, "Matched to Kilele's own options on this page, without AI");
    await update();
    busy = false;
    ask();
    if (got.length) autoIdea(false);
  }

  /* ---- The Cakes desk (POST /cakes/talk): a model, every pick checked by the server ---- */
  const SETS = ["size", "sponge", "colour", "finish", "deco", "message", "fulfilment", "date"];
  const specOut = () => ({ size: S.size, sponge: S.sponge, colour: S.colour, finish: S.finish, deco: S.deco, message: S.message, msg_asked: S.msgAsked, fulfilment: S.fulfilment, date: S.date, occasion: S.occasion, occasion_asked: true, theme: S.theme });
  let talks = 0;
  async function talk(text, picked) {
    const my = ++talks;
    presence(studioMark, "working");
    const ctl = window.AbortController ? new AbortController() : null;
    const timer = setTimeout(() => { if (ctl) ctl.abort(); }, 17000);
    try {
      const r = await fetch("/cakes/talk", { method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", signal: ctl ? ctl.signal : undefined,
        body: JSON.stringify({ text, picked, thread: T.slice(0, -1).slice(-8), spec: specOut() }) });
      if (!r.ok) throw new Error(String(r.status));
      const j = await r.json();
      if (j.by !== "model") { model = "down"; return null; }
      model = "up";
      return j;
    } catch { model = "down"; return null; }
    finally { clearTimeout(timer); if (my === talks) presence(studioMark, held ? "waiting" : "listening"); }
  }
  async function answer(j) {
    let changed = false;
    SETS.forEach(k => {
      const v = j.set && j.set[k];
      if (v === null || v === undefined || v === "") return;
      if (k === "message") { S.message = v; S.msgAsked = true; changed = true; return; }
      if (k === "fulfilment" && !(v === "collection" || (v === "delivery" && ZONE))) return;
      if (k === "date" && !DAYS[v]) return;
      if (k !== "fulfilment" && k !== "date" && !(OPT[v] && OPT[v].group === k)) return;
      if (S[k] !== v) { S[k] = v; changed = true; }
    });
    if (j.occasion) { S.occasion = j.occasion; changed = true; }
    if (j.theme) { S.theme = j.theme; changed = true; }
    if (changed) await update();
    showAi();
    const box = say(j.say || [], j.meta || "", true);
    if (j.pass) passOffer(box, j.pass);
    chipsFor(j.ask || null);
    if (j.draw) autoIdea(true);
  }
  /* A question the options don't cover goes to the owner only when the visitor sends it. */
  function passOffer(box, pass) {
    const acts = document.createElement("div"); acts.className = "acts";
    const open = document.createElement("button"); open.type = "button"; open.className = "act primary"; open.textContent = pass.label;
    acts.appendChild(open); box.appendChild(acts);
    open.addEventListener("click", () => {
      acts.remove();
      const f = document.createElement("form"); f.className = "pass-form";
      const q = document.createElement("textarea"); q.rows = 2; q.maxLength = 600; q.value = pass.question; q.setAttribute("aria-label", `Your question for ${OWNER}`);
      const c = document.createElement("input"); c.autocomplete = "email"; c.placeholder = "Email or WhatsApp number (optional)"; c.setAttribute("aria-label", `How ${OWNER} should reply`);
      const send = document.createElement("button"); send.className = "act primary"; send.type = "submit"; send.textContent = `Send to ${OWNER}`;
      f.append(q, c, send); box.appendChild(f); q.focus();
      f.addEventListener("submit", async e => {
        e.preventDefault(); if (!q.value.trim()) return; send.disabled = true;
        try {
          const r = await post("/ask/pass", { question: q.value.trim(), contact: c.value, desk: DESK, page: location.pathname });
          const j = await r.json(); f.remove();
          const done = document.createElement("div"); done.className = "wa-meta"; done.textContent = j.words || "Sent."; box.appendChild(done);
        } catch { send.disabled = false; toast("That didn't send. Check your connection and try again."); }
      });
    });
  }
  $("#form").addEventListener("submit", e => { e.preventDefault(); const t = input.value.trim(); if (!t) return; input.value = ""; free(t.slice(0, 300)); });

  /* ---- Note and recipient ---- */
  const note = $("#rNote"), recip = $("#rRecipient");
  note.addEventListener("input", () => { S.note = note.value; remember(); });
  if (recip) recip.addEventListener("input", () => { S.recipient = recip.value; remember(); renderQuote(); });

  /* ---- Save & share: the design lives in the link, nothing on the server ---- */
  const design = () => { const o = {}; ["size", "sponge", "colour", "finish", "deco", "fulfilment", "date", "occasion", "theme"].forEach(k => { if (S[k]) o[k] = S[k]; }); if (S.msgAsked) o.message = S.message; return o; };
  const enc = o => { let s = ""; new TextEncoder().encode(JSON.stringify(o)).forEach(c => { s += String.fromCharCode(c); }); return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); };
  const dec = t => { const s = atob(t.replace(/-/g, "+").replace(/_/g, "/")); return JSON.parse(new TextDecoder().decode(Uint8Array.from(s, c => c.charCodeAt(0)))); };
  const shareUrl = () => `${location.origin}/cakes/studio?d=${enc(design())}`;
  function remember() {
    if (!S.size || held) return;
    try { history.replaceState(null, "", `/cakes/studio?d=${enc(design())}`); } catch { /* history is optional */ }
  }
  function restore(o) {
    if (!o || typeof o !== "object") return false;
    let any = false;
    ["size", "sponge", "colour", "finish", "deco"].forEach(g => { if (OPT[o[g]] && OPT[o[g]].group === g) { S[g] = o[g]; any = true; } });
    if (typeof o.message === "string") { S.message = o.message.slice(0, 200); S.msgAsked = true; }
    if (o.fulfilment === "collection" || (o.fulfilment === "delivery" && ZONE)) S.fulfilment = o.fulfilment;
    if (DAYS[o.date]) S.date = o.date;
    const words = (v, n) => (typeof v === "string" && /^[\p{L}\p{N} ,'-]+$/u.test(v) ? v.slice(0, n) : "");
    S.occasion = words(o.occasion, 30); S.theme = words(o.theme, 40);
    if (typeof o.note === "string") { S.note = o.note.slice(0, 240); note.value = S.note; }
    if (typeof o.recipient === "string" && recip) { S.recipient = o.recipient.slice(0, 160); recip.value = S.recipient; }
    return any;
  }
  $("#shareBtn").addEventListener("click", () => {
    const box = $("#shareBox"), link = $("#shareLink");
    link.value = shareUrl(); box.hidden = false; link.focus(); link.select();
  });
  $("#copyLink").addEventListener("click", () => {
    const link = $("#shareLink"); link.value = shareUrl();
    const done = () => toast("Link copied. It opens this design in the studio, priced from the live price list.");
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(link.value).then(done, () => { link.select(); toast("Select the link and copy it."); });
    else { link.select(); toast("Select the link and copy it."); }
  });

  /* ---- Request this cake: a held request, nothing charged ---- */
  const btn = $("#requestBtn"), line = $("#reqLine"), pill = $("#reqPill"), link = $("#reqLink");
  btn.addEventListener("click", async () => {
    if (held || btn.disabled) return;
    key = key || (crypto.randomUUID ? crypto.randomUUID() : `k${Date.now()}${Math.random().toString(36).slice(2)}`);
    btn.disabled = true; btn.textContent = "Sending…";
    presence(reqMark, "working"); line.textContent = `Sending your request to ${OWNER}`;
    let r, j = {};
    try { r = await post("/cakes/request", Object.assign(payload(), { idempotency_key: key })); j = await r.json().catch(() => ({})); } catch { r = null; }
    if (r && r.status === 401) {
      try { sessionStorage.setItem(STORE, JSON.stringify(Object.assign(design(), { note: S.note, recipient: S.recipient }))); } catch { /* storage is optional */ }
      line.textContent = "Taking you to sign in. Your design is kept.";
      location.href = j.signin || D.sign_in;
      return;
    }
    if (!r || !r.ok) {
      refusal = (j && j.refused) || "The request didn't go through, and nothing was charged. Try again in a moment.";
      btn.textContent = "Request this cake"; presence(reqMark, "idle");
      line.textContent = "Nothing was sent.";
      renderQuote();
      say([refusal]);
      return;
    }
    held = j.order_id;
    ideaFollow();
    try { sessionStorage.removeItem(STORE); } catch { /* storage is optional */ }
    btn.textContent = `Requested · ${j.order_id}`; btn.classList.add("done"); btn.disabled = true;
    presence(reqMark, "waiting"); presence(studioMark, "waiting");
    line.textContent = j.status;
    pill.textContent = j.pill; pill.hidden = false;
    link.href = j.url; link.textContent = `Open ${j.order_id}`; link.hidden = false;
    visitor("Request this cake");
    say([`Done. Request **${j.order_id}** is **waiting for ${OWNER}'s OK**.`, `${j.total} (${j.usd}). Nothing has been charged. When ${OWNER} approves, the payment request appears on your order page.`], `Held for ${OWNER}`);
  });

  /* ---- Start: from a shared link, or back from sign-in ---- */
  let restored = false, from = "";
  const d = new URLSearchParams(location.search).get("d");
  if (d) { try { restored = restore(dec(d)); from = "link"; } catch { restored = false; } }
  if (!restored) { try { const saved = sessionStorage.getItem(STORE); if (saved) { restored = restore(JSON.parse(saved)); from = "signin"; } } catch { restored = false; } }
  if (window.WAMarks) window.WAMarks.draw(root);
  drawCake(); refreshChips();
  if (restored) {
    const back = D.signed_in ? `Welcome back, ${D.visitor}. Your design is as you left it.` : "Your design is as you left it.";
    say([from === "signin" ? back : "Here's the design from the link, priced again from today's price list."]);
    update().then(() => ask());
  } else {
    renderQuote();
  }
})();
