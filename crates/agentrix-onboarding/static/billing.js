/* Agentrix onboarding · the billing island (W4): the tabs, and cancelling with the owner's
   approval. The approval is a passkey assertion over the server's options, or an authenticator
   code; the server checks it against the cancel's fingerprint before anything is cancelled. */
(() => {
  const root = document.getElementById("billing");
  if (!root) return;
  const $ = id => document.getElementById(id);
  const err = $("billing-error");
  const show = m => { err.hidden = false; err.textContent = m; };
  document.querySelectorAll('.j-tabs [role="tab"]').forEach(tab => tab.addEventListener("click", () => {
    tab.parentElement.querySelectorAll('[role="tab"]').forEach(t => {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      const panel = $(t.getAttribute("aria-controls")); if (panel) panel.hidden = !on;
    });
  }));
  const card = $("cancel-card");
  if (!card) return;
  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  async function send(body) {
    const r = await fetch("/billing/cancel", { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify({ site: root.dataset.site, ...body }) });
    const d = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(d.message || "That didn't work, and nothing was cancelled. Try again.");
    location.assign(d.redirect || location.href);
  }
  const pk = $("cancel-passkey");
  if (pk) pk.addEventListener("click", async () => {
    err.hidden = true; pk.disabled = true;
    try {
      const o = JSON.parse(card.dataset.options);
      const p = structuredClone(o.publicKey);
      p.challenge = b64u.dec(p.challenge);
      (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id));
      const c = await navigator.credentials.get({ publicKey: p });
      await send({ passkey: { id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null }, extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {} } });
    } catch (e) {
      show(e && e.name === "NotAllowedError" ? "The passkey prompt was closed or timed out. Nothing was cancelled." : e.message);
      pk.disabled = false;
    }
  });
  const form = $("cancel-totp");
  if (form) form.addEventListener("submit", async e => {
    e.preventDefault(); err.hidden = true;
    const btn = e.submitter; btn.disabled = true;
    try { await send({ totp: $("cancel-code").value.trim() }); } catch (x) { show(x.message); btn.disabled = false; }
  });
})();
