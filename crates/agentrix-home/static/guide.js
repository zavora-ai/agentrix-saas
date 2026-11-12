// agentrix.website's own guide. The Agentrix mark in the header opens it: guided tours that move
// between the real pages and stop on real sections, and Ask, which uses the Your site desk.
// You control every stop; a tour carries on across page loads (sessionStorage) until you end it,
// and Esc ends it. Without this script the mark stays part of the home link, as before.
//
// Voice: when W11's voice serves agentrix.website's desks, a microphone button goes beside Ask's
// send button, with a line on when the microphone starts. Until then the guide is text only.
(() => {
  const logo = document.querySelector("header.top .logo");
  const mark = logo && logo.querySelector(".mark");
  if (!mark) return;

  // Each stop: the page, the part of it to show, and two short lines from that part's own words.
  // `near` is what "Look for this" points at, scrolled into view on a phone, above the guide.
  const TOURS = [
    {
      id: "meet", name: "Meet Agentrix", time: "About 2 minutes",
      line: "What Agentrix does to a website, how its WebAgents work, and what it costs.",
      stops: [
        { path: "/", sel: "#check", near: "#check form.paste", label: "Start", title: "What Agentrix gives you",
          say: "Agentrix upgrades your website into a team of WebAgents that talk with people and with AI agents.",
          look: "The box under the headline checks any site for free, from its public pages. You don't need an account." },
        { path: "/how-it-works", sel: "#conversations", label: "Conversations", title: "Three kinds of conversation",
          say: "Your customers, you and other AI agents each talk to your site, and one team of WebAgents handles all three.",
          look: "Each desk says it's AI, answers from your own pages and shows the page it used." },
        { path: "/how-it-works", sel: "#kernel", label: "Kernel", title: "Built on the ADK-Rust kernel",
          say: "Each WebAgent has one job, its own tools and a list of things it never does without you.",
          look: "The dials: only you turn one up, and money and promises always ask you first." },
        { path: "/prices", sel: "#pricing", near: "#pricing .plans", label: "Prices", title: "What it costs",
          say: "Moving your site is free. You pay monthly for the WebAgents that run it.",
          look: "Each plan card shows its price in US dollars and the WebAgents that come with it." },
      ],
    },
    {
      id: "sample", name: "See a WebAgent site", time: "About 1 minute",
      line: "A sample bakery run by WebAgents, and what to try there.",
      stops: [
        { path: "/samples", sel: ".sample.s-kilele", label: "Kilele", title: "Kilele Bakery",
          say: "Kilele Bakery is a sample we made up: a Nairobi bakery where each menu item is a WebAgent.",
          look: "The card says what its desks do, and whether the sample is open on this server now." },
        { path: "/samples", sel: ".sample.s-kilele .walk", label: "Try it", title: "What to try there",
          say: "Four things to try on the sample, in order. Orders there aren't real.",
          look: "First, ask if it's open on Sunday. The Visit desk answers from Kilele's own hours and shows the page.",
          open: { sel: ".sample.s-kilele .sample-go", words: "Open Kilele Bakery" } },
      ],
    },
    {
      id: "yours", name: "From your site to WebAgents", time: "About 2 minutes",
      line: "What changes when your site moves, how to start, and how you stay in charge.",
      stops: [
        { path: "/your-site", sel: "#changes", label: "Changes", title: "What changes",
          say: "Your site keeps its look, its pages and its address, and gains a team of WebAgents.",
          look: "Four cards: what changes for your customers, for you and for AI agents, and what stays." },
        { path: "/your-site", sel: '[aria-labelledby="cta-title"]', near: '[aria-labelledby="cta-title"] form.paste', label: "Free check", title: "Start with a free check",
          say: "Paste your site's address to start. The check is free.",
          look: "It reads public pages only, changes nothing on your site, and shows what the move would change." },
        { path: "/", sel: "#owner", near: "#owner .owner-ways", label: "Console", title: "You stay in charge",
          say: "After the move you run the site from the owner console, on your phone or computer.",
          look: "A brief each morning, decisions with one tap, and any conversation you want to step into." },
      ],
    },
  ];

  const KEY = "agentrix.guide";
  let state = {};
  try { state = JSON.parse(sessionStorage.getItem(KEY)) || {}; } catch { state = {}; }
  const save = () => { try { sessionStorage.setItem(KEY, JSON.stringify(state)); } catch { /* private window: the tour lasts one page */ } };
  const still = matchMedia("(prefers-reduced-motion: reduce)");
  const narrow = matchMedia("(max-width: 640px)");
  const clean = (p) => p.replace(/\/+$/, "") || "/";
  const onPage = (p) => clean(p) === clean(location.pathname);
  const tourOf = (id) => TOURS.find((t) => t.id === id);

  function el(tag, props = {}, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === "class") e.className = v;
      else if (k === "text") e.textContent = v;
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? "" : v);
    }
    e.append(...kids.filter((x) => x != null));
    return e;
  }

  // The mark becomes the guide's button, left of the wordmark; the wordmark stays the home link.
  const btn = el("button", { type: "button", class: "guide-btn", "aria-label": "Open the Agentrix guide", "aria-haspopup": "dialog", "aria-expanded": "false", "aria-controls": "guide" });
  const brand = el("div", { class: "brand" });
  logo.before(brand);
  btn.append(mark);
  brand.append(btn, logo);

  const status = el("span", { text: "Site guide" });
  const tabTours = el("button", { type: "button", role: "tab", id: "guide-tab-tours", class: "guide-tab", "aria-controls": "guide-tours" }, "Guided tours");
  const tabAsk = el("button", { type: "button", role: "tab", id: "guide-tab-ask", class: "guide-tab", "aria-controls": "guide-ask" }, "Ask");
  const tours = el("section", { role: "tabpanel", id: "guide-tours", class: "guide-pane", "aria-labelledby": "guide-tab-tours" });
  const ask = el("section", { role: "tabpanel", id: "guide-ask", class: "guide-pane", "aria-labelledby": "guide-tab-ask", hidden: true });
  const nav = el("div", { class: "guide-nav", hidden: true });
  const panel = el("div", { id: "guide", class: "guide", role: "dialog", "aria-modal": "true", "aria-labelledby": "guide-name", hidden: true },
    el("div", { class: "guide-head" },
      el("span", { class: "mark guide-mark", "aria-hidden": "true" }),
      el("div", { class: "guide-who" },
        el("p", { class: "guide-name", id: "guide-name", text: "Agentrix guide" }),
        el("p", { class: "guide-status" }, el("i", { "aria-hidden": "true" }), status)),
      el("button", { type: "button", class: "guide-x", "aria-label": "Close the guide", onclick: () => hide() }, "✕")),
    el("div", { class: "guide-tabs", role: "tablist", "aria-label": "Agentrix guide" }, tabTours, tabAsk),
    el("div", { class: "guide-body" }, tours, ask),
    nav,
    el("p", { class: "guide-foot", text: "Tours work without AI. Asking uses this site's WebAgents." }));
  document.body.append(panel);

  function tab(which, focus) {
    state.tab = which;
    save();
    for (const [t, p, w] of [[tabTours, tours, "tours"], [tabAsk, ask, "ask"]]) {
      const on = w === which;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      p.hidden = !on;
    }
    nav.hidden = which !== "tours" || !state.tour;
    if (which === "ask") loadAsk();
    if (focus) (which === "ask" ? tabAsk : tabTours).focus({ preventScroll: true });
  }
  tabTours.addEventListener("click", () => tab("tours"));
  tabAsk.addEventListener("click", () => tab("ask"));
  panel.querySelector(".guide-tabs").addEventListener("keydown", (e) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    tab(state.tab === "ask" ? "tours" : "ask", true);
  });

  // The tours list.
  function list() {
    status.textContent = "Site guide";
    panel.classList.remove("touring");
    nav.hidden = true;
    tours.replaceChildren(
      el("p", { class: "guide-eyebrow", text: "Choose a tour" }),
      el("h2", { class: "guide-h", tabindex: "-1", text: "Let me show you the website." }),
      el("p", { class: "guide-p", text: "A tour moves between the real pages. You control every stop, and I point out what matters when you arrive." }),
      el("ul", { class: "guide-tours" }, ...TOURS.map((t) => el("li", {},
        el("button", { type: "button", class: "guide-tour", onclick: () => start(t.id) },
          el("b", { text: t.name }),
          el("span", { class: "guide-meta", text: `${t.time} · ${t.stops.length} stops` }),
          el("span", { class: "guide-line", text: t.line }),
          el("i", { "aria-hidden": "true", text: "→" }))))));
  }

  // The part of the page a stop shows.
  let spotted = null;
  function spot(target, arriving, near) {
    if (spotted) spotted.classList.remove("guide-spot", "guide-spot-wide");
    spotted = target;
    if (!target) return;
    target.classList.add("guide-spot");
    // A part as wide as the page gets its ring inside its edges, where it can be seen.
    target.classList.toggle("guide-spot-wide", target.getBoundingClientRect().width >= document.documentElement.clientWidth - 2);
    // Straight there on a page that just loaded or when it's far down; a smooth move otherwise.
    const at = (narrow.matches && near && document.querySelector(near)) || target;
    const far = Math.abs(at.getBoundingClientRect().top) > innerHeight * 1.5;
    const go = () => at.scrollIntoView({ block: "start", behavior: arriving || far || still.matches ? "instant" : "smooth" });
    go();
    // Pictures above it may still be loading; place it again once they have.
    if (document.readyState !== "complete") addEventListener("load", () => { if (spotted === target) go(); }, { once: true });
  }

  function start(id) {
    state.tour = id;
    state.stop = 0;
    tab("tours");
    go(0);
  }

  // Moves to stop `n`, loading its page when it's on another one.
  function go(n) {
    const t = tourOf(state.tour);
    if (!t) return list();
    state.stop = Math.max(0, Math.min(n, t.stops.length - 1));
    state.open = true;
    save();
    const s = t.stops[state.stop];
    if (!onPage(s.path)) { location.assign(s.path); return; }
    stop(true);
  }

  function stop(focusHead, arriving) {
    const t = tourOf(state.tour);
    if (!t) return list();
    const i = Math.min(state.stop || 0, t.stops.length - 1);
    const s = t.stops[i];
    const last = i === t.stops.length - 1;
    const here = onPage(s.path);
    const target = here ? document.querySelector(s.sel) : null;
    spot(target, arriving, s.near);
    status.textContent = "Guiding";
    panel.classList.add("touring");
    const id = s.sel.startsWith("#") ? s.sel : "";
    const where = `${s.path}${id}`;
    const open = s.open && target ? document.querySelector(s.open.sel) : null;
    const head = el("h2", { class: "guide-h", tabindex: "-1", text: s.title });
    tours.replaceChildren(
      el("div", { class: "guide-stophead" },
        el("div", {}, el("p", { class: "guide-eyebrow", text: t.name }), head),
        el("button", { type: "button", class: "btn btn-sm guide-exit", onclick: () => end(true) }, "Exit tour")),
      el("ol", { class: "guide-bar", "aria-hidden": "true" }, ...t.stops.map((_, j) => el("li", { class: j < i ? "done" : j === i ? "now" : null }))),
      el("div", { class: "guide-card" },
        el("div", { class: "guide-top" },
          el("p", { class: "guide-n", text: `${String(i + 1).padStart(2, "0")} · ${s.label}` }),
          el("p", { class: "guide-do", text: s.say }),
          el("a", { class: "guide-anchor", href: `${s.path}${id}` }, el("span", { "aria-hidden": "true", text: "↳ " }), where)),
        here
          ? el("div", { class: "guide-look" },
              el("p", { class: "guide-look-h", text: "Look for this" }),
              el("p", { text: target ? s.look : "That part isn't on this page right now." }),
              open ? el("p", { class: "guide-go" }, el("a", { class: "btn btn-sm btn-primary", href: open.getAttribute("href") }, s.open.words)) : null)
          : el("div", { class: "guide-look" },
              el("p", { text: "This stop is on another page." }),
              el("p", { class: "guide-go" }, el("a", { class: "btn btn-sm btn-primary", href: s.path, onclick: () => { state.open = true; save(); } }, "Go to this stop")))));
    nav.replaceChildren(
      el("button", { type: "button", class: "btn btn-sm guide-back", disabled: i === 0, onclick: () => go(i - 1) }, "Back"),
      el("span", { class: "guide-count", "aria-live": "polite", text: `${i + 1} / ${t.stops.length}` }),
      el("button", { type: "button", class: "btn btn-sm btn-coral guide-next", onclick: () => (last ? end(true) : go(i + 1)) }, last ? "End tour" : "Next stop →"));
    nav.hidden = state.tab === "ask";
    panel.querySelector(".guide-body").scrollTop = 0;
    if (focusHead) head.focus({ preventScroll: true });
  }

  function end(focusList) {
    spot(null);
    state.tour = null;
    state.stop = 0;
    save();
    list();
    if (focusList) tours.querySelector(".guide-h").focus({ preventScroll: true });
  }

  // Ask: the Your site desk's peek, with the guide's own starters. Two of them start a tour.
  let asking = false;
  const said = [];
  async function loadAsk() {
    if (asking) return;
    asking = true;
    const box = el("div", { class: "guide-peek" });
    const starts = [
      ["Show me how I can transform my site", "yours"],
      ["Give me the two-minute tour", "meet"],
      ["How do AI agents use my site?"],
      ["What does it cost?"],
    ];
    ask.replaceChildren(
      el("h2", { class: "guide-h", tabindex: "-1", text: "What would you like to know?" }),
      el("p", { class: "guide-p", text: "Ask in your own words. The answer comes from this site's pages, with the page it used." }),
      el("div", { class: "guide-starters" }, ...starts.map(([q, tour]) =>
        el("button", { type: "button", class: "starter", onclick: () => (tour ? start(tour) : send(q)) }, q))),
      box);
    try {
      const res = await fetch("/_home/peek/your-site", { credentials: "same-origin" });
      if (!res.ok) throw new Error(String(res.status));
      box.innerHTML = await res.text(); // our own server-rendered fragment
    } catch {
      box.replaceChildren(el("p", { class: "guide-p", text: "Asking is paused right now. The tours still work." }));
      return;
    }
    const peek = box.querySelector(".peek");
    if (peek) { peek.removeAttribute("role"); peek.removeAttribute("aria-label"); }
    const form = box.querySelector("form.ask");
    if (form) form.addEventListener("submit", (e) => { if (window.agentrixDesks) { e.preventDefault(); send(); } });
  }
  async function send(q) {
    const form = ask.querySelector("form.ask");
    const log = ask.querySelector(".answers");
    if (!form || !log || !window.agentrixDesks) return;
    const input = form.querySelector("input[name=q]");
    q = (q ?? input.value).trim();
    if (!q) return;
    input.value = "";
    const turn = await window.agentrixDesks.turn("/_home/ask", { desk: "your-site", q, history: said.slice(-6) }, log, q, form.querySelector("button[type=submit]"));
    if (turn) said.push({ who: "visitor", text: q }, turn);
  }

  function show(focus, arriving) {
    panel.hidden = false;
    btn.setAttribute("aria-expanded", "true");
    state.open = true;
    save();
    tab(state.tab === "ask" ? "ask" : "tours");
    if (state.tour) stop(focus && state.tab !== "ask", arriving);
    else list();
    if (focus && (state.tab === "ask" || !state.tour)) (state.tab === "ask" ? tabAsk : tabTours).focus({ preventScroll: true });
  }
  function hide() {
    panel.hidden = true;
    btn.setAttribute("aria-expanded", "false");
    state.open = false;
    save();
    spot(null);
    btn.focus({ preventScroll: true });
  }
  btn.addEventListener("click", () => (panel.hidden ? show(true) : hide()));

  // Esc ends a tour and closes the guide; Tab stays inside it while it's open.
  panel.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (state.tour) end(false);
      hide();
      return;
    }
    if (e.key !== "Tab") return;
    const all = [...panel.querySelectorAll('a[href], button:not([disabled]), input:not([type="hidden"]), [tabindex="0"]')]
      .filter((x) => x.getClientRects().length);
    if (!all.length) return;
    const first = all[0], lastOne = all[all.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); lastOne.focus({ preventScroll: true }); }
    else if (!e.shiftKey && document.activeElement === lastOne) { e.preventDefault(); first.focus({ preventScroll: true }); }
  });

  // A tour that's under way carries on when its next page loads.
  if (state.tour && state.open) show(true, true);
})();
