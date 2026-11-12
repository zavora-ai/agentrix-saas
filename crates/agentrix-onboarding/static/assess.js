/* Agentrix onboarding · the Assess island (W4).
   The page is rendered server-side first; this only adds what the server can't draw at once.
   It replays the run's recorded events from the first one (Last-Event-ID: 0), so a reload still
   shows every page read, then follows the run live. Every change on screen comes from an event
   (design §4.3a UiEvents); nothing is invented. */
(() => {
  const root = document.getElementById("assess");
  if (!root) return;
  const state = root.dataset.state;
  const url = root.dataset.events;
  const host = (root.dataset.host || "").toLowerCase();
  const running = state === "running";
  const $ = id => document.getElementById(id);
  const set = (id, v) => { const el = $(id); if (el && el.textContent !== String(v)) el.textContent = v; };
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  // Same words as Assess's report (detect.rs bytes_words), so the live and final totals agree.
  const bytes = n => n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n >= 1e3 ? `${Math.floor(n / 1e3)} KB` : `${n} bytes`;
  const LOCK = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>';

  // ---------- Share report ----------
  const share = $("share");
  if (share) {
    const full = new URL(share.dataset.path, location.origin).href;
    set("share-url", full.replace(/^https?:\/\//, ""));
    share.addEventListener("click", async () => {
      try {
        if (navigator.share) { await navigator.share({ title: document.title, url: full }); return; }
        await navigator.clipboard.writeText(full);
        set("share-note", "Link copied. Anyone with the link can read this report.");
        share.textContent = "Copied";
      } catch (_) { /* the person closed the share sheet; nothing to say */ }
    });
  }
  if (!url) return;

  // ---------- Pages ----------
  const list = $("pagelist");
  const seen = new Set();
  let refusedPages = 0;
  function where(u) {
    try { const p = new URL(u, location.origin); return p.host.toLowerCase() === host ? (p.pathname + p.search) : p.href; }
    catch (_) { return u; }
  }
  function page(p) {
    if (running) set("assess-progress", `${p.n} of ${p.of} pages read`);
    if (!list || seen.has(p.n)) return;
    seen.add(p.n);
    const li = document.createElement("li");
    li.innerHTML = '<div><div class="path"></div><div class="ttl"></div></div><span></span>';
    li.querySelector(".path").textContent = where(p.url);
    const badge = li.lastElementChild;
    if (p.status === "read") {
      li.querySelector(".ttl").textContent = "Read";
      badge.className = "sealed";
      badge.innerHTML = `<span class="seal">${LOCK}</span>network: this site only <span class="sep">·</span> keys: none`;
    } else {
      const refused = p.status === "refused";
      li.querySelector(".ttl").textContent = `${refused ? "Refused" : "Didn't load"}${p.reason ? `: ${p.reason}` : ""}`;
      badge.className = `chip ${refused ? "chip-risk" : "chip-wait"}`;
      badge.textContent = refused ? "Refused" : "Didn't load";
      if (refused) refusedPages += 1;
    }
    const empty = $("pagelist-empty"); if (empty) empty.hidden = true;
    if (running) {
      // As the concept draws it: the newest page on top, the last few in view.
      list.prepend(li);
      while (list.children.length > 6) list.lastElementChild.remove();
    } else {
      list.append(li);
    }
  }

  // ---------- The room's receipt ----------
  function tick(t) {
    set("tick-hosts", t.hosts); set("tick-requests", t.requests);
    set("tick-bytes", bytes(t.bytes)); set("tick-refusals", t.refusals);
    const box = $("refused");
    if (box && t.refusals > 0) {
      box.hidden = false;
      set("refused-title", `Refused: ${plural(t.refusals, "request", "requests")}`);
      set("refused-words", `Something on the site tried to send our reader beyond ${host}. The room blocked it and wrote it into its receipt.`);
    }
  }

  // ---------- Frames ----------
  let loading = false, waiting = null;
  function frame(f) {
    const img = $("frame-img"), label = $("frame-label");
    if (!img) return;
    // Preload, then swap: changing img.src on every event cancels the load in flight, so on a
    // slow link the panel could stay blank. While one loads, only the newest waiting one is kept.
    if (loading) { waiting = f; return; }
    loading = true;
    const next = new Image();
    const done = ok => {
      loading = false;
      if (ok) {
        img.src = f.src; img.alt = `What the sealed browser sees on ${f.page_url}`; img.hidden = false;
        const empty = $("frame-empty"); if (empty) empty.hidden = true;
        set("frame-url", where(f.page_url));
        const cap = $("frame-cap"); if (cap) cap.hidden = false;
        if (label) {
          const live = f.kind === "live";
          label.textContent = live ? "Live" : "Sealed screenshot, about 1 a second";
          label.className = `chip ${live ? "chip-ok" : "chip-sealed"}`;
        }
      }
      if (waiting) { const w = waiting; waiting = null; frame(w); }
    };
    next.onload = () => done(true);
    next.onerror = () => done(false);
    next.src = f.src;
  }

  // ---------- Findings as they're raised ----------
  let found = 0;
  function finding(f) {
    const box = $("found"), ul = $("found-list");
    if (!box || !ul) return;
    found += 1;
    box.hidden = false;
    set("found-title", `Found so far: ${found}`);
    const li = document.createElement("li");
    const sev = { urgent: ["chip-risk", "Urgent"], warn: ["chip-wait", "Worth fixing"], info: ["", "Good to know"] }[f.severity] || ["", ""];
    li.innerHTML = '<span class="chip"></span> <span class="t"></span>';
    li.firstElementChild.className = `chip ${sev[0]}`;
    li.firstElementChild.textContent = sev[1];
    li.lastElementChild.textContent = f.title;
    ul.append(li);
  }

  function error(e) {
    const el = $("live-error"); if (!el) return;
    el.hidden = false; el.textContent = e.retrying ? `${e.message} Trying again.` : e.message;
  }

  let finished = false;
  function done(d) {
    finished = true;
    // A page that rendered the finished report replays the whole run, end included: stay put.
    if (running) { if (d.href) location.assign(d.href); else location.reload(); }
  }

  const handlers = {
    "page": page, "receipt-tick": tick, "frame": frame, "finding": finding, "error": error, "done": done,
    "snapshot": s => { if (s.tick) tick(s.tick); if (s.last_frame) frame(s.last_frame); },
  };
  function dispatch(name, data) {
    const fn = handlers[name]; if (!fn) return;
    try { fn(JSON.parse(data)); } catch (_) { /* a malformed event changes nothing */ }
  }

  // ---------- The stream: fetch, so the first request can ask for everything from 0 ----------
  let last = 0, tries = 0;
  async function follow() {
    const res = await fetch(url, { headers: { "Accept": "text/event-stream", "Last-Event-ID": String(last) }, cache: "no-store" });
    if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
    tries = 0;
    const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
    let buf = "";
    for (;;) {
      const { value, done: end } = await reader.read();
      if (end) break;
      buf += value.replace(/\r\n?/g, "\n");
      let cut;
      while ((cut = buf.indexOf("\n\n")) >= 0) {
        const block = buf.slice(0, cut); buf = buf.slice(cut + 2);
        let name = "message", data = "", id = null;
        for (const line of block.split("\n")) {
          if (line.startsWith(":")) continue;
          const i = line.indexOf(":"); const field = i < 0 ? line : line.slice(0, i);
          const val = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
          if (field === "event") name = val; else if (field === "data") data += (data ? "\n" : "") + val; else if (field === "id") id = val;
        }
        if (id !== null && /^\d+$/.test(id)) last = Math.max(last, Number(id));
        if (data) dispatch(name, data);
      }
    }
  }
  async function run() {
    while (!finished) {
      try { await follow(); } catch (_) { /* dropped: try again below */ }
      if (finished || !running) return;
      tries += 1;
      if (tries > 5) { error({ message: "Live updates stopped. Reload the page to see the latest.", retrying: false }); return; }
      await new Promise(r => setTimeout(r, 1000 * tries));
    }
  }
  if (window.fetch && window.TextDecoderStream && window.ReadableStream) { run(); return; }

  // Older browsers: EventSource follows from now on, without the replay.
  if (!("EventSource" in window) || !running) return;
  const es = new EventSource(url);
  Object.keys(handlers).forEach(name => es.addEventListener(name, ev => { if (name === "done") es.close(); dispatch(name, ev.data); }));
})();
