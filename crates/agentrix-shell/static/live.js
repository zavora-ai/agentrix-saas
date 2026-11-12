// Live: the open conversation polls for new turns every 3 seconds and appends them. Take over,
// Send and Hand back are plain forms (each redirects back), so the page works without this.
const view = document.querySelector("[data-live]");
if (view) {
  const thread = view.querySelector("[data-thread]");
  const state = view.querySelector("[data-state]");
  const mode = view.querySelector("[data-mode]");
  let after = Number(view.dataset.after || 0);
  let owner = mode && mode.classList.contains("chip-ok");

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text) n.textContent = text;
    return n;
  };

  function add(t) {
    if (thread.querySelector(`[data-seq="${t.seq}"]`)) return;
    const empty = thread.querySelector("p.muted");
    if (empty) empty.remove();
    const m = el("div", `msg ${t.who}`);
    m.dataset.seq = t.seq;
    if (t.by) {
      const by = el("span", "by", `${t.by} `);
      by.append(el("time", "", t.at_words));
      m.append(by);
    }
    m.append(document.createTextNode(t.text));
    if (t.cites && t.cites.length) {
      const c = el("span", "cites");
      for (const s of t.cites) c.append(el("span", "cite", s));
      m.append(c);
    }
    thread.append(m);
  }

  async function poll() {
    if (document.hidden) return;
    try {
      const r = await fetch(`${view.dataset.turns}?after=${after}`, { headers: { accept: "application/json" }, cache: "no-store" });
      if (!r.ok) return;
      const d = await r.json();
      for (const t of d.turns) {
        add(t);
        after = Math.max(after, t.seq);
      }
      if (state) state.lastChild.textContent = d.state_words;
      // The desk took it back after 10 quiet minutes, or another device changed it: reload the controls.
      if (d.owner !== owner) location.reload();
    } catch {
      // Offline for a moment: the next poll tries again.
    }
  }
  setInterval(poll, 3000);

  const say = view.querySelector("[data-say]");
  if (say) {
    say.addEventListener("submit", async (e) => {
      e.preventDefault();
      const input = say.querySelector("input");
      const text = input.value.trim();
      if (!text) return;
      input.disabled = true;
      try {
        const r = await fetch(say.action, {
          method: "POST",
          headers: { accept: "application/json", "content-type": "application/x-www-form-urlencoded" },
          body: new URLSearchParams({ text }),
        });
        const d = await r.json().catch(() => ({}));
        if (r.ok) {
          input.value = "";
          await poll();
        } else if (window.toast) {
          window.toast(d.message || "That didn't send. Try again.");
        }
      } finally {
        input.disabled = false;
        input.focus();
      }
    });
    say.querySelector("input").focus();
  }
}
