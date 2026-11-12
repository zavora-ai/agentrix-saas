/* Agentrix · People (W6): invite and change roles with the owner's passkey over that exact
   grant, through /team/api/*. Without a passkey, an authenticator code. Removal is a form post. */
(() => {
  const root = document.getElementById("people");
  if (!root) return;
  const site = encodeURIComponent(root.dataset.site);
  const status = document.getElementById("people-status");
  const tell = (m, warn) => { status.hidden = false; status.textContent = m; status.classList.toggle("warn", !!warn); };
  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const request = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const assertion = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null }, extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {} });
  async function call(url, body) {
    const r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(body) });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Try again.");
    return data;
  }
  const said = e => (e && e.name === "NotAllowedError") ? "The passkey prompt was closed or timed out. Nobody was invited and nothing changed." : (e && e.message) || "That didn't work. Try again.";

  // Start, prove, finish. `codeInput` is used when the account has no passkey.
  async function proven(startUrl, startBody, finishUrl, codeInput) {
    const start = await call(startUrl, startBody);
    if (start.passkey) {
      const cred = await navigator.credentials.get(request(start.passkey));
      return call(finishUrl, { challenge: start.challenge, passkey: assertion(cred) });
    }
    const code = codeInput && codeInput.value.trim();
    if (!code) {
      if (codeInput) { codeInput.closest("label").hidden = false; codeInput.focus(); }
      throw new Error("Type the 6-digit code from your authenticator app, then try again.");
    }
    return call(finishUrl, { challenge: start.challenge, totp: code });
  }

  const invite = document.getElementById("people-invite");
  if (invite) invite.addEventListener("submit", async ev => {
    ev.preventDefault();
    const btn = invite.querySelector("button"); btn.disabled = true;
    const email = invite.elements.email.value.trim(), role = invite.elements.namedItem("role").value;
    try {
      await proven(`/team/api/sites/${site}/invite/start`, { email, role }, `/team/api/sites/${site}/invite`, invite.elements.totp);
      tell(`Invited ${email}. The email is on its way.`);
      setTimeout(() => location.reload(), 900);
    } catch (e) { tell(said(e), true); btn.disabled = false; }
  });

  document.querySelectorAll("form.people-role").forEach(f => f.addEventListener("submit", async ev => {
    ev.preventDefault();
    const btn = f.querySelector("button"); btn.disabled = true;
    const id = encodeURIComponent(f.dataset.member), role = f.elements.namedItem("role").value;
    try {
      await proven(`/team/api/sites/${site}/members/${id}/role/start`, { role }, `/team/api/sites/${site}/members/${id}/role`, invite && invite.elements.totp);
      location.reload();
    } catch (e) { tell(said(e), true); btn.disabled = false; }
  }));
})();
