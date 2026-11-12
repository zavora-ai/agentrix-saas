// The free check, in place on screen 1. The address box posts to W4's Assess; this script sends
// the same post, follows it to /assess/{id}, and shows that check's live events here: what it is
// reading, each finding as it is found, then the report and "Keep this report". Every state shown
// comes from an event. Without this script (or EventSource) the box posts as a normal form and
// Assess's own page shows the check.
(() => {
  const forms = document.querySelectorAll("form.paste[data-check]");
  const tpl = document.getElementById("check-tpl");
  const box = document.getElementById("check");
  if (!forms.length || !tpl || !box || !window.fetch || !window.EventSource || !window.URLSearchParams) return;

  // Assess's words for how much a finding matters and what it is about (design §4.3a).
  const SEVERITY = { urgent: "Act now", warn: "Worth fixing", info: "Good to know" };
  const CATEGORY = { security: "Security", speed: "Speed", links: "Links", "ai-readiness": "AI assistants" };
  const $ = (sel, root) => root.querySelector(sel);

  // Both boxes (screen 1 and screen 7) run the check on screen 1.
  forms.forEach(form => form.addEventListener("submit", async e => {
    e.preventDefault();
    const address = form.elements.url.value.trim();
    if (!address) return form.reportValidity();
    const button = $("button[type=submit]", form);
    button.disabled = true;
    let res;
    try {
      res = await fetch(form.action, { method: "POST", body: new URLSearchParams(new FormData(form)), redirect: "follow", credentials: "same-origin" });
    } catch {
      button.disabled = false;
      return form.submit();
    }
    const match = new URL(res.url).pathname.match(/^\/assess\/([A-Za-z0-9_-]+)$/);
    if (!res.ok || !match) {
      // Assess says what's wrong with the address on its own page; show its words here.
      button.disabled = false;
      const doc = new DOMParser().parseFromString(await res.text(), "text/html");
      const said = doc.querySelector("[role=alert], .error, .field-error, .form-error");
      if (!said) return form.submit();
      let note = $(".paste-error", form);
      if (!note) { note = document.createElement("p"); note.className = "paste-error"; note.setAttribute("role", "alert"); form.append(note); }
      note.textContent = said.textContent.trim();
      return;
    }
    start(match[1], address);
  }));

  function start(id, address) {
    const report = `/assess/${id}`;
    const live = tpl.content.firstElementChild.cloneNode(true);
    $("[data-site]", live).textContent = address.replace(/^https?:\/\//, "").replace(/\/$/, "");
    $("[data-report]", live).href = report;
    $("[data-assessment]", live).value = id;
    box.replaceChildren(live);
    document.querySelector(".hero").classList.add("checking");
    const film = document.querySelector(".hero-film video");
    if (film) film.pause();
    box.scrollIntoView({ block: "start" });

    const state = $("[data-state]", live), list = $("[data-findings]", live);
    const say = words => { if (words) state.textContent = words; };
    const es = new EventSource(`${report}/events`);
    const read = ev => { try { return JSON.parse(ev.data); } catch { return null; } };

    es.addEventListener("step", ev => { const d = read(ev); if (d) say(d.summary); });
    es.addEventListener("page", ev => {
      const d = read(ev); if (!d) return;
      say(d.status === "read" ? `Reading your pages (${d.of} found). Page ${d.n} opened in a sealed room.` : `Reading your pages (${d.of} found).`);
    });
    es.addEventListener("finding", ev => {
      const d = read(ev); if (!d) return;
      const li = document.createElement("li");
      li.className = `sev-${d.severity}`;
      const tag = document.createElement("span");
      tag.className = "check-tag";
      tag.textContent = [SEVERITY[d.severity], CATEGORY[d.category]].filter(Boolean).join(" · ");
      const title = document.createElement("b");
      title.textContent = d.title;
      const where = document.createElement("span");
      where.className = "check-where";
      where.textContent = d.where;
      li.append(tag, title, where);
      list.append(li);
    });
    es.addEventListener("error", ev => {
      if (!ev.data) return; // the connection dropped: EventSource reconnects and replays
      const d = read(ev); if (!d) return;
      say(d.message);
      if (!d.retrying) { const fail = $("[data-fail]", live); fail.textContent = d.message; fail.hidden = false; }
    });
    es.addEventListener("done", ev => {
      es.close();
      const d = read(ev) || {};
      if (d.href) $("[data-report]", live).href = d.href;
      const done = $("[data-done]", live);
      const words = $("[data-done-words]", live);
      if (d.outcome === "unreachable") {
        words.textContent = "We couldn't reach that site, so there's no report.";
        $("[data-keep]", live).remove();
        $("[data-talk]", live).remove();
      } else {
        const n = list.children.length;
        words.textContent = n ? `Your report is ready. We found ${n} ${n === 1 ? "thing" : "things"} worth knowing.` : "Your report is ready. We found nothing to fix.";
      }
      say("");
      state.hidden = true;
      done.hidden = false;
    });
    keep(live);
    talk(live, id);
  }

  // Screen 7's weekly fix sign-up, in place.
  const join = document.querySelector("form[data-join]");
  if (join) join.addEventListener("submit", async e => {
    e.preventDefault();
    const out = document.querySelector("[data-joined]"), button = join.querySelector("button");
    button.disabled = true;
    try {
      const res = await fetch(join.action, { method: "POST", body: new URLSearchParams(new FormData(join)), headers: { accept: "application/json" }, credentials: "same-origin" });
      const kept = await res.json();
      out.textContent = kept.message;
      out.hidden = false;
      if (res.ok) join.remove(); else button.disabled = false;
    } catch { button.disabled = false; join.submit(); }
  });

  // "Talk about your site": questions about the site just checked, answered from its own pages.
  function talk(live, id) {
    const box = $("[data-talk]", live), form = $("[data-talk-form]", live);
    if (!box || !form) return;
    const log = $(".answers", box), input = $("input[name=q]", form);
    const turns = [];
    const send = async q => {
      q = (q ?? input.value).trim();
      if (!q || !window.agentrixDesks) return;
      input.value = "";
      const said = await window.agentrixDesks.turn("/_home/talk", { assessment: id, q, history: turns.slice(-6) }, log, q, $("button[type=submit]", form));
      if (said) turns.push({ who: "visitor", text: q }, said);
    };
    form.addEventListener("submit", e => { e.preventDefault(); send(); });
    box.querySelectorAll(".starter").forEach(b => b.addEventListener("click", () => (b.dataset.focus !== undefined ? input.focus() : send(b.dataset.q))));
  }

  function keep(live) {
    const f = $("[data-keep]", live), out = $("[data-kept]", live);
    f.addEventListener("submit", async e => {
      e.preventDefault();
      const button = $("button[type=submit]", f);
      button.disabled = true;
      try {
        const res = await fetch(f.action, { method: "POST", body: new URLSearchParams(new FormData(f)), headers: { accept: "application/json" }, credentials: "same-origin" });
        const kept = await res.json();
        out.textContent = kept.message;
        out.hidden = false;
        if (res.ok) f.remove(); else button.disabled = false;
      } catch {
        button.disabled = false;
        f.submit();
      }
    });
  }
})();
