/* The owner approves a support-access request: a passkey assertion over the grant's exact terms,
   or a fresh authenticator code bound to them. Options come from the server in data-options. */
(() => {
  const card = document.getElementById("grant");
  if (!card || card.dataset.method === "none") return;
  const $ = (id) => document.getElementById(id);
  const b64u = {
    dec: (s) => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: (buf) => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const say = (t) => { const e = $("err"); if (e) { e.textContent = t; e.hidden = !t; } };
  async function send(proof) {
    const url = card.dataset.approveUrl || `/support-access/${card.dataset.grant}/approve`;
    let r;
    try { r = await fetch(url, { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: JSON.stringify(proof) }); }
    catch (_) { throw new Error("We couldn't reach Agentrix, so we can't tell whether it was approved. Reload the page to see."); }
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || "That didn't work. Reload the page to see whether it was approved.");
    location.assign(data.next || location.href);
  }
  const pk = $("approve-passkey");
  if (pk) pk.addEventListener("click", async () => {
    say(""); pk.disabled = true;
    try {
      const o = JSON.parse(card.dataset.options);
      const p = structuredClone(o.publicKey);
      p.challenge = b64u.dec(p.challenge);
      (p.allowCredentials || []).forEach((c) => (c.id = b64u.dec(c.id)));
      const c = await navigator.credentials.get({ publicKey: p });
      await send({ method: "passkey", credential: {
        id: c.id, rawId: b64u.enc(c.rawId), type: c.type,
        response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null },
        extensions: c.getClientExtensionResults ? c.getClientExtensionResults() : {},
      } });
    } catch (e) {
      say(e && e.name === "NotAllowedError" ? "The passkey prompt was closed or timed out. Nothing was approved." : (e.message || "That didn't work."));
      pk.disabled = false;
    }
  });
  const use = $("use-code");
  if (use) use.addEventListener("click", () => { $("code-form").hidden = false; use.hidden = true; $("code").focus(); });
  const form = $("code-form");
  if (form) form.addEventListener("submit", async (ev) => {
    ev.preventDefault(); say("");
    try { await send({ method: "totp", code: $("code").value.trim() }); } catch (e) { say(e.message); }
  });
})();
