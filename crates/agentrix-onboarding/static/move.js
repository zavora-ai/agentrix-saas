/* Agentrix onboarding · the Move island (W4). The page is rendered from the Move's stored
   state; this follows it live over /moves/{id}/events (design §4.3a UiEvents). Every change
   on screen comes from an event; when the Move ends, the page reloads to its new state. */
(() => {
  const root = document.getElementById("move");
  const url = root && root.dataset.events;
  if (!url || !("EventSource" in window)) return;
  const $ = id => document.getElementById(id);
  const set = (id, v) => { const el = $(id); if (el && el.textContent !== String(v)) el.textContent = v; };
  const bytes = n => n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n >= 1e3 ? `${Math.floor(n / 1e3)} KB` : `${n} bytes`;
  const secs = ms => ms < 60000 ? `${Math.round(ms / 1000)} s` : `${Math.floor(ms / 60000)} m ${Math.round((ms % 60000) / 1000)} s`;
  const words = { pending: "Not started", running: "Now", waiting: "Your decision", done: "Done", failed: "Stopped" };
  const railClass = { done: "", running: "run", waiting: "wait", failed: "fail", pending: "todo" };
  const chipClass = { done: "chip-ok", failed: "chip-risk", running: "chip-info", waiting: "chip-wait", pending: "" };
  const short = r => { const h = String(r).split(":").pop(); return h.length > 12 ? `${h.slice(0, 4)}…${h.slice(-4)}` : h; };

  function step(s) {
    const stop = document.querySelector(`#rail [data-key="${s.step}"]`);
    if (stop) {
      stop.className = railClass[s.state] ?? "";
      const w = stop.querySelector(".w");
      if (w) w.textContent = s.state === "done" && s.elapsed_ms ? secs(s.elapsed_ms) : (words[s.state] || s.state);
    }
    const room = document.querySelector(`.room[data-key="${s.step}"]`);
    if (room) {
      room.dataset.state = s.state;
      const sum = room.querySelector("[data-summary]"); if (sum && s.summary) sum.textContent = s.summary;
      const chip = room.querySelector("[data-chip]");
      if (chip) { chip.className = `chip ${chipClass[s.state] || ""}`; chip.textContent = `${words[s.state] || s.state}${s.state !== "pending" && s.elapsed_ms ? ` · ${secs(s.elapsed_ms)}` : ""}`; }
      const a = room.querySelector("[data-receipt]");
      if (a && s.receipt) { a.hidden = false; a.href = `/receipts/${s.receipt}`; a.textContent = `receipt ${short(s.receipt)}`; }
    }
  }
  function tick(t) {
    set("tick-hosts", t.hosts); set("tick-requests", t.requests);
    set("tick-bytes", bytes(t.bytes)); set("tick-refusals", t.refusals); set("refused-count", t.refusals);
  }
  let loading = false, waiting = null;
  function frame(f) {
    const img = $("frame-img"); if (!img) return;
    if (loading) { waiting = f; return; }
    loading = true;
    const next = new Image();
    const done = ok => {
      loading = false;
      if (ok) {
        img.src = f.src; img.alt = `What the sealed browser sees on ${f.page_url}`; img.hidden = false;
        const empty = $("frame-empty"); if (empty) empty.hidden = true;
        const cap = $("frame-cap"); if (cap) cap.hidden = false;
        set("frame-url", f.page_url);
        const label = $("frame-label");
        if (label) { const live = f.kind === "live"; label.textContent = live ? "Live" : "Sealed screenshot, about 1 a second"; label.className = `chip ${live ? "chip-ok" : "chip-sealed"}`; }
      }
      if (waiting) { const w = waiting; waiting = null; frame(w); }
    };
    next.onload = () => done(true); next.onerror = () => done(false); next.src = f.src;
  }
  function parity(p) {
    if (p.page == null) {
      const pct = Math.round(p.score * 100);
      set("parity-score", `${pct}%`);
      const meter = document.querySelector(".parity-meter");
      if (meter) { meter.className = `meter parity-meter pct-${Math.floor(pct / 5) * 5}${pct >= 95 ? " ok" : ""}`; }
      return;
    }
    const list = $("parity-pages"); if (!list) return;
    let li = [...list.children].find(x => x.dataset.page === p.page);
    if (!li) { li = document.createElement("li"); li.dataset.page = p.page; li.innerHTML = '<div class="spread"><b class="mono"></b><span class="chip"></span></div><p class="small muted"></p>'; list.append(li); list.hidden = false; }
    li.querySelector("b").textContent = p.page;
    const chip = li.querySelector(".chip");
    chip.textContent = p.state === "pass" ? `${Math.round(p.score * 100)}%` : p.state === "retry" ? `Retrying (${p.attempt} of 3)` : "Flagged for you";
    chip.className = `chip ${p.state === "pass" ? "chip-ok" : p.state === "retry" ? "chip-wait" : "chip-risk"}`;
    li.querySelector("p").textContent = p.verdict;
    if (p.state === "flagged" && !li.querySelector("form")) {
      const form = document.createElement("form"); form.method = "post"; form.action = url.replace(/\/events$/, "/accept");
      const page = document.createElement("input"); page.type = "hidden"; page.name = "page"; page.value = p.page;
      const go = document.createElement("button"); go.type = "submit"; go.className = "btn btn-ghost btn-sm"; go.textContent = "Accept this page as it is";
      form.append(page, go); li.append(form);
    }
  }
  function quarantined(q) {
    const list = $("quarantine-live"); if (!list) return;
    const item = document.createElement("article"); item.className = "qitem";
    item.innerHTML = '<div class="qicon" aria-hidden="true"></div><div><div class="spread"><h3></h3><span class="chip chip-risk">Left behind</span></div><p class="plain"></p><p class="where"></p></div>';
    item.querySelector("h3").textContent = q.title; item.querySelector(".plain").textContent = q.reason; item.querySelector(".where").textContent = q.where;
    list.append(item);
    set("quarantine-count", list.children.length);
    const empty = $("quarantine-empty"); if (empty) empty.hidden = true;
  }
  function card(c) {
    const b = $("card-banner"); if (!b) return;
    b.hidden = false; const a = b.querySelector("a"); if (a) { a.href = c.href; a.textContent = c.title; }
  }
  function error(e) {
    const el = $("live-error"); if (!el) return;
    el.hidden = false; el.textContent = e.retrying ? `${e.message} Trying again.` : e.message;
  }
  const es = new EventSource(url);
  const on = (name, fn) => es.addEventListener(name, ev => { try { fn(JSON.parse(ev.data)); } catch (_) { /* a malformed event changes nothing */ } });
  on("step", step); on("receipt-tick", tick); on("frame", frame); on("parity", parity);
  on("quarantined", quarantined); on("card", card); on("error", error);
  on("page", p => set("move-progress", `${p.n} of ${p.of} pages`));
  on("snapshot", s => { (s.steps || []).forEach(step); if (s.tick) tick(s.tick); if (s.last_frame) frame(s.last_frame); if (s.parity) parity(s.parity); });
  on("done", () => { es.close(); location.reload(); });
  es.onerror = () => { if (es.readyState === EventSource.CLOSED) error({ message: "Live updates stopped. Reload the page to see the latest.", retrying: false }); };
})();
