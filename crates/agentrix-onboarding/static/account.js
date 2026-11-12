/* Agentrix onboarding · the account island (W4): create the account, sign in, get back in.
   Passkeys and password + authenticator: the first release's /auth/passkey/*, /auth/password, /auth/totp
   (webauthn-rs JSON, base64url). Email codes, recovery codes and recovery: W6's /auth/email/*,
   /auth/recovery-codes, /auth/api/recovery-codes/shown and /auth/recover/*. Errors arrive as
   {message, code}; the words on screen are the server's. Email never approves anything. */
(() => {
  const root = document.getElementById("account");
  if (!root) return;
  const mode = root.dataset.mode;
  const next = root.dataset.next || "/os";
  const host = root.dataset.host || "";
  const $ = id => document.getElementById(id);
  const val = id => (($(id) && $(id).value) || "").trim();
  const enc = encodeURIComponent;
  const err = $("account-error");
  const show = m => { if (!err) return; err.hidden = false; err.textContent = m; err.scrollIntoView({ block: "nearest" }); };
  const clear = () => { if (err) { err.hidden = true; err.textContent = ""; } };
  const busy = (el, on) => { if (el) el.disabled = on; };
  const go = to => location.assign(to);
  const setup = `/setup?next=${enc(next)}${host ? `&host=${enc(host)}` : ""}`;

  // ---------- Tabs (the concept's j-tabs) ----------
  document.querySelectorAll('.j-tabs [role="tab"]').forEach(tab => tab.addEventListener("click", () => {
    const list = tab.parentElement;
    list.querySelectorAll('[role="tab"]').forEach(t => {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      const panel = $(t.getAttribute("aria-controls")); if (panel) panel.hidden = !on;
    });
    clear();
  }));

  // ---------- Talking to the server ----------
  class Refused extends Error { constructor(message, code, status) { super(message); this.code = code; this.status = status; } }
  async function call(method, url, body) {
    const r = await fetch(url, {
      method, credentials: "same-origin",
      headers: { "Accept": "application/json", ...(body ? { "Content-Type": "application/json" } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) {
      // A route that isn't there at all (no JSON, no code) means the piece isn't switched on.
      if ((r.status === 404 || r.status === 405) && !data.code) throw new Refused("", "not-here", r.status);
      throw new Refused(data.message || "That didn't work, and nothing was changed. Try again.", data.code || "", r.status);
    }
    return data;
  }
  const post = (url, body) => call("POST", url, body || {});
  const said = (e, what) => {
    if (e && e.name === "NotAllowedError") return "The passkey prompt was closed or timed out. Nothing was created or signed.";
    if (e && e.code === "not-here") return `${what} aren't switched on here yet. Use a passkey, or a password and an authenticator app.`;
    return (e && e.message) || "That didn't work, and nothing was changed. Try again.";
  };

  // ---------- Passkeys (webauthn-rs JSON) ----------
  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const ext = c => (c.getClientExtensionResults ? c.getClientExtensionResults() : {});
  const creation = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); p.user.id = b64u.dec(p.user.id); (p.excludeCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const request = o => { const p = structuredClone(o.publicKey); p.challenge = b64u.dec(p.challenge); (p.allowCredentials || []).forEach(c => c.id = b64u.dec(c.id)); return { publicKey: p }; };
  const registration = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { attestationObject: b64u.enc(c.response.attestationObject), clientDataJSON: b64u.enc(c.response.clientDataJSON) }, extensions: ext(c) });
  const assertion = c => ({ id: c.id, rawId: b64u.enc(c.rawId), type: c.type, response: { authenticatorData: b64u.enc(c.response.authenticatorData), clientDataJSON: b64u.enc(c.response.clientDataJSON), signature: b64u.enc(c.response.signature), userHandle: c.response.userHandle ? b64u.enc(c.response.userHandle) : null }, extensions: ext(c) });
  // ---------- Already signed in in this browser ----------
  async function whoIsSignedIn() {
    try {
      const r = await fetch("/auth/session", { credentials: "same-origin", headers: { Accept: "application/json" } });
      if (!r.ok) return null;
      const d = await r.json();
      return d && (d.display_name || d.email) ? d : null;
    } catch (_) { return null; }
  }
  if ($("signed-in")) {
    whoIsSignedIn().then(d => {
      if (!d) return;
      const name = d.display_name || d.email;
      const put = (id, v) => { const el = $(id); if (el) el.textContent = v; };
      put("who", `Signed in as ${name}${d.display_name && d.email ? ` (${d.email})` : ""}.`);
      put("continue-as", `Continue as ${name}`);
    });
    $("sign-out").addEventListener("click", async () => {
      busy($("sign-out"), true);
      try { await fetch("/auth/signout", { method: "POST", credentials: "same-origin" }); } catch (_) { /* reload shows where it stands */ }
      location.reload();
    });
    return;
  }
  // A tab opened before someone signed in elsewhere still shows the form: check again before
  // creating anything, so nothing is ever added to the account that's signed in.
  async function refuseIfSignedIn() {
    const d = await whoIsSignedIn();
    if (!d) return false;
    show(`This browser is now signed in as ${d.display_name || d.email}, so nothing was created. Reload the page to sign out first.`);
    return true;
  }

  const passkeys = !!(window.PublicKeyCredential && navigator.credentials);
  const pk = $("pk-go");
  if (pk && !passkeys) {
    pk.disabled = true;
    show("This browser can't use passkeys. Use the other ways below.");
    const other = $("tab-other"); if (other) other.click();
  }

  // ---------- Recovery codes, shown once after the account is made ----------
  function showCodes(codes) {
    $("step-start").hidden = true;
    const enrol = $("step-totp-enrol"); if (enrol) enrol.hidden = true;
    const list = $("codes");
    codes.forEach(c => { const li = document.createElement("li"); li.textContent = c; list.append(li); });
    const text = `Agentrix recovery codes${host ? ` for ${host}` : ""}\nEach works once. Keep them away from the device you sign in on.\n\n${codes.join("\n")}\n`;
    $("codes-download").href = `data:text/plain;charset=utf-8,${enc(text)}`;
    $("codes-copy").addEventListener("click", async () => { try { await navigator.clipboard.writeText(codes.join("\n")); $("codes-copy").textContent = "Copied"; } catch (_) { /* no clipboard: Download still works */ } });
    $("codes-saved").addEventListener("change", e => { $("codes-done").disabled = !e.target.checked; });
    $("codes-done").addEventListener("click", () => go(setup));
    $("step-codes").hidden = false;
    $("step-codes").scrollIntoView({ block: "start" });
  }
  async function made(how, verified) {
    // The account exists and this browser is signed in. An email sign-up hands back a
    // one-time recovery id; a passkey or authenticator sign-up proves itself once more
    // (W6: new codes need a passkey or a code), then gets its codes.
    set("made-words", `✓ Account created · ${how} · signed in on this device`);
    // Signed in now, so the header's "Sign in" becomes the way to the console.
    const top = document.querySelector(".topbar-right a");
    if (top) { top.textContent = "Your console"; top.href = "/os"; }
    try {
      const r = verified && (verified.recovery || (verified.redirect && new URL(verified.redirect, location.origin).searchParams.get("r")));
      const codes = r ? (await call("GET", `/auth/api/recovery-codes/shown?r=${enc(r)}`)).codes : await provenCodes();
      if (Array.isArray(codes) && codes.length) { showCodes(codes); return; }
    } catch (_) { /* recovery codes not switched on here, or the prompt was closed: carry on */ }
    go(setup);
  }
  async function provenCodes() {
    const start = await post("/auth/recovery-codes/start");
    if (start.passkey) {
      set("made-words", `${$("made-words").textContent}. Touch your passkey once more to make your recovery codes.`);
      $("step-codes").hidden = true;
      const cred = await navigator.credentials.get(request(start.passkey));
      return (await post("/auth/recovery-codes", { challenge: start.challenge, passkey: assertion(cred) })).codes;
    }
    // No passkey: one more authenticator code.
    $("step-start").hidden = true;
    const enrol = $("step-totp-enrol"); if (enrol) enrol.hidden = true;
    const form = $("codes-totp"); form.hidden = false; $("codes-totp-code").focus();
    return new Promise((resolve, reject) => form.addEventListener("submit", async e => {
      e.preventDefault(); clear();
      const btn = e.submitter; busy(btn, true);
      try { const d = await post("/auth/recovery-codes", { challenge: start.challenge, totp: val("codes-totp-code") }); form.hidden = true; resolve(d.codes); }
      catch (x) { show(said(x, "Recovery codes")); busy(btn, false); if (x.code === "not-here") reject(x); }
    }));
  }
  const set = (id, v) => { const el = $(id); if (el) el.textContent = v; };

  // ---------- Email codes ----------
  let pending = null;
  async function sendCode(purpose, email, extra) {
    const btn = $("em-send"); clear();
    if (!email) { show("Enter your email first."); return; }
    busy(btn, true);
    try {
      const d = await post("/auth/email/start", { email, purpose, ...extra });
      pending = d.pending;
      set("em-sent", `We sent a 6-digit code to ${d.sent_to || email}. Check your spam folder if it isn't there in a minute.`);
      $("em-code-form").hidden = false;
      $("em-code").focus();
      btn.textContent = "Send another code";
    } catch (e) {
      show(said(e, "Email codes"));
      // An account with a passkey or an authenticator can't sign in by email alone (W6).
      if (e.code === "use-passkey" && err && root.dataset.recover) {
        const a = document.createElement("a"); a.href = root.dataset.recover; a.textContent = "Get back into your account";
        err.append(" ", a);
      }
    }
    busy(btn, false);
  }
  function onCode(handler) {
    const form = $("em-code-form"); if (!form) return;
    form.addEventListener("submit", async e => {
      e.preventDefault(); clear();
      const btn = e.submitter; busy(btn, true);
      try { await handler(await post("/auth/email/verify", { pending, code: val("em-code") })); }
      catch (x) { show(said(x, "Email codes")); busy(btn, false); }
    });
  }

  // ---------- Password, then the authenticator ----------
  function passwordFlow(action, email, onDone) {
    const form = $("pw-form"); if (!form) return;
    let pend = null;
    form.addEventListener("submit", async e => {
      e.preventDefault(); clear();
      if (!email()) { show("Enter your email first."); return; }
      if (action === "register" && await refuseIfSignedIn()) return;
      const btn = e.submitter; busy(btn, true);
      try {
        const d = await post("/auth/password", { action, email: email(), password: $("pw-password").value, display_name: val("em-name") || val("pk-name") || undefined, next });
        pend = d.pending;
        $("step-start").hidden = true;
        if (d.next === "totp-enrol") {
          $("qr").innerHTML = d.qr_svg || ""; // drawn by the server from the seed only
          set("totp-key", d.key || "");
          $("step-totp-enrol").hidden = false;
        } else {
          $("step-totp").hidden = false;
        }
        $("totp-code").focus();
      } catch (x) { show(said(x, "Passwords")); busy(btn, false); }
    });
    const codeForm = $("totp-form") || $("step-totp");
    codeForm.addEventListener("submit", async e => {
      e.preventDefault(); clear();
      const btn = e.submitter; busy(btn, true);
      try { onDone(await post("/auth/totp", { pending: pend, code: val("totp-code") })); }
      catch (x) { show(said(x, "Authenticator codes")); busy(btn, false); }
    });
  }

  // ---------- Create the account ----------
  if (mode === "new") {
    pk.addEventListener("click", async () => {
      clear();
      const email = val("pk-email");
      if (!email) { show("Enter your email first."); $("pk-email").focus(); return; }
      if (await refuseIfSignedIn()) return;
      busy(pk, true);
      try {
        const opts = await post("/auth/passkey/register/start", { email, display_name: val("pk-name") || undefined, next });
        const cred = await navigator.credentials.create(creation(opts));
        await post("/auth/passkey/register/finish", registration(cred));
        await made("passkey on this device");
      } catch (e) { show(said(e, "Passkeys")); busy(pk, false); }
    });
    $("em-send").addEventListener("click", async () => {
      clear();
      if (await refuseIfSignedIn()) return;
      sendCode("signup", val("em-email"), { display_name: val("em-name") || undefined, next: setup });
    });
    onCode(d => made("signed in with an email code", d));
    passwordFlow("register", () => val("em-email"), () => made("password and authenticator app"));
  }

  // ---------- Sign in ----------
  if (mode === "signin") {
    pk.addEventListener("click", async () => {
      clear();
      const email = val("si-email");
      if (!email) { show("Enter your email first."); $("si-email").focus(); return; }
      busy(pk, true);
      try {
        const opts = await post("/auth/passkey/login/start", { email, next });
        const cred = await navigator.credentials.get(request(opts));
        const d = await post("/auth/passkey/login/finish", assertion(cred));
        go(d.redirect || next);
      } catch (e) { show(said(e, "Passkeys")); busy(pk, false); }
    });
    $("em-send").addEventListener("click", () => sendCode("signin", val("si-email"), { next }));
    onCode(d => go(d.redirect || next));
    passwordFlow("login", () => val("si-email"), d => go(d.redirect || next));
  }

  // ---------- Get back in ----------
  if (mode === "recover") {
    // A signed-in owner who followed "Stop the recovery" from the email sees each hold here.
    (async () => {
      let holds = [];
      try { holds = (await call("GET", "/auth/recover/holds")).holds || []; } catch (_) { return; /* signed out, or holds aren't switched on here */ }
      if (!holds.length) return;
      const list = $("holds");
      holds.forEach(h => {
        const li = document.createElement("li");
        li.innerHTML = '<div><b></b><span class="small muted"></span></div><button class="btn btn-sm btn-coral" type="button">Stop the recovery</button>';
        li.querySelector("b").textContent = `Opens ${h.release}`;
        li.querySelector("span").textContent = `Asked ${h.requested}${h.device ? ` from ${h.device}` : ""}`;
        const btn = li.querySelector("button");
        btn.addEventListener("click", async () => {
          clear(); busy(btn, true);
          try {
            await post("/auth/recover/cancel", { hold: h.id });
            li.remove();
            set("stop-done", "Stopped. Nobody gets into your account this way, and nothing else changed. Consider adding a second passkey.");
            $("stop-done").hidden = false;
          } catch (e) { show(said(e, "Recovery holds")); busy(btn, false); }
        });
        list.append(li);
      });
      $("step-stop").hidden = false;
    })();

    $("t-code").addEventListener("submit", async e => {
      e.preventDefault(); clear();
      const btn = e.submitter; busy(btn, true);
      try { const d = await post("/auth/recover/code", { email: val("rc-email"), code: val("rc-code") }); go(d.redirect || next); }
      catch (x) { show(said(x, "Recovery codes")); busy(btn, false); }
    });
    $("em-send").addEventListener("click", () => sendCode("recover", val("re-email"), { next }));
    onCode(d => {
      if (d.held) {
        $("step-start").hidden = true;
        set("held-words", `For your safety, your account opens at ${d.release_at}. Come back then and do the same two steps.`);
        $("step-held").hidden = false;
        return;
      }
      go(d.redirect || next);
    });
  }
})();
