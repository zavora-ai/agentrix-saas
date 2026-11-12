/* Operator console islands. Pages read fine without them; the actions need them.
   An action follows the decision contract: prepare (the server checks it and returns exactly
   what will happen), then approve that exact effect with a passkey or an authenticator code. */
(() => {
  const b64u = {
    dec: (s) => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  // A network failure ("Failed to fetch") becomes `offline`, words the person can act on.
  async function post(url, body, offline) {
    let r;
    try { r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(body) }); }
    catch (_) { throw new Error(offline); }
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Reload the page to see what was saved.");
    return data;
  }
  // Going to the page we're on (with a #hash) wouldn't reload it, so reload instead.
  function go(next) {
    const url = new URL(next || location.href, location.href);
    if (url.pathname === location.pathname && url.search === location.search) { location.hash = url.hash; location.reload(); }
    else location.assign(url.href);
  }
  async function assertion(options) {
    const p = structuredClone(options.publicKey);
    p.challenge = b64u.dec(p.challenge);
    (p.allowCredentials || []).forEach((c) => (c.id = b64u.dec(c.id)));
    const c = await navigator.credentials.get({ publicKey: p });
    return {
      id: c.id, rawId: b64u.enc(c.rawId), type: c.type,
      response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null },
      extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {},
    };
  }

  // Each [data-action] form or button owns a .action-sheet where the effect and the approval show.
  function bind(el, collect) {
    const host = el.closest("[data-action-host]") || el.parentElement;
    const sheet = host.querySelector(".action-sheet");
    if (!sheet) return;
    const err = document.createElement("div");
    err.className = "err"; err.hidden = true; err.setAttribute("role", "alert");
    const say = (t) => { err.textContent = t; err.hidden = !t; };
    const start = async (ev) => {
      if (ev) ev.preventDefault();
      say("");
      let prepared;
      try { prepared = await post("/actions/prepare", collect(), "We couldn't reach the control plane, so nothing was checked or changed. Check your connection and try again."); } catch (e) { say(e.message); sheet.replaceChildren(err); sheet.classList.add("on"); return; }
      sheet.innerHTML = prepared.effect;
      const row = document.createElement("div");
      row.className = "code-row";
      const finish = async (proof) => {
        say("");
        try { const done = await post(`/actions/${prepared.id}/approve`, { proof }, "We couldn't reach the control plane, so we can't tell whether your approval arrived. Reload the page to see what was saved."); go(done.next); } catch (e) { say(e.message); }
      };
      if (prepared.method === "passkey" && window.PublicKeyCredential) {
        const b = document.createElement("button");
        b.type = "button"; b.className = "btn btn-primary"; b.textContent = "Approve with passkey";
        b.addEventListener("click", async () => {
          b.disabled = true;
          try { await finish({ method: "passkey", credential: await assertion(prepared.options) }); }
          catch (e) { say(e && e.name === "NotAllowedError" ? "The passkey prompt was closed or timed out. Nothing changed." : e.message); }
          b.disabled = false;
        });
        row.append(b);
      }
      const code = document.createElement("input");
      code.inputMode = "numeric"; code.autocomplete = "one-time-code"; code.maxLength = 6; code.placeholder = "6 digits";
      code.setAttribute("aria-label", "Code from your authenticator app");
      const ok = document.createElement("button");
      ok.type = "button"; ok.className = prepared.method === "passkey" ? "btn" : "btn btn-primary";
      ok.textContent = "Approve with code";
      ok.addEventListener("click", () => finish({ method: "totp", code: code.value.trim() }));
      const cancel = document.createElement("button");
      cancel.type = "button"; cancel.className = "btn btn-ghost"; cancel.textContent = "Cancel";
      cancel.addEventListener("click", () => { sheet.classList.remove("on"); sheet.replaceChildren(); });
      row.append(code, ok, cancel);
      sheet.append(row, err);
      sheet.classList.add("on");
    };
    if (el.tagName === "FORM") el.addEventListener("submit", start); else el.addEventListener("click", start);
  }
  document.querySelectorAll("form[data-action]").forEach((form) => {
    bind(form, () => {
      const body = { action: form.dataset.action };
      new FormData(form).forEach((v, k) => { body[k] = String(v); });
      return body;
    });
  });
  document.querySelectorAll("button[data-action-json]").forEach((btn) => bind(btn, () => JSON.parse(btn.dataset.actionJson)));
})();
