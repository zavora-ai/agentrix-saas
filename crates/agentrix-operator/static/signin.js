/* Operator sign-in: email, then a passkey assertion over the server's sign-in fingerprint, or an
   authenticator code when the account has no passkey. webauthn-rs JSON uses base64url. */
(() => {
  const form = document.getElementById("signin");
  if (!form) return;
  const $ = (id) => document.getElementById(id);
  const b64u = {
    dec: (s) => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const say = (text) => { const e = $("err"); e.textContent = text; e.hidden = !text; };
  async function post(url, body) {
    let r;
    try { r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(body) }); }
    catch (_) { throw new Error("We couldn't reach the control plane. Check your connection and try again."); }
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Try again.");
    return data;
  }
  let ceremony = null;
  const finish = async (proof) => {
    const done = await post("/signin/finish", { ceremony, email: $("email").value, next: form.dataset.next, proof });
    location.assign(done.next || "/");
  };
  async function passkey(options) {
    const p = structuredClone(options.publicKey);
    p.challenge = b64u.dec(p.challenge);
    (p.allowCredentials || []).forEach((c) => (c.id = b64u.dec(c.id)));
    const c = await navigator.credentials.get({ publicKey: p });
    return {
      id: c.id, rawId: b64u.enc(c.rawId), type: c.type,
      response: {
        authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON),
        signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null,
      },
      extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {},
    };
  }
  form.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    say("");
    const go = $("go");
    go.disabled = true;
    try {
      if (!$("code-row").hidden && ceremony) {
        await finish({ method: "totp", code: $("code").value.trim() });
        return;
      }
      const started = await post("/signin/start", { email: $("email").value.trim() });
      ceremony = started.ceremony;
      if (started.method === "passkey" && window.PublicKeyCredential) {
        try {
          await finish({ method: "passkey", credential: await passkey(started.options) });
          return;
        } catch (e) {
          if (e && e.name === "NotAllowedError") say("The passkey prompt was closed or timed out. Use a code from your authenticator app instead, or try again.");
          else throw e;
          const again = await post("/signin/start", { email: $("email").value.trim() });
          ceremony = again.ceremony;
        }
      }
      $("code-row").hidden = false;
      $("email").readOnly = true;
      go.textContent = "Sign in";
      $("code").focus();
    } catch (e) {
      say(e.message || "That didn't work. Try again.");
    } finally {
      go.disabled = false;
    }
  });
})();
