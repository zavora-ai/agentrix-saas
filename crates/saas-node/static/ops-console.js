// Agentrix's real agents in the operator console (saas-node's ops_console.rs). The dock's agent
// marks show what each agent is doing now. The askline asks Agentrix's Chief, which answers on
// the governed path with its own tools, and its steps and answer appear above the bar as they
// come. The model's words are shown as text, never as markup.
(() => {
  const dock = document.querySelector("[data-agentrix-agents]");
  const form = document.querySelector("[data-agentrix-ask]");
  if (!dock && !form) return;
  const input = form ? form.querySelector("input") : null;
  const send = form ? form.querySelector("button") : null;

  const TOOLS = {
    company_brief: "the company's position",
    ar_ageing: "what customers owe",
    ap_bills_due: "the bills due",
    wallet_status: "the wallet record",
    brief_owner: "a brief to the owner",
  };
  const toolWords = (t) => TOOLS[t] || String(t || "").replace(/_/g, " ");

  const style = document.createElement("style");
  style.textContent = `
    .agx-answer { margin: 0 0 .5rem; padding: .75rem 1rem; border-radius: .75rem;
      background: var(--surface, #fff); border: 1px solid var(--line, #d9d9e3);
      max-height: 45vh; overflow: auto; font-size: .95rem; }
    .agx-answer header { display: flex; justify-content: space-between; gap: 1rem; align-items: baseline; }
    .agx-answer .q { font-weight: 600; }
    .agx-answer .steps { margin: .4rem 0; padding: 0; list-style: none; color: var(--muted, #666); font-size: .85rem; }
    .agx-answer .text { white-space: pre-wrap; margin: .4rem 0 0; }
    .agx-answer .note { color: var(--muted, #666); font-size: .85rem; margin: .4rem 0 0; }
    .agx-answer button { background: none; border: 0; cursor: pointer; color: inherit; font-size: 1.1rem; }
  `;
  document.head.appendChild(style);

  let panel = null;
  function openPanel(question) {
    if (!panel) {
      panel = document.createElement("section");
      panel.className = "agx-answer";
      panel.setAttribute("aria-live", "polite");
      panel.setAttribute("aria-label", "The Chief's answer");
      const cmd = form.closest(".cmd") || form.parentElement;
      cmd.insertBefore(panel, cmd.firstChild);
    }
    panel.replaceChildren();
    const head = document.createElement("header");
    const q = document.createElement("span");
    q.className = "q";
    q.textContent = question;
    const close = document.createElement("button");
    close.type = "button";
    close.setAttribute("aria-label", "Close the answer");
    close.textContent = "×";
    close.addEventListener("click", () => { panel.remove(); panel = null; });
    head.append(q, close);
    const steps = document.createElement("ul");
    steps.className = "steps";
    const text = document.createElement("p");
    text.className = "text";
    text.textContent = "Agentrix's Chief is thinking…";
    panel.append(head, steps, text);
    return { steps, text };
  }

  function note(where, words) {
    const p = document.createElement("p");
    p.className = "note";
    p.textContent = words;
    where.parentElement.appendChild(p);
  }

  async function refresh() {
    let data;
    try {
      const r = await fetch("/activity/agents.json", {
        credentials: "same-origin",
        headers: { Accept: "application/json" },
      });
      if (!r.ok) throw new Error(String(r.status));
      data = await r.json();
    } catch (_) {
      if (input && !form.dataset.busy) {
        input.placeholder = "Agentrix's Chief isn't reachable just now; the money layer may be starting.";
      }
      return;
    }
    for (const a of data.agents || []) {
      const el = dock && dock.querySelector(`[data-agent="${a.role}"]`);
      if (!el) continue;
      const mark = el.querySelector(".os-mark");
      if (mark) mark.dataset.state = a.mark;
      let words = `Agentrix's ${a.name}: ${a.words}.`;
      if (a.last) words += ` Last: ${a.last.what}, at ${a.last.at_words}.`;
      el.title = words;
      el.setAttribute("aria-label", words);
      el.dataset.agentState = a.state;
    }
    if (input && send && !form.dataset.busy) {
      const ready = !!(data.chief && data.chief.ready);
      input.disabled = !ready;
      send.disabled = !ready;
      input.placeholder = ready
        ? "Ask Agentrix's Chief: cash, what customers owe, bills due, the wallet…"
        : "Agentrix's Chief can't take questions now: it is frozen or switched off.";
    }
  }

  // The answer comes as server-sent events over a POST: read the stream and split it.
  async function ask(question) {
    const view = openPanel(question);
    form.dataset.busy = "1";
    input.disabled = true;
    send.disabled = true;
    let answered = false;
    try {
      const r = await fetch("/activity/ask", {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
        body: JSON.stringify({ question }),
      });
      if (!r.ok || !r.body) {
        let words = "The Chief couldn't take the question. Nothing was asked.";
        try { words = (await r.json()).message || words; } catch (_) {}
        view.text.textContent = words;
        return;
      }
      const reader = r.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let cut;
        while ((cut = buffer.indexOf("\n\n")) >= 0) {
          const block = buffer.slice(0, cut);
          buffer = buffer.slice(cut + 2);
          let event = "message";
          const data = [];
          for (const line of block.split("\n")) {
            if (line.startsWith("event:")) event = line.slice(6).trim();
            else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
          }
          if (!data.length) continue;
          let body;
          try { body = JSON.parse(data.join("\n")); } catch (_) { continue; }
          if (event === "step") {
            if (body.kind === "call") {
              const li = document.createElement("li");
              li.dataset.tool = body.tool;
              li.textContent = `Reading ${toolWords(body.tool)}…`;
              view.steps.appendChild(li);
            } else if (body.kind === "result") {
              const li = [...view.steps.querySelectorAll("li")]
                .reverse()
                .find((x) => x.dataset.tool === body.tool && !x.dataset.done);
              if (li) {
                li.dataset.done = "1";
                li.textContent = body.ok
                  ? `Read ${toolWords(body.tool)}.`
                  : `Couldn't read ${toolWords(body.tool)}.`;
              }
            } else if (body.kind === "held") {
              const li = document.createElement("li");
              li.textContent = `Held for the owner's passkey: ${toolWords(body.tool)}.`;
              view.steps.appendChild(li);
            } else if (body.kind === "text" && body.text) {
              view.text.textContent = body.text;
            }
          } else if (event === "done") {
            answered = true;
            if (body.error) {
              view.text.textContent = body.error;
            } else {
              view.text.textContent = body.text || "The Chief had nothing to add.";
              if (body.held && body.held.length) {
                note(view.text, `Waiting for the owner's passkey: ${body.held.map(toolWords).join(", ")}.`);
              }
              if (body.ended) note(view.text, `The turn ended early: ${body.ended}`);
            }
          }
        }
      }
      if (!answered) view.text.textContent = "The answer was cut off. Ask again in a moment.";
    } catch (_) {
      view.text.textContent = "The answer was cut off. Ask again in a moment.";
    } finally {
      delete form.dataset.busy;
      input.value = answered ? "" : input.value;
      refresh();
    }
  }

  if (form && input) {
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const question = input.value.trim();
      if (question && !form.dataset.busy) ask(question);
    });
  }

  refresh();
  setInterval(refresh, 20000);
})();
