/* Agentrix · live island for Assess and the Move.
   Consumes design §4.3a UiEvents over SSE. The page is rendered server-side first; this only updates it.
   Never invents state: every change on screen comes from an event. */
(() => {
  const me = document.currentScript;
  const url = me && me.dataset.events;
  if (!url || !("EventSource" in window)) return;
  const $ = id => document.getElementById(id);
  const words = n => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB` : `${(n / 1048576).toFixed(1)} MB`;
  const secs = ms => ms < 60000 ? `${Math.round(ms / 1000)} s` : `${Math.floor(ms / 60000)} m ${Math.round((ms % 60000) / 1000)} s`;
  const set = (id, v) => { const el = $(id); if (el && el.textContent !== String(v)) el.textContent = v; };

  function tick(t) {
    set("tick-hosts", t.hosts); set("tick-requests", t.requests);
    set("tick-bytes", words(t.bytes)); set("tick-refusals", t.refusals);
  }
  let loading = false, waiting = null;
  function frame(f) {
    const img = $("frame-img"), label = $("frame-label"), empty = $("frame-empty");
    if (!img) return;
    // Preload, then swap: setting img.src on every event cancels the load in flight, so on a slow
    // link the panel could stay blank. While one frame loads, only the newest waiting one is kept.
    if (loading) { waiting = f; return; }
    loading = true;
    const next = new Image();
    const done = ok => {
      loading = false;
      if (ok) {
        img.src = f.src; img.alt = `What the sealed browser sees on ${f.page_url}`; img.hidden = false;
        if (empty) empty.hidden = true;
      }
      if (waiting) { const w = waiting; waiting = null; frame(w); }
    };
    next.onload = () => done(true);
    next.onerror = () => done(false);
    next.src = f.src;
    if (label) {
      label.hidden = false;
      const live = f.kind === "live";
      label.textContent = live ? "Live" : "Sealed screenshot, about 1 a second";
      label.className = `label chip ${live ? "ok" : "sealed"}`;
    }
  }
  function page(p) {
    set("assess-progress", `${p.n} of ${p.of} pages read`);
    set("move-progress", `${p.n} of ${p.of} pages`);
  }
  function finding(f) {
    const list = $("live-findings"); if (!list) return;
    list.hidden = false;
    const li = document.createElement("li");
    li.className = "finding";
    const sev = { urgent: ["risk", "Urgent"], warn: ["wait", "Worth fixing"], info: ["", "Good to know"] }[f.severity] || ["", ""];
    li.innerHTML = `<span class="chip ${sev[0]}"></span><b></b><p class="small muted"></p>`;
    li.children[0].textContent = sev[1]; li.children[1].textContent = f.title; li.children[2].textContent = f.where;
    list.append(li);
  }
  function step(s) {
    const row = document.querySelector(`[data-step="${s.step}"]`); if (!row) return;
    row.dataset.state = s.state;
    const what = row.querySelector(".what"); if (what && s.summary) what.textContent = s.summary;
    const took = row.querySelector(".took"); if (took) took.textContent = s.state === "pending" ? "" : secs(s.elapsed_ms);
    const st = row.querySelector(".sr-state"); if (st) st.textContent = { pending: "Not started", running: "In progress", waiting: "Waiting for you", done: "Done", failed: "Stopped" }[s.state] || s.state;
    if (s.receipt) { let a = row.querySelector(".rcpt"); if (!a) { a = document.createElement("a"); a.className = "rcpt small"; a.textContent = "receipt"; (what || row).after(a); } a.href = `/receipts/${s.receipt}`; }
  }
  function parity(p) {
    if (p.page == null) {
      set("parity-score", `${Math.round(p.score * 100)}%`);
      const bar = $("parity-bar"); if (bar) bar.style.width = `${Math.round(p.score * 100)}%`;
      return;
    }
    const list = $("parity-pages"); if (!list) return;
    let li = list.querySelector(`[data-page="${CSS.escape(p.page)}"]`);
    if (!li) { li = document.createElement("li"); li.dataset.page = p.page; li.innerHTML = `<div class="spread"><b class="mono"></b><span class="chip"></span></div><p class="small muted"></p>`; list.append(li); list.hidden = false; }
    li.querySelector("b").textContent = p.page;
    const chip = li.querySelector(".chip");
    chip.textContent = p.state === "pass" ? `${Math.round(p.score * 100)}%` : p.state === "retry" ? `Retrying (${p.attempt} of 3)` : "Flagged for you";
    chip.className = `chip ${p.state === "pass" ? "ok" : p.state === "retry" ? "wait" : "risk"}`;
    li.querySelector("p").textContent = p.verdict;
    // R6.4: a flagged page can be accepted as it is (the Move's own accept route; not on Assess).
    if (p.state === "flagged" && /^\/moves\/[^/]+\/events$/.test(url) && !li.querySelector("form")) {
      const form = document.createElement("form");
      form.method = "post"; form.action = url.replace(/\/events$/, "/accept");
      const page = document.createElement("input");
      page.type = "hidden"; page.name = "page"; page.value = p.page;
      const go = document.createElement("button");
      go.type = "submit"; go.className = "btn btn-ghost"; go.textContent = "Accept this page as it is";
      form.append(page, go); li.append(form);
    }
  }
  function quarantined(q) {
    const list = $("quarantine-live"); if (!list) return;
    list.hidden = false;
    const li = document.createElement("li");
    li.innerHTML = `<b></b><p class="small muted"></p>`;
    li.children[0].textContent = q.title; li.children[1].textContent = `${q.reason} · ${q.where}`;
    list.append(li);
    const n = $("quarantine-count"); if (n) n.textContent = list.children.length;
  }
  function card(c) {
    const b = $("card-banner"); if (!b) return;
    b.hidden = false;
    const a = b.querySelector("a"); if (a) { a.href = c.href; a.textContent = c.title; }
  }
  function error(e) {
    const el = $("live-error"); if (!el) return;
    el.hidden = false; el.textContent = e.retrying ? `${e.message} Trying again.` : e.message;
  }

  const es = new EventSource(url);
  const on = (name, fn) => es.addEventListener(name, ev => { try { fn(JSON.parse(ev.data)); } catch (_) { /* a malformed event changes nothing */ } });
  on("receipt-tick", tick);
  on("frame", frame);
  on("page", page);
  on("finding", finding);
  on("step", step);
  on("parity", parity);
  on("quarantined", quarantined);
  on("card", card);
  on("error", error);
  on("snapshot", s => { (s.steps || []).forEach(step); if (s.tick) tick(s.tick); if (s.last_frame) frame(s.last_frame); if (s.parity) parity(s.parity); });
  on("done", d => { es.close(); if (d.href) location.assign(d.href); else location.reload(); });
  es.onerror = () => { const el = $("live-error"); if (el && es.readyState === EventSource.CLOSED) { el.hidden = false; el.textContent = "Live updates stopped. Reload the page to see the latest."; } };
})();
