/* Desks, the Ask bar and the rail on a sample site (01a S-1 to S-4).
   Ported from concept/experience/nav-agents.html and experience.js, answering from the real
   POST /ask stream. Server text is set with textContent only; markup comes from fixed parts. */
(() => {
  const $ = (s, r = document) => r.querySelector(s), $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const svg = p => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${p}</svg>`;
  const I = {
    send: svg('<path d="M5 12h14M13 6l6 6-6 6"/>'),
    close: svg('<path d="M6 6l12 12M18 6L6 18"/>'),
    mic: svg('<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>'),
    handover: svg('<path d="M4 7h12l-3-3M20 17H8l3 3"/>'),
  };
  const data = (() => { try { return JSON.parse($("#wa-desks").textContent); } catch { return { desks: [], front: {}, site: {} }; } })();
  const SITE = data.site || {};
  const DESKS = Object.fromEntries((data.desks || []).map(d => [d.id, d]));
  const FRONT = data.front || { id: "front", name: "Front desk", color: "#3b2416", glyph: "search", seed: 1 };
  const deskOf = d => DESKS[d && d.id] || (d && d.id === "front" ? FRONT : d) || FRONT;
  const owner = document.body.dataset.owner || "the owner";

  $$(".icon-btn:empty, .peek-x").forEach(b => { if (!b.innerHTML.trim()) b.innerHTML = I.close; });
  $$(".wa-send").forEach(b => { if (!b.innerHTML.trim()) b.innerHTML = I.send; });
  $$(".wa-mic").forEach(b => { if (!b.innerHTML.trim()) b.innerHTML = I.mic; });

  const markSpan = (d, size) => {
    const s = document.createElement("span");
    s.className = "wa-living"; s.setAttribute("aria-hidden", "true");
    Object.assign(s.dataset, { size: String(size), color: d.color || "#38356d", glyph: d.glyph || "chat", seed: String(d.seed || 1), id: d.id || "", site: SITE.mark_key || "", family: SITE.family || "grain", n: String(SITE.squareness || 4.6), mono: (d.name || "?")[0] });
    if (window.WAMarks) window.WAMarks.draw(s.parentNode || (() => { const w = document.createElement("span"); w.appendChild(s); window.WAMarks.draw(w); return w; })());
    return s;
  };
  const presence = (el, state) => { if (el && window.WAMarks) window.WAMarks.setState(el, state); };

  const toast = msg => {
    let t = $(".toast");
    if (!t) { t = document.createElement("div"); t.className = "toast"; t.setAttribute("role", "status"); document.body.appendChild(t); }
    t.textContent = msg; t.classList.add("show");
    clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove("show"), 3200);
  };

  // Point at the cited place: same page scrolls and lights it up; another page opens at it.
  let lastPointed = null, pointTimer = null;
  const point = (href, label) => {
    const url = new URL(href, location.href);
    if (url.pathname !== location.pathname) { location.href = url.pathname + url.hash; return; }
    const el = url.hash ? document.getElementById(decodeURIComponent(url.hash.slice(1))) : null;
    if (!el) return;
    if (lastPointed) { lastPointed.classList.remove("pointed"); lastPointed.removeAttribute("data-pointer"); }
    el.classList.add("pointed"); el.setAttribute("data-pointer", label);
    el.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "center" });
    lastPointed = el; clearTimeout(pointTimer);
    pointTimer = setTimeout(() => { el.classList.remove("pointed"); el.removeAttribute("data-pointer"); }, 5200);
  };

  // "**6 sourdough** left [1]" → text, <b> and citation chips, built safely.
  const renderPhrase = (text, onCite) => {
    const frag = document.createDocumentFragment();
    const re = /\*\*(.+?)\*\*|\[(\d+)\]/g; let last = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
      if (m[1] !== undefined) { const b = document.createElement("b"); b.textContent = m[1]; frag.appendChild(b); }
      else { const a = document.createElement("a"); a.href = "#"; a.className = "cite"; a.dataset.n = m[2]; a.textContent = m[2]; a.addEventListener("click", e => { e.preventDefault(); onCite(m[2]); }); frag.appendChild(a); }
      last = re.lastIndex;
    }
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    return frag;
  };

  const line = (box, cls, text) => {
    const s = document.createElement("div"); s.className = cls;
    s.innerHTML = I.handover; const t = document.createElement("span"); t.textContent = text; s.appendChild(t);
    box.appendChild(s); return s;
  };

  const post = (url, body) => fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body), credentials: "same-origin" });

  // An order card in the conversation: the lines, the total, and one button that puts them in
  // the bag and goes to checkout, where the customer pays with PayPal.
  function payCard(act) {
    const c = act.card, card = document.createElement("div"); card.className = "pay-card";
    card.style.cssText = "margin:10px 0 4px;border:1.5px solid rgba(0,0,0,.14);border-radius:14px;padding:12px 14px;background:rgba(255,255,255,.75);font-size:.92em";
    const head = document.createElement("div"); head.textContent = "Your order";
    head.style.cssText = "font-weight:700;letter-spacing:.04em;text-transform:uppercase;font-size:.78em;opacity:.7;margin-bottom:6px";
    card.appendChild(head);
    (c.lines || []).forEach(l => {
      const row = document.createElement("div"); row.style.cssText = "display:flex;justify-content:space-between;gap:12px;padding:3px 0";
      const name = document.createElement("span"); name.textContent = `${l.qty} \u00d7 ${l.name}`;
      const amt = document.createElement("span"); amt.textContent = l.amount; amt.style.fontVariantNumeric = "tabular-nums";
      row.append(name, amt); card.appendChild(row);
    });
    const tot = document.createElement("div"); tot.style.cssText = "display:flex;justify-content:space-between;gap:12px;padding:7px 0 2px;margin-top:5px;border-top:1px solid rgba(0,0,0,.12);font-weight:700";
    const tl = document.createElement("span"); tl.textContent = "Total";
    const tv = document.createElement("span"); tv.textContent = c.usd ? `${c.total} (${c.usd})` : c.total;
    tot.append(tl, tv); card.appendChild(tot);
    const b = document.createElement("button"); b.type = "button"; b.className = "act primary"; b.textContent = act.label;
    b.style.cssText = "width:100%;margin-top:10px;justify-content:center";
    b.addEventListener("click", async () => {
      b.disabled = true; b.textContent = "Placing your order\u2026";
      try {
        for (const l of c.lines || []) {
          await fetch("/bag/add", { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ sku: l.sku, qty: String(l.qty), back: "/bag" }), credentials: "same-origin" });
        }
        location.href = act.href || "/checkout";
      } catch { b.disabled = false; b.textContent = act.label; toast("The order didn't go through. Try again."); }
    });
    card.appendChild(b);
    if (c.note) { const n = document.createElement("div"); n.textContent = c.note; n.style.cssText = "margin-top:8px;font-size:.84em;opacity:.7"; card.appendChild(n); }
    return card;
  }

  function passForm(box, question, desk) {
    if (box.querySelector(".pass-form")) return;
    const f = document.createElement("form"); f.className = "pass-form";
    f.innerHTML = '<label class="sr-only">How should they reply?</label><input autocomplete="email" placeholder="Email or WhatsApp number (optional)"><button class="act primary" type="submit"></button>';
    f.querySelector("button").textContent = `Send to ${owner}`;
    f.addEventListener("submit", async e => {
      e.preventDefault(); f.querySelector("button").disabled = true;
      try {
        const r = await post("/ask/pass", { question, contact: f.querySelector("input").value, desk: desk.name, page: location.pathname });
        const j = await r.json(); f.remove(); line(box, "wa-handover", j.words || "Sent.");
      } catch { f.querySelector("button").disabled = false; toast("That didn't send. Check your connection and try again."); }
    });
    box.appendChild(f); f.querySelector("input").focus();
  }

  async function doHold(box, act, button) {
    const name = (document.body.dataset.visitor || "") || prompt("Name for the hold?", "") || "";
    if (!name.trim()) return;
    button.disabled = true;
    try {
      const r = await post("/hold", { sku: act.sku, qty: act.qty, name });
      const j = await r.json();
      box.querySelectorAll(".act").forEach(x => x.disabled = true);
      line(box, "wa-handover", j.words);
    } catch { button.disabled = false; toast("The hold didn't go through. Try again."); }
  }

  // "Was this right?": recorded for the owner (POST /ask/feedback); a No can carry a short note.
  async function markAnswer(box, a, desk, right, button, note) {
    const buttons = box.querySelectorAll(".wa-meta button");
    buttons.forEach(x => { x.disabled = true; x.removeAttribute("aria-pressed"); });
    try {
      const r = await post("/ask/feedback", { answer: a.id || "", right, note: note || "", desk: desk.id || "", question: a.q || "" });
      const j = await r.json();
      toast(j.words || "Thanks.");
      if (!j.ok) { buttons.forEach(x => x.disabled = false); return; }
      if (button) button.setAttribute("aria-pressed", "true");
      buttons.forEach(x => { if (x !== button) x.disabled = false; });
      const old = box.querySelector(".fb-note");
      if (right || note) { if (old) old.remove(); return; }
      if (old) return;
      const f = document.createElement("form"); f.className = "pass-form fb-note";
      f.innerHTML = '<label class="sr-only">What was wrong?</label><input maxlength="280" autocomplete="off" placeholder="What was wrong? (optional)"><button class="act" type="submit">Send note</button>';
      f.addEventListener("submit", e => { e.preventDefault(); const t = f.querySelector("input").value.trim(); if (t) markAnswer(box, a, desk, false, button, t); });
      box.appendChild(f); f.querySelector("input").focus();
    } catch { buttons.forEach(x => x.disabled = false); toast("That didn't send. Check your connection and try again."); }
  }

  function renderAnswer(thread, a, mark) {
    const desk = deskOf(a.desk);
    if (a.handover) line(thread, "wa-handover", a.handover);
    const box = document.createElement("div"); box.className = "wa-a";
    const by = document.createElement("div"); by.className = "by";
    by.appendChild(markSpan(desk, 20)); by.appendChild(document.createTextNode(` ${desk.name} `));
    // AI only when a model wrote the words; "Instant" when a capability answered from records.
    const chipWords = a.basis === "ai" ? "AI" : a.basis === "instant" ? "Instant" : "";
    if (chipWords) { const chip = document.createElement("span"); chip.className = "wa-ai"; chip.textContent = chipWords; by.appendChild(chip); }
    box.dataset.desk = desk.name || "";
    const txt = document.createElement("div"); txt.className = "txt";
    box.appendChild(by); box.appendChild(txt); thread.appendChild(box);
    if (window.WAMarks) window.WAMarks.draw(by);
    const cite = n => { const s = (a.sources || []).find(x => String(x.n) === String(n)); if (s) point(s.href, `Cited [${s.n}]`); };
    (a.phrases || []).forEach((p, i) => {
      const s = document.createElement("span"); s.className = "phrase";
      s.appendChild(renderPhrase(p, cite)); s.appendChild(document.createTextNode(" "));
      if (!reduced) s.style.animationDelay = `${i * 140}ms`;
      txt.appendChild(s);
    });
    if ((a.sources || []).length) {
      const srcs = document.createElement("div"); srcs.className = "srcs";
      a.sources.forEach(s => {
        const b = document.createElement("button"); b.type = "button"; b.className = "src" + (s.lock ? " lock" : "");
        const n = document.createElement("b"); n.textContent = s.n; b.appendChild(n); b.appendChild(document.createTextNode(s.label));
        if (s.href) b.addEventListener("click", () => point(s.href, `Cited [${s.n}]`)); else b.disabled = true; // the owner's own answer points at no page
        srcs.appendChild(b);
      });
      box.appendChild(srcs);
    }
    (a.actions || []).filter(x => x.kind === "pay" && x.card).forEach(x => box.appendChild(payCard(x)));
    if ((a.actions || []).some(x => x.kind !== "pay")) {
      const acts = document.createElement("div"); acts.className = "acts";
      a.actions.filter(x => x.kind !== "pay").forEach(x => {
        const el = document.createElement(x.kind === "link" ? "a" : "button");
        el.className = "act" + (x.primary ? " primary" : ""); el.textContent = x.label;
        if (x.kind === "link") el.href = x.href; else el.type = "button";
        if (x.kind === "pass") el.addEventListener("click", () => passForm(box, x.question, desk));
        if (x.kind === "hold") el.addEventListener("click", () => doHold(box, x, el));
        acts.appendChild(el);
      });
      box.appendChild(acts);
    }
    if (a.meta) {
      const meta = document.createElement("div"); meta.className = "wa-meta";
      const m = document.createElement("span"); m.textContent = a.meta; meta.appendChild(m);
      const fb = document.createElement("span"); fb.append("Was this right? ");
      ["Yes", "No"].forEach((w, i) => { const b = document.createElement("button"); b.type = "button"; b.textContent = w; b.addEventListener("click", () => markAnswer(box, a, desk, w === "Yes", b)); fb.appendChild(b); if (!i) fb.append(" · "); });
      meta.appendChild(fb); box.appendChild(meta);
    }
    presence(mark, a.waiting ? "waiting" : "listening");
    const first = (a.sources || [])[0];
    if (first && first.href && new URL(first.href, location.href).pathname === location.pathname) setTimeout(() => point(first.href, `Cited [${first.n}]`), 250);
    return { box, desk };
  }

  // The conversation so far in this thread, oldest first, for the desk to read with the question.
  const words = el => { const c = el.cloneNode(true); c.querySelectorAll(".cite").forEach(x => x.remove()); return c.textContent; };
  const history = thread => [...thread.children]
    .filter(n => n.classList.contains("wa-v") || n.classList.contains("wa-a"))
    .map(n => n.classList.contains("wa-v")
      ? { who: "visitor", text: n.textContent }
      : { who: n.dataset.desk || "Desk", text: words(n.querySelector(".txt") || n) })
    .map(t => ({ who: t.who, text: t.text.replace(/\s+/g, " ").trim().slice(0, 600) }))
    .filter(t => t.text).slice(-8);

  // POST /ask and read the event stream.
  async function ask(thread, q, deskId, mark, onDesk) {
    const before = history(thread);
    const v = document.createElement("div"); v.className = "wa-v"; v.textContent = q; thread.appendChild(v);
    thread.setAttribute("aria-busy", "true"); presence(mark, "working");
    let progress = null, result = null;
    try {
      const r = await post("/ask", { q, desk: deskId || "", history: before });
      if (!r.ok || !r.body) throw new Error("ask failed");
      const reader = r.body.getReader(), dec = new TextDecoder(); let buf = "";
      for (;;) {
        const { value, done } = await reader.read(); if (done) break;
        buf += dec.decode(value, { stream: true });
        let i; while ((i = buf.indexOf("\n\n")) >= 0) {
          const chunk = buf.slice(0, i); buf = buf.slice(i + 2);
          const ev = (chunk.match(/^event: ?(.*)$/m) || [])[1], raw = chunk.split("\n").filter(l => l.startsWith("data:")).map(l => l.slice(5).trim()).join("\n");
          if (!ev || !raw) continue; let d; try { d = JSON.parse(raw); } catch { continue; }
          if (ev === "progress") {
            if (!progress) { progress = document.createElement("div"); progress.className = "wa-progress"; thread.appendChild(progress); }
            progress.textContent = ""; progress.appendChild(markSpan(deskOf(d.desk), 20)); const s = document.createElement("span"); s.textContent = d.label; progress.appendChild(s);
            if (window.WAMarks) window.WAMarks.draw(progress);
          } else if (ev === "answer") {
            if (progress) { progress.remove(); progress = null; }
            presence(mark, "answering"); d.q = q; result = renderAnswer(thread, d, mark);
            if (onDesk) onDesk(deskOf(d.desk));
          }
        }
      }
    } catch {
      if (progress) progress.remove();
      const e = document.createElement("div"); e.className = "wa-a"; e.textContent = "Answers are paused right now. The page and the forms on it still work."; thread.appendChild(e);
      presence(mark, "resting");
    }
    thread.removeAttribute("aria-busy"); save();
    return result;
  }

  // ---------- Peeks ----------
  let openPeek = null;
  const closePeek = p => { if (!p) return; p.classList.remove("open"); p.dataset.pinned = ""; const a = p.parentElement.querySelector("a"); a.setAttribute("aria-expanded", "false"); if (openPeek === p) openPeek = null; };
  const show = p => { if (openPeek && openPeek !== p) closePeek(openPeek); p.classList.add("open"); p.parentElement.querySelector("a").setAttribute("aria-expanded", "true"); openPeek = p; };
  $$(".wa-navitem").forEach(wrap => {
    const a = wrap.querySelector("a"), p = wrap.querySelector(".peek"); if (!p) return;
    const id = p.dataset.desk, mark = p.querySelector(".peek-head .wa-living");
    const thread = p.querySelector('[data-role="thread"]'), form = p.querySelector('[data-role="form"]'), input = form.querySelector("input");
    const pin = () => { p.dataset.pinned = "1"; p.querySelector('[data-role="pin"]').textContent = "Pinned · Esc to close"; };
    let tIn, tOut;
    wrap.addEventListener("mouseenter", () => { clearTimeout(tOut); tIn = setTimeout(() => show(p), 120); });
    wrap.addEventListener("mouseleave", () => { clearTimeout(tIn); if (!p.dataset.pinned) tOut = setTimeout(() => closePeek(p), 260); });
    a.addEventListener("focus", () => show(p));
    a.addEventListener("keydown", e => { if (e.key === "ArrowDown") { e.preventDefault(); show(p); const s = p.querySelector(".starter"); if (s) s.focus(); } });
    input.addEventListener("focus", () => { pin(); presence(mark, "listening"); });
    let lastDesk = DESKS[id];
    const run = async q => {
      pin(); p.querySelector(".peek-starters").style.display = "none"; p.querySelector(".peek-facts").style.display = "none";
      const res = await ask(thread, q, id, mark, d => { lastDesk = d; });
      if (res && !p.querySelector(".dock-it")) {
        const more = document.createElement("button"); more.type = "button"; more.className = "act dock-it"; more.textContent = "Keep talking in the side panel";
        more.addEventListener("click", () => { openRail(lastDesk, thread); closePeek(p); });
        thread.parentElement.appendChild(more);
      }
    };
    p.querySelectorAll(".starter").forEach(b => b.addEventListener("click", () => run(b.dataset.q)));
    p.querySelector(".peek-x").addEventListener("click", e => { e.stopPropagation(); closePeek(p); });
    form.addEventListener("submit", e => { e.preventDefault(); const q = input.value.trim(); if (!q) return; input.value = ""; run(q); });
    if (DESKS[id]) DESKS[id].open = () => { show(p); pin(); input.focus(); };
  });
  document.addEventListener("click", e => { if (openPeek && !openPeek.parentElement.contains(e.target)) closePeek(openPeek); });

  // ---------- Rail ----------
  const rail = $("#rail"), railThread = $("#railThread"), railBody = $("#railBody");
  let railDesk = FRONT;
  function setRailDesk(desk) {
    railDesk = desk; const holder = $("#railMark");
    if (holder) { holder.textContent = ""; holder.appendChild(markSpan(desk, 28)); if (window.WAMarks) window.WAMarks.draw(holder); }
    const n = $("#railName"); if (n) n.textContent = desk.name;
  }
  function openRail(desk, fromThread) {
    if (!rail) return;
    if (desk) setRailDesk(desk);
    if (fromThread) { railThread.textContent = ""; [...fromThread.children].forEach(n => railThread.appendChild(n.cloneNode(true))); }
    rail.classList.add("open"); railBody.scrollTop = railBody.scrollHeight; const q = $("#railQ"); if (q) q.focus(); save();
  }
  const railMark = () => $("#railMark .wa-living");
  const askRail = async q => { rail.classList.add("open"); await ask(railThread, q, railDesk.id === "front" ? "" : railDesk.id, railMark(), d => setRailDesk(d)); };
  if (rail) {
    $("#railClose").addEventListener("click", () => { rail.classList.remove("open"); save(); });
    $("#railForm").addEventListener("submit", e => { e.preventDefault(); const i = $("#railQ"); const q = i.value.trim(); if (!q) return; i.value = ""; askRail(q); });
  }
  const dock = $("#dockForm");
  if (dock) dock.addEventListener("submit", e => { e.preventDefault(); const i = $("#dockQ"); const q = i.value.trim(); if (!q) return; i.value = ""; setRailDesk(FRONT); askRail(q); });
  $$("[data-person]").forEach(a => a.addEventListener("click", e => {
    if (!rail) return; e.preventDefault(); openRail(railDesk);
    const box = document.createElement("div"); box.className = "wa-a";
    const t = document.createElement("div"); t.className = "txt"; t.textContent = `What would you like to ask ${owner}? A person reads this, not a WebAgent.`;
    box.appendChild(t); railThread.appendChild(box);
    const f = document.createElement("form"); f.className = "pass-form";
    f.innerHTML = '<textarea rows="3" placeholder="Your question"></textarea><input autocomplete="email" placeholder="Email or WhatsApp number (optional)"><button class="act primary" type="submit"></button>';
    f.querySelector("button").textContent = `Send to ${owner}`;
    f.addEventListener("submit", async ev => {
      ev.preventDefault(); const q = f.querySelector("textarea").value.trim(); if (!q) return;
      const r = await post("/ask/pass", { question: q, contact: f.querySelector("input").value, desk: "Person", page: location.pathname });
      const j = await r.json(); f.remove(); line(box, "wa-handover", j.words || "Sent.");
    });
    box.appendChild(f); f.querySelector("textarea").focus();
  }));

  // The thread follows the visitor across pages for this tab only.
  function save() { try { sessionStorage.setItem("wa-rail", JSON.stringify({ html: railThread ? railThread.innerHTML : "", desk: railDesk.id, open: rail && rail.classList.contains("open") })); } catch {} }
  try {
    const s = JSON.parse(sessionStorage.getItem("wa-rail") || "null");
    if (s && railThread && s.html) {
      railThread.innerHTML = s.html; // our own markup, saved from this page's renderer
      railThread.querySelectorAll(".acts, .pass-form, .fb-note, .wa-meta button").forEach(n => n.remove());
      setRailDesk(DESKS[s.desk] || FRONT);
      railThread.querySelectorAll(".wa-living[data-drawn]").forEach(n => n.removeAttribute("data-drawn"));
      if (window.WAMarks) window.WAMarks.draw(railThread);
    }
  } catch {}

  // ---------- Keys and the team sheet ----------
  const sheet = $("#teamSheet"), menuBtn = $("#menuBtn");
  const closeSheet = () => { if (!sheet) return; sheet.classList.remove("open"); if (menuBtn) menuBtn.setAttribute("aria-expanded", "false"); };
  if (sheet && menuBtn) {
    menuBtn.addEventListener("click", () => { const o = sheet.classList.toggle("open"); menuBtn.setAttribute("aria-expanded", String(o)); });
    const x = $("#sheetClose"); if (x) x.addEventListener("click", closeSheet);
    $$("[data-ask]", sheet).forEach(b => b.addEventListener("click", e => { e.stopPropagation(); closeSheet(); const d = DESKS[b.dataset.ask]; if (d && d.open) d.open(); else openRail(d); }));
  }
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") { if (openPeek) { const a = openPeek.parentElement.querySelector("a"); closePeek(openPeek); a.focus(); } closeSheet(); }
    if (e.key === "/" && !/input|textarea|select/i.test(document.activeElement.tagName)) { const q = $("#dockQ"); if (q) { e.preventDefault(); q.focus(); } }
  });
  // W8's visit.js shows and hides the footer's visit-count lines (GPC included); the samples
  // only confirm a change in words.
  document.addEventListener("agx-count-changed", e => {
    if (e.detail && e.detail.gpc) return;
    toast(e.detail && e.detail.counting ? "Visits on this device count again, by page only." : "Your visits on this device won't be counted.");
  });
  window.WADesks = { ask, openRail, toast, point };
})();
