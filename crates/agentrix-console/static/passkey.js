/* Agentrix · passkey island for card approvals (design D9, webauthn-rs 0.5.5). Sign-in is static/auth.js.
   Options arrive as webauthn-rs JSON with base64url binary fields; credentials go back the same way.
   An approval's challenge is SHA-256(fingerprint ‖ nonce), set by the server, so the assertion commits to the card. */
(() => {
  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const supported = () => !!(window.PublicKeyCredential && navigator.credentials);

  function requestOptions(o) {
    const p = structuredClone(o.publicKey);
    p.challenge = b64u.dec(p.challenge);
    (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id));
    return { publicKey: p, ...(o.mediation ? { mediation: o.mediation } : {}) };
  }
  const assertion = c => ({
    id: c.id, rawId: b64u.enc(c.rawId), type: c.type,
    response: {
      authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON),
      signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null,
    },
    extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {},
  });
  async function post(url, body) {
    const r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(body) });
    if (r.redirected) { location.assign(r.url); return null; }
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Please try again.");
    return data;
  }
  const said = e => e && e.name === "NotAllowedError" ? "The passkey prompt was closed or timed out. Nothing was signed." : (e && e.message) || "That didn't work. Please try again.";

  // ---- Approving a card: the assertion is over the card's fingerprint ----
  const sheet = document.getElementById("approve");
  const btn = document.getElementById("approve-passkey");
  if (sheet && btn) {
    const err = document.getElementById("approve-error");
    if (!supported() || !sheet.dataset.options) { btn.disabled = true; err.hidden = false; err.textContent = supported() ? "Reload the page to approve with your passkey." : "This browser can't use passkeys. Use your authenticator app below."; }
    btn.addEventListener("click", async () => {
      err.hidden = true; btn.disabled = true;
      try {
        const cred = await navigator.credentials.get(requestOptions(JSON.parse(sheet.dataset.options)));
        const done = await post(sheet.dataset.approveUrl, { method: "passkey", credential: assertion(cred) });
        if (done) location.assign(done.redirect || location.href);
      } catch (e) { err.hidden = false; err.textContent = said(e); btn.disabled = false; }
    });
  }
})();
