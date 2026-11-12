/* Ask this site: the island on every converted page (R8.2, R8.3; 06 §4.2, §4.5, §4.6).
   Served by the edge at /_agentrix/ask.js; the theme links it with `defer`.
   Posts to /ask and reads the SSE frames: ready, awp.presence, delta, awp.citation, reply,
   error, done. When the site doesn't say, the offer is a small form that posts to /ask/pass.
   Text from the server is only ever set with textContent. */
(() => {
  "use strict";
  if (document.querySelector(".ax-ask")) return;

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  };
  const sameOrigin = (path, anchor) => {
    if (typeof path !== "string" || !path.startsWith("/") || path.startsWith("//")) return null;
    return anchor ? `${path}#${encodeURIComponent(anchor)}` : path;
  };

  const root = el("aside", "ax-ask");
  root.setAttribute("aria-label", "Ask this site");
  const head = el("div", "ax-head");
  const mark = el("span", "ax-mark");
  mark.setAttribute("aria-hidden", "true");
  mark.dataset.state = "idle";
  const who = el("div", "ax-who");
  who.append(el("b", null, "Concierge"), el("span", "ax-ai", "AI"));
  const sub = el("span", "ax-sub", "Answers from this site");
  who.append(sub);
  const toggle = el("button", "ax-toggle", "Ask");
  toggle.type = "button";
  toggle.setAttribute("aria-expanded", "false");
  head.append(mark, who, toggle);

  const panel = el("div", "ax-panel");
  panel.hidden = true;
  panel.id = "ax-panel";
  toggle.setAttribute("aria-controls", panel.id);
  const disclosure = el("p", "ax-first", "You're talking to AI. It never guesses prices, dates or allergens.");
  const log = el("div", "ax-log");
  log.setAttribute("role", "log");
  log.setAttribute("aria-label", "Answers");
  const status = el("p", "ax-status");
  status.setAttribute("role", "status");
  const form = el("form", "ax-form");
  const label = el("label", "ax-sr", "Ask a question about this site");
  label.htmlFor = "ax-q";
  const input = el("input", "ax-input");
  input.id = "ax-q";
  input.name = "q";
  input.autocomplete = "off";
  input.maxLength = 800;
  input.placeholder = "Ask about this site";
  const send = el("button", "ax-send", "Ask");
  send.type = "submit";
  form.append(label, input, send);
  panel.append(disclosure, log, status, form);
  root.append(head, panel);
  document.body.append(root);

  const open = (yes) => {
    panel.hidden = !yes;
    toggle.setAttribute("aria-expanded", String(yes));
    toggle.textContent = yes ? "Close" : "Ask";
    root.classList.toggle("ax-open", yes);
    if (yes) input.focus();
  };
  toggle.addEventListener("click", () => open(panel.hidden));
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !panel.hidden) { open(false); toggle.focus(); }
  });

  const PRESENCE = { working: "Reading this site", answering: "Answering", idle: "", resting: "Answers are paused right now" };
  const setPresence = (state, label) => {
    mark.dataset.state = state || "idle";
    const words = label || PRESENCE[state] || "";
    sub.textContent = words || "Answers from this site";
    status.textContent = words;
  };

  /* Renders the final answer: [n] markers become links to their sources. */
  const renderAnswer = (box, text, cites) => {
    box.textContent = "";
    let at = 0;
    const re = /\[(\d+)\]/g;
    let m;
    while ((m = re.exec(text))) {
      box.append(document.createTextNode(text.slice(at, m.index)));
      const c = cites.get(Number(m[1]));
      const href = c && sameOrigin(c.path, c.anchor);
      if (href) {
        const a = el("a", "ax-cite", m[1]);
        a.href = href;
        a.setAttribute("aria-label", `Source ${m[1]}: ${c.title}`);
        box.append(a);
      }
      at = m.index + m[0].length;
    }
    box.append(document.createTextNode(text.slice(at)));
  };

  let passForms = 0;
  async function ask(question) {
    const turn = el("div", "ax-turn");
    turn.append(el("p", "ax-me", question));
    const answer = el("div", "ax-answer");
    answer.setAttribute("aria-busy", "true");
    const text = el("p", "ax-text");
    const sources = el("ol", "ax-sources");
    sources.hidden = true;
    answer.append(text, sources);
    turn.append(answer);
    log.append(turn);
    send.disabled = true;
    const cites = new Map();

    const finish = () => {
      answer.setAttribute("aria-busy", "false");
      send.disabled = false;
    };
    /* "Pass your question to the owner" (R8.3): a small form with the question prefilled and
       an optional way to reply. Posts to /ask/pass; the result is announced by role=status.
       The question is stored for the owner as data; it never reaches a model. */
    const offer = (label, href) => {
      const n = ++passForms;
      const box = el("form", "ax-pass");
      box.setAttribute("aria-label", label);
      const title = el("p", "ax-pass-title", label);
      const qLabel = el("label", "ax-pass-label", "Your question");
      qLabel.htmlFor = `ax-pass-q-${n}`;
      const q = el("textarea", "ax-pass-field");
      q.id = qLabel.htmlFor;
      q.rows = 2;
      q.maxLength = 500;
      q.required = true;
      q.value = question.slice(0, 500);
      const cLabel = el("label", "ax-pass-label", "How can the owner reach you? (optional)");
      cLabel.htmlFor = `ax-pass-c-${n}`;
      const c = el("input", "ax-pass-field");
      c.id = cLabel.htmlFor;
      c.maxLength = 120;
      c.autocomplete = "email";
      c.placeholder = "Phone or email";
      const go = el("button", "ax-send ax-pass-send", "Send to the owner");
      go.type = "submit";
      const result = el("p", "ax-pass-result");
      result.setAttribute("role", "status");
      box.append(title, qLabel, q, cLabel, c, go, result);
      const safe = sameOrigin(href);
      if (safe) {
        const a = el("a", "ax-offer-link", "Or contact the owner directly");
        a.href = safe;
        const alt = el("p", "ax-offer");
        alt.append(a);
        box.append(alt);
      }
      const failed = "That didn't go through. Please use the contact page.";
      box.addEventListener("submit", async (e) => {
        e.preventDefault();
        const asked = q.value.trim();
        if (!asked) { result.textContent = "Write the question you'd like to pass on."; q.focus(); return; }
        go.disabled = true;
        result.textContent = "Sending";
        try {
          const contact = c.value.trim();
          const r = await fetch("/ask/pass", {
            method: "POST",
            headers: { "content-type": "application/json", accept: "application/json" },
            body: JSON.stringify(contact ? { question: asked, contact } : { question: asked }),
          });
          let words = "";
          try { words = (await r.json()).message || ""; } catch { /* plain words below */ }
          result.textContent = words || (r.ok ? "Sent to the owner." : failed);
          if (r.ok) { q.disabled = true; c.disabled = true; } else { go.disabled = false; }
        } catch {
          result.textContent = failed;
          go.disabled = false;
        }
      });
      answer.append(box);
    };

    let res;
    try {
      res = await fetch("/ask", {
        method: "POST",
        headers: { "content-type": "application/json", accept: "text/event-stream" },
        body: JSON.stringify({ question }),
      });
    } catch {
      text.textContent = "Answers are paused right now. The page still works.";
      setPresence("resting");
      finish();
      return;
    }
    if (!res.ok || !res.body) {
      text.textContent = "Answers are paused right now. The page still works.";
      setPresence("resting");
      finish();
      return;
    }

    const handle = (event, data) => {
      switch (event) {
        case "ready": break;
        case "awp.presence": setPresence(data.state, data.label); break;
        case "delta": text.textContent += data.text || ""; break;
        case "awp.citation": {
          cites.set(data.n, data);
          const href = sameOrigin(data.path, data.anchor);
          const li = el("li", "ax-source");
          li.value = data.n;
          if (href) {
            const a = el("a", null, data.title || data.path);
            a.href = href;
            li.append(a);
          } else {
            li.textContent = data.title || "";
          }
          if (data.quote) li.title = data.quote;
          sources.append(li);
          sources.hidden = false;
          break;
        }
        case "reply":
          if (data.kind === "answer") {
            renderAnswer(text, data.text || "", cites);
          } else {
            text.textContent = data.text || "";
            sources.textContent = "";
            sources.hidden = true;
            if (data.offer) offer(data.offer, data.offer_href);
          }
          break;
        case "error":
          text.textContent = data.message || "Answers are paused right now. The page still works.";
          break;
        case "done": finish(); break;
      }
    };

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = "";
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      let cut;
      while ((cut = buf.indexOf("\n\n")) >= 0) {
        const block = buf.slice(0, cut);
        buf = buf.slice(cut + 2);
        let event = "message", data = "";
        for (const line of block.split("\n")) {
          if (line.startsWith("event:")) event = line.slice(6).trim();
          else if (line.startsWith("data:")) data += line.slice(5).trim();
        }
        if (!data) continue;
        try { handle(event, JSON.parse(data)); } catch { /* ignore a malformed frame */ }
      }
    }
    finish();
  }

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const q = input.value.trim();
    if (!q || send.disabled) return;
    input.value = "";
    ask(q);
  });
})();
