/* Agentrix WebAgent Platform: website experience concepts, shared helpers.
   Everything here is scripted for the concept: no model is called. */
(() => {
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const wait = ms => new Promise(r => setTimeout(r, reduced ? Math.min(ms, 60) : ms));
  const esc = s => String(s).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

  const I = {
    send: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14M13 6l6 6-6 6"/></svg>',
    close: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>',
    mic: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/></svg>',
    handover: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h12l-3-3M20 17H8l3 3"/></svg>',
    share: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12M7 8l5-5 5 5"/><path d="M5 13v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6"/></svg>',
    doc: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h4"/></svg>',
    lock: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>',
    play: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/></svg>',
    pause: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="6" y="5" width="4" height="14" rx="1"/><rect x="14" y="5" width="4" height="14" rx="1"/></svg>',
    prev: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 6l-6 6 6 6"/></svg>',
    next: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6"/></svg>',
  };

  // Presence: truthful states only (see docs/06 §4.3)
  const presence = (mark, state) => { if (mark) mark.dataset.state = state || "idle"; };

  // The site points: highlight the cited place on the real page
  let lastPointed = null, pointTimer = null;
  const point = (target, label) => {
    const el = typeof target === "string" ? document.querySelector(target) : target;
    if (!el) return;
    if (lastPointed) { lastPointed.classList.remove("pointed"); lastPointed.removeAttribute("data-pointer"); }
    el.classList.add("pointed");
    if (label) el.setAttribute("data-pointer", label);
    const box = el.closest(".frame-body");
    if (box) {
      const top = el.getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop - box.clientHeight / 2 + el.offsetHeight / 2;
      box.scrollTo({ top: Math.max(0, top), behavior: reduced ? "auto" : "smooth" });
    } else el.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "center" });
    lastPointed = el;
    clearTimeout(pointTimer);
    pointTimer = setTimeout(() => { el.classList.remove("pointed"); el.removeAttribute("data-pointer"); }, 5200);
  };

  const toast = msg => {
    let t = document.querySelector(".toast");
    if (!t) { t = document.createElement("div"); t.className = "toast"; t.setAttribute("role", "status"); document.body.appendChild(t); }
    t.textContent = msg; t.classList.add("show");
    clearTimeout(t._h); t._h = setTimeout(() => t.classList.remove("show"), 2600);
  };

  const visitor = (thread, text) => {
    const v = document.createElement("div"); v.className = "wa-v"; v.textContent = text;
    thread.appendChild(v); return v;
  };

  /* Scripted answer, streamed phrase by phrase (never a fake typing indicator).
     opts: { thread, desk:{name, icon, color}, mark, steps:[label], phrases:[html],
             sources:[{n,label,target,pointer,lock}], actions:[{label,primary,href,on}],
             handover:{from,to,text}, meta, scroller } */
  const answer = async opts => {
    const { thread, desk, mark } = opts;
    // Bring new content into view once; never follow the stream (NN/g: no autoscroll while streaming)
    const scroll = el => { const s = opts.scroller; if (!s || !el) return; const r = el.getBoundingClientRect(), sr = s.getBoundingClientRect(); if (r.top > sr.bottom - 48 || r.top < sr.top) s.scrollTop += r.top - sr.top - 12; };
    if (!thread.hasAttribute("role")) { thread.setAttribute("role", "log"); thread.setAttribute("aria-label", "Conversation"); }
    if (thread.lastElementChild) scroll(thread.lastElementChild);
    presence(mark, "working");
    for (const step of (opts.steps || [])) {
      const p = document.createElement("div"); p.className = "wa-progress";
      p.innerHTML = `<span class="wa-mark sm" data-state="working" style="--c:${desk.color}">${desk.icon}</span><span>${esc(step)}</span>`;
      thread.appendChild(p); scroll(p);
      await wait(520);
      p.remove();
    }
    if (opts.handover) {
      const h = document.createElement("div"); h.className = "wa-handover";
      h.innerHTML = `${I.handover}<span>${opts.handover}</span>`;
      thread.appendChild(h); scroll(h); await wait(380);
    }
    presence(mark, "answering");
    const a = document.createElement("div"); a.className = "wa-a";
    a.innerHTML = `<div class="by"><span class="wa-mark sm" style="--c:${desk.color}">${desk.icon}</span>${esc(desk.name)} <span class="wa-ai">AI</span></div><div class="txt"></div>`;
    thread.appendChild(a); scroll(a);
    const txt = a.querySelector(".txt");
    for (const ph of opts.phrases) {
      const s = document.createElement("span"); s.className = "phrase"; s.innerHTML = ph + " ";
      txt.appendChild(s);
      await wait(170);
    }
    if (opts.sources && opts.sources.length) {
      const srcs = document.createElement("div"); srcs.className = "srcs";
      opts.sources.forEach(s => {
        const b = document.createElement("button"); b.type = "button"; b.className = "src" + (s.lock ? " lock" : "");
        b.innerHTML = `<b>${s.n}</b>${esc(s.label)}`;
        b.addEventListener("click", () => s.target && point(s.target, s.pointer || `Cited [${s.n}]`));
        srcs.appendChild(b);
      });
      a.appendChild(srcs);
      txt.querySelectorAll(".cite").forEach(c => c.addEventListener("click", e => {
        e.preventDefault(); const s = opts.sources.find(x => String(x.n) === c.dataset.n);
        if (s && s.target) point(s.target, s.pointer || `Cited [${s.n}]`);
      }));
    }
    if (opts.actions && opts.actions.length) {
      const acts = document.createElement("div"); acts.className = "acts";
      opts.actions.forEach(x => {
        const el = document.createElement(x.href ? "a" : "button");
        el.className = "act" + (x.primary ? " primary" : "");
        if (x.href) el.href = x.href; else el.type = "button";
        el.innerHTML = x.label;
        if (x.on) el.addEventListener("click", x.on);
        acts.appendChild(el);
      });
      a.appendChild(acts);
    }
    if (opts.meta === false) { presence(mark, "listening"); return a; }
    const meta = document.createElement("div"); meta.className = "wa-meta";
    meta.innerHTML = `<span>${opts.meta || "Answered from this site's pages"}</span><span>Was this right? <button type="button">Yes</button> · <button type="button">No</button></span>`;
    meta.querySelectorAll("button").forEach(b => b.addEventListener("click", () => toast(b.textContent === "Yes" ? "Thanks. Counted as a good answer for the owner." : "Thanks. Wanjiru will see this so the page can be fixed.")));
    a.appendChild(meta);
    presence(mark, "listening");
    if (opts.firstPoint) { const s = opts.sources.find(x => x.n === opts.firstPoint); if (s) setTimeout(() => point(s.target, s.pointer || `Cited [${s.n}]`), 250); }
    return a;
  };

  window.XP = { reduced, wait, esc, I, presence, point, toast, visitor, answer };
})();
