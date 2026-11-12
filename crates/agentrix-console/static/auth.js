/* Agentrix · sign-in and registration island (R2), on control-plane's JSON endpoints (S1-07):
   passkey ceremonies in webauthn-rs JSON (base64url), password + authenticator code, errors as {message}. */
(() => {
  const root = document.getElementById("auth");
  if (!root) return;
  const register = root.dataset.auth === "register";
  const next = root.dataset.next || "/os";
  const $ = id => document.getElementById(id);
  const err = $("auth-error");
  const show = m => { err.hidden = false; err.textContent = m; };
  const clear = () => { err.hidden = true; err.textContent = ""; };

  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const creation = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); p.user.id = b64u.dec(p.user.id); (p.excludeCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const request = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const ext = c => (c.getClientExtensionResults ? c.getClientExtensionResults() : {});
  const registration = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { attestationObject: b64u.enc(c.response.attestationObject), clientDataJSON: b64u.enc(c.response.clientDataJSON) }, extensions: ext(c) });
  const assertion = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null }, extensions: ext(c) });

  async function post(url, body) {
    const r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(body) });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Please try again.");
    return data;
  }
  const said = e => (e && e.name === "NotAllowedError") ? "The passkey prompt was closed or timed out. Nothing was created or signed." : (e && e.message) || "That didn't work. Please try again.";
  const go = d => location.assign((d && d.redirect) || next);
  const email = () => ($("email").value || "").trim();
  const name = () => (($("display-name") && $("display-name").value) || "").trim();
  const busy = (el, on) => { if (el) el.disabled = on; };

  // Passkey
  const pk = $("pk");
  if (pk) {
    if (!(window.PublicKeyCredential && navigator.credentials)) {
      pk.disabled = true; $("pw-wrap").open = true;
      show("This browser can't use passkeys. Use a password and your authenticator app below.");
    }
    pk.addEventListener("click", async () => {
      clear();
      if (!email()) { show("Enter your email first."); $("email").focus(); return; }
      busy(pk, true);
      try {
        if (register) {
          const opts = await post("/auth/passkey/register/start", { email: email(), display_name: name() || undefined, next });
          const cred = await navigator.credentials.create(creation(opts));
          go(await post("/auth/passkey/register/finish", registration(cred)));
        } else {
          const opts = await post("/auth/passkey/login/start", { email: email(), next });
          const cred = await navigator.credentials.get(request(opts));
          go(await post("/auth/passkey/login/finish", assertion(cred)));
        }
      } catch (e) { show(said(e)); busy(pk, false); }
    });
  }

  // Password, then the authenticator code
  let pending = null;
  $("pw").addEventListener("submit", async e => {
    e.preventDefault(); clear();
    if (!email()) { show("Enter your email first."); $("email").focus(); return; }
    const btn = e.submitter; busy(btn, true);
    try {
      const d = await post("/auth/password", { action: register ? "register" : "login", email: email(), password: $("password").value, display_name: name() || undefined, next });
      pending = d.pending;
      $("step-start").hidden = true;
      if (d.next === "totp-enrol") {
        $("qr").innerHTML = d.qr_svg || ""; // server-rendered from the seed only
        $("totp-key").textContent = d.key || "";
        $("step-totp-enrol").hidden = false;
      } else {
        $("step-totp").hidden = false;
      }
      $("code").focus();
    } catch (x) { show(said(x)); busy(btn, false); }
  });
  $("code-form").addEventListener("submit", async e => {
    e.preventDefault(); clear();
    const btn = e.submitter; busy(btn, true);
    try { go(await post("/auth/totp", { pending, code: $("code").value.trim() })); }
    catch (x) { show(said(x)); busy(btn, false); }
  });
})();
