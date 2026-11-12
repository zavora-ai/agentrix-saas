/* Agentrix · customer accounts on a site's own host (W6). Passkey first, then an email code.
   Talks to /account/api/* as JSON; webauthn-rs options and credentials travel as base64url.
   Every form also works without this script (code by email), except passkeys. */
(() => {
  const root = document.getElementById("acct");
  if (!root) return;
  const $ = id => document.getElementById(id);
  const kind = root.dataset.acct;
  const next = root.dataset.next || "/account";
  const err = $("acct-error");
  const show = m => { if (err) { err.hidden = false; err.textContent = m; err.focus && err.focus(); } };
  const clear = () => { if (err) { err.hidden = true; err.textContent = ""; } };
  const done = $("acct-done");
  const tell = m => { if (done) { done.hidden = false; done.textContent = m; done.focus && done.focus(); } };

  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const canPasskey = () => !!(window.PublicKeyCredential && navigator.credentials && window.isSecureContext);
  const creation = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); p.user.id = b64u.dec(p.user.id); (p.excludeCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const request = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const ext = c => (c.getClientExtensionResults ? c.getClientExtensionResults() : {});
  const registration = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { attestationObject: b64u.enc(c.response.attestationObject), clientDataJSON: b64u.enc(c.response.clientDataJSON) }, extensions: ext(c) });
  const assertion = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null }, extensions: ext(c) });

  async function call(url, body, method = "POST") {
    const r = await fetch(url, { method, credentials: "same-origin", headers: { "Content-Type": "application/json", Accept: "application/json" }, body: method === "POST" ? JSON.stringify(body || {}) : undefined });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) { const e = new Error(data.message || "That didn't work. Try again."); e.code = data.code; e.redirect = data.redirect; throw e; }
    return data;
  }
  const said = e => (e && e.name === "NotAllowedError") ? "The passkey prompt was closed or timed out, so nothing changed. Try again." : (e && e.message) || "That didn't work. Try again.";
  const go = d => location.assign((d && d.redirect) || next);
  const busy = (el, on) => { if (el) el.disabled = on; };
  const val = id => (($(id) && $(id).value) || "").trim();

  async function makePasskey(body, btn) {
    clear(); busy(btn, true);
    try {
      const start = await call("/account/api/passkey/register/start", body);
      const cred = await navigator.credentials.create(creation(start.options));
      return await call("/account/api/passkey/register/finish", { pending: start.pending, credential: registration(cred) });
    } catch (e) {
      if (e.code === "email-taken") show(e.message); else show(said(e));
      return null;
    } finally { busy(btn, false); }
  }

  // ---- sign in (A-01, A-14) ----
  if (kind === "sign-in") {
    const pk = $("acct-passkey");
    if (pk && canPasskey()) {
      pk.hidden = false;
      pk.addEventListener("click", async () => {
        clear();
        const email = val("acct-email");
        if (!email) { show("Type your email first, so we know which passkey to ask for."); $("acct-email").focus(); return; }
        busy(pk, true);
        try {
          const start = await call("/account/api/passkey/login/start", { email, next });
          const cred = await navigator.credentials.get(request(start.options));
          go(await call("/account/api/passkey/login/finish", { pending: start.pending, credential: assertion(cred) }));
        } catch (e) {
          if (e.code === "no-account" && e.redirect) { location.assign(e.redirect); return; }
          show(said(e));
          if (e.code === "no-passkey") $("acct-code").focus();
        } finally { busy(pk, false); }
      });
    }
  }

  // ---- create (A-03) ----
  if (kind === "create") {
    const pk = $("acct-passkey");
    if (pk && canPasskey()) {
      pk.hidden = false; $("acct-passkey-why").hidden = false;
      pk.addEventListener("click", async () => {
        const body = { name: val("acct-name"), email: val("acct-email"), phone: val("acct-phone"), next };
        if (!body.name || !body.email) { show("Add your name and email first."); return; }
        const d = await makePasskey(body, pk);
        if (d) go(d);
      });
    }
  }

  // ---- recovery codes (A-06): download as a text file ----
  if (kind === "recovery-codes") {
    const dl = $("acct-download");
    if (dl && window.Blob && window.URL) {
      dl.hidden = false;
      dl.addEventListener("click", () => {
        const codes = [...document.querySelectorAll("#acct-codes li")].map(li => li.textContent.trim());
        const text = "Recovery codes for " + root.dataset.site + "\nEach code works once.\n\n" + codes.join("\n") + "\n";
        const a = document.createElement("a");
        a.href = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
        a.download = root.dataset.site + "-recovery-codes.txt";
        document.body.appendChild(a); a.click(); a.remove();
      });
    }
  }

  // ---- one passkey offer per device (A-07) ----
  if (kind === "passkey-offer") {
    const pk = $("acct-passkey"), no = $("acct-not-now");
    if (!canPasskey()) { pk.hidden = true; }
    pk.addEventListener("click", async () => {
      const d = await makePasskey({ next }, pk);
      if (d) { pk.hidden = true; no.hidden = true; tell("Done: this device's screen lock signs you in to " + location.hostname + " only."); }
    });
    no.addEventListener("click", async () => {
      try { await call("/account/api/passkey/decline", {}); } catch (_) { /* the cookie is a nicety */ }
      pk.hidden = true; no.hidden = true;
      tell("Done. This device won't offer a passkey again. You can add one any time in Account, Sign-in and security.");
    });
  }

  // ---- recover (A-16) ----
  if (kind === "recover") {
    const form = $("acct-recover-form"), copy = $("acct-email-copy");
    $("acct-email").addEventListener("input", () => { copy.value = val("acct-email"); });
    form.addEventListener("submit", async ev => {
      ev.preventDefault(); clear();
      const btn = $("acct-use-recovery"); busy(btn, true);
      try { go(await call("/account/api/recovery/use", { email: val("acct-email"), code: val("acct-recovery") })); }
      catch (e) { show(said(e)); }
      finally { busy(btn, false); }
    });
  }

  // ---- security (A-09, A-19) ----
  if (kind === "security") {
    const again = () => location.assign("/account/sign-in?again=1&next=" + encodeURIComponent("/account/security"));
    document.querySelectorAll("[data-end-all]").forEach(b => b.addEventListener("click", async () => {
      busy(b, true);
      try { go(await call("/account/api/sessions/end", { all: true })); } catch (e) { show(said(e)); busy(b, false); }
    }));
    document.querySelectorAll("[data-end-session]").forEach(b => b.addEventListener("click", async () => {
      busy(b, true);
      try { await call("/account/api/sessions/end", { id: b.dataset.endSession }); b.closest("li").remove(); tell("Signed out on that device."); }
      catch (e) { show(said(e)); busy(b, false); }
    }));
    document.querySelectorAll("[data-remove-passkey]").forEach(b => b.addEventListener("click", async () => {
      busy(b, true);
      try { await call("/account/api/passkeys/remove", { id: b.dataset.removePasskey }); b.closest("li").remove(); tell("Passkey removed. It can't sign in here any more."); }
      catch (e) { if (e.code === "sign-in-again") again(); else { show(said(e)); busy(b, false); } }
    }));
    const was = $("acct-was-me");
    if (was) was.addEventListener("click", async () => {
      try { await call("/account/api/alerts/dismiss", {}); $("acct-alert").remove(); tell("Thanks. We'll warn you again if another new device signs in."); }
      catch (e) { show(said(e)); }
    });
    const add = $("acct-add-passkey");
    if (add && canPasskey()) {
      add.hidden = false;
      add.addEventListener("click", async () => { const d = await makePasskey({ next: "/account/security" }, add); if (d) location.reload(); });
    }
    const codes = $("acct-new-codes");
    if (codes) codes.addEventListener("click", async () => {
      busy(codes, true);
      try { go(await call("/account/api/recovery/new", {})); }
      catch (e) { if (e.code === "sign-in-again") again(); else { show(said(e)); busy(codes, false); } }
    });
  }

  // ---- your data: delete (A-12) ----
  if (kind === "data") {
    const del = $("acct-delete");
    if (del) del.addEventListener("click", async () => {
      if (!window.confirm("Delete your account for good? This can't be undone.")) return;
      busy(del, true); clear();
      try { go(await call("/account/api/delete", {})); }
      catch (e) {
        if (e.code === "sign-in-again") location.assign("/account/sign-in?again=1&next=" + encodeURIComponent("/account/data"));
        else { show(said(e)); busy(del, false); }
      }
    });
  }

  // ---- choices and consent receipts (A-08) ----
  if (kind === "choices") {
    document.querySelectorAll("input[data-purpose]").forEach(sw => sw.addEventListener("change", async () => {
      clear();
      const purpose = sw.dataset.purpose, on = sw.checked;
      const adultBox = $("acct-adult");
      const adult = purpose === "account-memory" ? !!(adultBox && adultBox.checked) : false;
      if (on && purpose === "account-memory" && !adult) { sw.checked = false; show("Account memory is for people 18 or over. Tick the box to say you are."); adultBox && adultBox.focus(); return; }
      sw.disabled = true;
      try { await call("/account/api/consents", { purpose, on, adult }); location.reload(); }
      catch (e) { sw.checked = !on; sw.disabled = false; show(said(e)); }
    }));
  }
})();
