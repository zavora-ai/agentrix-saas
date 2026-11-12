// saas-web · the Verify box: resolves an agent's DID through the node and shows every check,
// green when they all hold, or the exact check that failed.

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

for (const form of document.querySelectorAll("[data-verify]")) {
  const out = form.querySelector("[data-verify-result]");
  const input = form.querySelector("input[name=did]");
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const did = input.value.trim();
    if (!did) return;
    out.className = "verify-result busy";
    out.replaceChildren(el("p", "small muted", "Resolving the DID from the site's own domain…"));
    try {
      // POST, so the same box works inside Agentrix OS, where an app's GET returns a page.
      const res = await fetch(form.action, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ did }),
      });
      const v = await res.json();
      if (!res.ok) throw new Error(v.message || `The verifier answered ${res.status}.`);
      out.className = `verify-result ${v.verified ? "ok" : "risk"}`;
      const head = el("p", "verdict", v.verified ? "Verified" : `Not verified: ${v.failure ? v.failure.message : "a check failed"}`);
      const list = el("ol", "checks");
      for (const c of v.checks || []) {
        const li = el("li", c.ok ? "ok" : "risk");
        li.append(el("b", "", `${c.ok ? "✓" : "✗"} ${c.check}`));
        if (c.detail) li.append(el("span", "small muted", ` · ${c.detail}`));
        list.append(li);
      }
      out.replaceChildren(head, list);
    } catch (err) {
      out.className = "verify-result risk";
      out.replaceChildren(el("p", "verdict", err.message || "The verifier didn't answer."));
    }
  });
}

// The token check: who an agent token names, and who vouched for it (saas-badge).
for (const form of document.querySelectorAll("[data-verify-token]")) {
  const out = form.querySelector("[data-verify-result]");
  const input = form.querySelector("textarea[name=token]");
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const token = input.value.trim();
    if (!token) return;
    out.className = "verify-result busy";
    out.replaceChildren(el("p", "small muted", "Checking the token against its issuer's published keys…"));
    try {
      const res = await fetch(form.action, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ token }),
      });
      const v = await res.json();
      if (!res.ok) throw new Error(v.message || `The checker answered ${res.status}.`);
      const who = v.principal?.principal?.id || v.principal?.subject || "an agent";
      out.className = `verify-result ${v.ok ? "ok" : "risk"}`;
      const parts = [el("p", "verdict", v.ok ? `Accepted: ${who}` : `Refused: ${v.refused || "a check failed"}`)];
      if (v.vouched_by) parts.push(el("p", "small", `Vouched for by ${v.vouched_by}`));
      if (v.badge) {
        parts.push(el("p", "small", v.badge.verified ? "Its badge verified." : `Its badge didn't verify: ${v.badge.failure?.message || "a check failed"}`));
        const list = el("ol", "checks");
        for (const c of v.badge.checks || []) {
          const li = el("li", c.ok ? "ok" : "risk");
          li.append(el("b", "", `${c.ok ? "✓" : "✗"} ${c.check || c.name}`));
          if (c.detail) li.append(el("span", "small muted", ` · ${c.detail}`));
          list.append(li);
        }
        parts.push(list);
      }
      out.replaceChildren(...parts);
    } catch (err) {
      out.className = "verify-result risk";
      out.replaceChildren(el("p", "verdict", err.message || "The checker didn't answer."));
    }
  });
}
