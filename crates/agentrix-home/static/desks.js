// agentrix.website's desks: the peek beside each desk in the header, and every ask form (in a
// peek or on a desk's page) answering in place. Without this script the desks are links to their
// pages and the forms open an answer page, so nothing depends on it.
(() => {
  const history = new Map(); // desk id -> [{who, text}]
  // An answer's words for the conversation sent back, without its citation links.
  const plain = html => { const d = document.createElement("div"); d.innerHTML = html; d.querySelectorAll("a.cite").forEach(c => c.remove()); return d.textContent.replace(/\s+/g, " ").trim(); };

  // One answer into a log: the question, the answer's HTML (built and escaped on the server),
  // its sources, where it came from and one next step.
  function render(log, q, a) {
    const box = document.createElement("div");
    box.className = "turn-a";
    const who = document.createElement("p");
    who.className = "turn-who";
    who.textContent = a.name + (a.ai ? " · AI" : "");
    box.append(who);
    const body = document.createElement("div");
    body.innerHTML = a.html;
    box.append(body);
    if (a.sources.length) {
      const ol = document.createElement("ol");
      ol.className = "answer-sources";
      for (const s of a.sources) { const li = document.createElement("li"); const l = document.createElement("a"); l.href = s.href; l.textContent = s.label; li.append(l); ol.append(li); }
      box.append(ol);
    }
    const foot = document.createElement("p");
    foot.className = "answer-foot";
    foot.textContent = a.footer;
    box.append(foot);
    if (a.step) { const p = document.createElement("p"); const l = document.createElement("a"); l.className = "btn btn-sm"; l.href = a.step.href; l.textContent = a.step.label; p.append(l); box.append(p); }
    log.append(box);
    return { who: a.name, text: plain(a.html) };
  }

  // Asks `url` with `body` and shows the turn in `log`; returns the conversation so far.
  async function turn(url, body, log, q, button) {
    const mine = document.createElement("p");
    mine.className = "turn-q";
    mine.textContent = q;
    log.append(mine);
    const wait = document.createElement("p");
    wait.className = "turn-wait";
    wait.textContent = "Reading…";
    log.append(wait);
    if (button) button.disabled = true;
    try {
      const res = await fetch(url, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify(body),
        credentials: "same-origin",
      });
      const a = await res.json().catch(() => null);
      if (!res.ok || !a || !a.html) { wait.textContent = (a && a.message) || "Answers are paused right now. The page and the form above still work."; return null; }
      wait.remove();
      return render(log, q, a);
    } catch {
      wait.textContent = "Answers are paused right now. The page and the form above still work.";
      return null;
    } finally {
      if (button) button.disabled = false;
    }
  }
  window.agentrixDesks = { turn };

  async function ask(form, q) {
    const desk = form.dataset.ask;
    const log = form.parentElement.querySelector(".answers");
    const input = form.querySelector("input[name=q]");
    q = (q ?? input.value).trim();
    if (!q || !log) return;
    input.value = "";
    const turns = history.get(desk) || [];
    const said = await turn("/_home/ask", { desk, q, history: turns.slice(-6) }, log, q, form.querySelector("button[type=submit]"));
    if (said) { turns.push({ who: "visitor", text: q }, said); history.set(desk, turns); }
  }

  function wire(root) {
    root.querySelectorAll("form.ask[data-ask]").forEach(form => {
      if (form.dataset.wired) return;
      form.dataset.wired = "1";
      form.addEventListener("submit", e => { e.preventDefault(); ask(form); });
      form.parentElement.querySelectorAll(".starter[data-q]").forEach(s => s.addEventListener("click", e => { e.preventDefault(); ask(form, s.dataset.q); }));
    });
  }
  wire(document);

  // The header's peeks, on wide screens with a pointer or a keyboard.
  const wide = matchMedia("(min-width: 960px)");
  const peeks = new Map();
  let open = null, closeTimer = 0;
  const close = (focusLink) => {
    if (!open) return;
    open.panel.hidden = true;
    open.link.setAttribute("aria-expanded", "false");
    if (focusLink) open.link.focus();
    open = null;
  };
  async function show(link) {
    if (!wide.matches) return;
    clearTimeout(closeTimer);
    if (open && open.link === link) return;
    close(false);
    let panel = peeks.get(link);
    if (!panel) {
      panel = document.createElement("div");
      panel.className = "peek-wrap";
      panel.hidden = true;
      link.after(panel);
      peeks.set(link, panel);
      try {
        const res = await fetch(`/_home/peek/${link.dataset.desk}`, { credentials: "same-origin" });
        if (!res.ok) throw new Error(String(res.status));
        panel.innerHTML = await res.text(); // our own server-rendered fragment
        wire(panel);
        panel.addEventListener("mouseenter", () => clearTimeout(closeTimer));
        panel.addEventListener("mouseleave", () => { closeTimer = setTimeout(() => close(false), 250); });
        panel.addEventListener("keydown", e => { if (e.key === "Escape") { e.preventDefault(); close(true); } });
      } catch {
        panel.remove();
        peeks.delete(link);
        return;
      }
    }
    panel.style.left = `${link.offsetLeft}px`;
    panel.hidden = false;
    link.setAttribute("aria-expanded", "true");
    open = { link, panel };
  }
  document.querySelectorAll(".desks .desk[data-desk]").forEach(link => {
    link.setAttribute("aria-expanded", "false");
    link.setAttribute("aria-haspopup", "dialog");
    link.addEventListener("mouseenter", () => { closeTimer = setTimeout(() => show(link), 120); });
    link.addEventListener("mouseleave", () => { clearTimeout(closeTimer); closeTimer = setTimeout(() => close(false), 250); });
    link.addEventListener("focus", () => show(link));
    link.addEventListener("keydown", e => {
      if (e.key === "ArrowDown" && open && open.link === link) { e.preventDefault(); open.panel.querySelector("button, input, a")?.focus(); }
      if (e.key === "Escape") { e.preventDefault(); close(true); }
    });
  });
  document.addEventListener("click", e => { if (open && !open.panel.contains(e.target) && e.target !== open.link) close(false); });

  // Screen 2's team: each WebAgent opens a panel under its card on hover, keyboard focus or a
  // tap. The four that talk load the same peek as the header (ask box included); the others show
  // what they're doing and the one thing a visitor can do about it.
  const cards = [...document.querySelectorAll(".team-card")].filter(c => c.querySelector(".team-pop"));
  if (cards.length) {
    document.querySelectorAll(".team-cards").forEach(ul => ul.classList.add("team-js"));
    let card = null, timer = 0;
    const shut = () => {
      if (!card) return;
      card.classList.remove("is-open");
      card.querySelector(".team-pop").hidden = true;
      card.setAttribute("aria-expanded", "false");
      card = null;
    };
    async function openCard(c) {
      clearTimeout(timer);
      if (card === c) return;
      shut();
      const pop = c.querySelector(".team-pop");
      if (c.dataset.desk && !pop.dataset.loaded) {
        pop.dataset.loaded = "1";
        try {
          const res = await fetch(`/_home/peek/${c.dataset.desk}`, { credentials: "same-origin" });
          if (!res.ok) throw new Error(String(res.status));
          pop.innerHTML = await res.text(); // our own server-rendered fragment
          // The header's peek uses the same ids; keep these apart.
          pop.querySelectorAll("[id]").forEach(el => {
            const id = el.id;
            el.id = `team-${id}`;
            pop.querySelectorAll(`label[for="${id}"]`).forEach(l => { l.htmlFor = el.id; });
          });
          wire(pop);
        } catch {
          delete pop.dataset.loaded;
          return;
        }
      }
      pop.hidden = false;
      c.classList.add("is-open");
      c.setAttribute("aria-expanded", "true");
      card = c;
    }
    cards.forEach(c => {
      c.tabIndex = 0;
      c.setAttribute("aria-expanded", "false");
      c.addEventListener("mouseenter", () => { clearTimeout(timer); timer = setTimeout(() => openCard(c), 120); });
      c.addEventListener("mouseleave", () => {
        clearTimeout(timer);
        // Once someone has asked something, it stays open until they click away or press Escape.
        timer = setTimeout(() => {
          if (card === c && !c.contains(document.activeElement) && !c.querySelector(".answers > *")) shut();
        }, 300);
      });
      c.addEventListener("focusin", () => openCard(c));
      c.addEventListener("click", e => { if (!e.target.closest(".team-pop")) openCard(c); });
      c.addEventListener("keydown", e => { if (e.key === "Escape" && card === c) { e.preventDefault(); shut(); c.focus(); } });
    });
    document.addEventListener("click", e => { if (card && !card.contains(e.target)) shut(); });
  }
})();
