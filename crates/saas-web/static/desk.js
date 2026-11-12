// saas-web · the money host's desks (Sales, Support). Posts the question to /desk/{id} and
// shows the answer with its sources. No inline script: loaded as a module.

const desks = document.querySelector("[data-desks]");

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function link(href, text) {
  const a = el("a", "", text);
  a.href = href;
  return a;
}

async function ask(panel, q) {
  const thread = panel.querySelector("[data-thread]");
  thread.append(el("li", "me", q));
  const wait = el("li", "wait", "Thinking…");
  thread.append(wait);
  thread.scrollTop = thread.scrollHeight;
  try {
    const res = await fetch(panel.querySelector("[data-ask]").action, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ q }),
    });
    if (!res.ok) throw new Error(String(res.status));
    const r = await res.json();
    const li = el("li", "them", r.text);
    if (r.ai) li.prepend(el("span", "chip chip-info", "AI"), " ");
    if (r.sources && r.sources.length) {
      const s = el("span", "src", "From: ");
      for (const src of r.sources) s.append(link(src.href, src.title));
      li.append(s);
    }
    if (r.action) {
      const a = link(r.action.href, r.action.label || r.action.title);
      a.className = "btn btn-sm next";
      li.append(a);
    }
    wait.replaceWith(li);
  } catch {
    wait.replaceWith(el("li", "them", "The desk couldn't answer just now. Try again in a moment."));
  }
  thread.scrollTop = thread.scrollHeight;
}

if (desks) {
  for (const tab of desks.querySelectorAll("[data-desk-tab]")) {
    tab.addEventListener("click", () => {
      for (const t of desks.querySelectorAll("[data-desk-tab]")) t.setAttribute("aria-selected", String(t === tab));
      for (const p of desks.querySelectorAll("[data-desk]")) p.hidden = p.dataset.desk !== tab.dataset.deskTab;
    });
  }
  for (const panel of desks.querySelectorAll("[data-desk]")) {
    const form = panel.querySelector("[data-ask]");
    const input = form.querySelector("input");
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const q = input.value.trim();
      if (!q) return;
      input.value = "";
      ask(panel, q);
    });
    for (const b of panel.querySelectorAll("[data-starter]")) {
      b.addEventListener("click", () => ask(panel, b.textContent.trim()));
    }
  }
}
