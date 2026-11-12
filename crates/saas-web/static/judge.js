// saas-web · "Run the day" in judge mode: starts the story's run on the node and shows each
// scene as it runs, with the evidence it left (PayPal ids, ERA journals, audit entries).

const box = document.querySelector("[data-run-day]");

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const STATES = new Set(["pending", "running", "done", "failed"]);

/** Only an http(s) address becomes a link (PayPal's approve link, a sandbox page). */
function safeHref(href) {
  try {
    const u = new URL(href, window.location.href);
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}

// company's run: {done, scenes: [{n, scene, title, state, evidence: [{label, value, href, step}]}]},
// where `step` is the story step's words, shared by the ids that step left.
function draw(list, run) {
  list.replaceChildren(
    ...(run.scenes || []).map((s) => {
      const state = STATES.has(s.state) ? s.state : "pending";
      const li = el("li", `scene ${state}`);
      const head = el("div", "spread");
      head.append(el("b", "", `${s.n} · ${s.title || ""}`), el("span", "chip", state));
      li.append(head);
      let step = null;
      for (const ev of s.evidence || []) {
        if (ev.step && ev.step !== step) {
          step = ev.step;
          li.append(el("p", "small", step));
        }
        const p = el("p", "small mono");
        const text = `${ev.label}: ${ev.value ?? ""}`;
        const href = ev.href ? safeHref(ev.href) : null;
        if (href) {
          const a = el("a", "", text);
          a.href = href;
          a.target = "_blank";
          a.rel = "noopener noreferrer";
          p.append(a);
        } else {
          p.textContent = text;
        }
        li.append(p);
      }
      return li;
    }),
  );
}

async function poll(base, run, list, status, button) {
  try {
    const res = await fetch(`${base}/${encodeURIComponent(run)}`, { headers: { accept: "application/json" } });
    const body = await res.json();
    if (!res.ok) throw new Error(body.message || `The run answered ${res.status}.`);
    draw(list, body);
    if (body.done) {
      const failed = (body.scenes || []).filter((s) => s.state === "failed").length;
      status.textContent = failed
        ? `The day has run: ${failed} scene${failed > 1 ? "s" : ""} failed. The evidence above says where.`
        : "The day has run.";
      if (button) button.disabled = false;
      return;
    }
  } catch (e) {
    status.textContent = e.message;
    if (button) button.disabled = false;
    return;
  }
  setTimeout(() => poll(base, run, list, status, button), 1500);
}

if (box) {
  const button = box.querySelector("[data-run-day-start]");
  const list = box.querySelector("[data-run-day-scenes]");
  const status = box.querySelector("[data-run-day-status]");
  button?.addEventListener("click", async () => {
    button.disabled = true;
    status.textContent = "Starting the day…";
    try {
      const res = await fetch(box.dataset.url, { method: "POST", headers: { accept: "application/json" } });
      const body = await res.json();
      if (!res.ok) throw new Error(body.message || `The node answered ${res.status}.`);
      status.textContent = "Running…";
      poll(box.dataset.url, body.run, list, status, button);
    } catch (e) {
      status.textContent = e.message;
      button.disabled = false;
    }
  });
}
