/* Owner console: the approval sheet (01b C8; design §4.1, D9). An ES module on agentrix-ui's kit.js.
   A card's Approve opens the sheet with a fresh ceremony from GET {sheet_url}: the card's
   fingerprint and passkey options whose challenge is SHA-256(fingerprint ‖ nonce). The browser's
   assertion, or a single-use authenticator code bound to the same fingerprint, goes to the card's
   own approve URL in the frozen contract's shape. Esc and Cancel close the sheet; neither decides.
   Typing a code doesn't approve until the owner presses the button. */
import { openOverlay } from "/static/ui/kit.js";

let showData = () => {};
/** Opens the approval sheet for a fingerprint and ceremony another island already has. */
export function openSheetData(data, from) { showData(data, from); }
let showUrl = () => {};
/** Opens the approval sheet for a card by its sheet URL (a fresh ceremony from the server). */
export function openSheetUrl(url, from) { showUrl(url, from); }

const $ = (s, r = document) => r.querySelector(s);
const overlay = document.getElementById("passkey");
if (overlay) {
  const box = $(".passkey", overlay);
  const hasPasskey = box.dataset.hasPasskey === "true";
  const err = $(".sheet-error", box);
  const code = $(".stepup-code", box);
  let sheet = null; // { title, sign_words, fingerprint, fingerprint_short, approve_url, has_passkey, passkey_options }
  let card = null;  // the [data-decision] element the sheet was opened from

  const b64u = {
    dec: s => { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); const u = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) u[i] = b.charCodeAt(i); return u.buffer; },
    enc: buf => btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""),
  };
  const supported = () => !!(window.PublicKeyCredential && navigator.credentials);
  function requestOptions(o) {
    const p = structuredClone(o.publicKey);
    p.challenge = b64u.dec(p.challenge);
    (p.allowCredentials || []).forEach(c => { c.id = b64u.dec(c.id); });
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

  function say(text) { err.hidden = !text; err.textContent = text || ""; }
  function busy(on) { box.classList.toggle("signing", on); box.querySelectorAll("[data-do-sign]").forEach(b => { b.disabled = on; }); }
  function stepup(on) {
    box.classList.toggle("stepup", on || !hasPasskey);
    const label = $("[data-sig-label]", box);
    if (label) label.textContent = (on || !hasPasskey) ? "Your code approves:" : "Your passkey signs:";
    if (on || !hasPasskey) setTimeout(() => code && code.focus(), 30);
  }
  function reset() {
    box.classList.remove("signing", "signed");
    stepup(false);
    say("");
    if (code) { code.value = ""; code.classList.remove("bad"); code.placeholder = "6 digits"; }
  }

  async function json(url, opts) {
    const r = await fetch(url, { credentials: "same-origin", headers: { Accept: "application/json", ...(opts && opts.body ? { "Content-Type": "application/json" } : {}) }, ...opts });
    const data = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(data.message || data.error || "That didn't work. Nothing was changed; try again.");
    return data;
  }

  // The sheet for an edited card: POST the owner's edits, get back what the approval will sign.
  let refetch = null;
  async function openSheet(url, method, from, body) {
    card = from;
    reset();
    refetch = body ? () => json(url, { method: "POST", body: JSON.stringify(body) }) : null;
    $("[data-sign-what]", box).textContent = "Getting the card…";
    $("[data-sign-fp]", box).textContent = "";
    openOverlay("passkey", from && from.querySelector("[data-sheet]"));
    try {
      sheet = await json(url, method === "POST" ? { method: "POST", ...(body ? { body: JSON.stringify(body) } : {}) } : undefined);
      $("[data-sign-what]", box).textContent = sheet.sign_words || sheet.title;
      $("[data-sign-fp]", box).textContent = sheet.fingerprint_short ? `${sheet.fingerprint_short} (${sheet.fingerprint})` : sheet.fingerprint;
      if (hasPasskey && (!sheet.passkey_options || !supported())) {
        stepup(true);
        say(supported() ? "" : "This browser can't use passkeys. Use a code from your authenticator app.");
      }
    } catch (e) {
      sheet = null;
      $("[data-sign-what]", box).textContent = "";
      say(e.message);
    }
  }

  showUrl = (url, from) => openSheet(url, "GET", from);

  // A sheet whose fingerprint and ceremony another island already has (W1's dial raises).
  showData = (data, from) => {
    card = from;
    reset();
    sheet = data;
    $("[data-sign-what]", box).textContent = data.sign_words || data.title;
    $("[data-sign-fp]", box).textContent = data.fingerprint;
    openOverlay("passkey", from && from.querySelector("button"));
    if (hasPasskey && (!data.passkey_options || !supported())) {
      stepup(true);
      say(supported() ? "" : "This browser can't use passkeys. Use a code from your authenticator app.");
    }
  };

  function finish(redirect, words) {
    box.classList.remove("signing");
    box.classList.add("signed");
    const ok = $(".ok", box);
    if (ok && words) ok.textContent = words;
    if (card) {
      card.classList.add("done");
      const line = $(".done-line", card);
      if (line) line.textContent = words;
    }
    // Go-live moves to the Move page, where the switch is shown live. Approved on the card's own
    // page, a console redirect (the new mission, the WebAgent just added) is where it went;
    // approved from the lane, stay here.
    const own = /\/decisions\/[^/]+$/.test(location.pathname) && redirect && /^\/console\//.test(redirect);
    const go = redirect && (/\/console\/move$/.test(redirect) || own) ? redirect : location.pathname + location.search;
    setTimeout(() => location.assign(go), 900);
  }

  async function approve(byCode) {
    if (!sheet) return;
    say("");
    let body;
    if (byCode) {
      const v = (code.value || "").trim();
      if (!/^\d{6}$/.test(v)) { code.classList.add("bad"); code.placeholder = "Enter the 6-digit code"; code.focus(); return; }
      body = sheet.body_format === "proof" ? { totp: v } : { method: "totp", code: v, fingerprint: sheet.fingerprint };
    } else {
      if (!sheet.passkey_options) { stepup(true); return; }
      busy(true);
      try {
        const cred = await navigator.credentials.get(requestOptions(JSON.parse(sheet.passkey_options)));
        body = sheet.body_format === "proof" ? { passkey: assertion(cred) } : { method: "passkey", credential: assertion(cred) };
      } catch (e) {
        busy(false);
        say(e && e.name === "NotAllowedError" ? "The passkey prompt was closed or timed out. Nothing was approved." : (e && e.message) || "That didn't work. Try again.");
        return;
      }
    }
    const edited = sheet.edits && Object.keys(sheet.edits).length > 0;
    if (edited) body.edits = sheet.edits;
    if (sheet.grant) body.grant = sheet.grant;
    busy(true);
    try {
      const done = await json(sheet.approve_url, { method: "POST", body: JSON.stringify(body) });
      finish(done.redirect, (edited ? "Your version approved" : "Approved") + (byCode ? " with an authenticator code" : " with your passkey") + (sheet.grant ? " · allowed again within the limit, and it tells you each time" : " · it runs once") + " · receipt written");
    } catch (e) {
      busy(false);
      say(e.message);
      // A used ceremony can't be retried; fetch a fresh one for the next try.
      if (!byCode && sheet && card) {
        const opener = card.querySelector("[data-sheet]");
        if (refetch) { try { sheet = await refetch(); } catch (_) { /* keep the message shown */ } }
        else if (opener) { try { sheet = await json(opener.dataset.sheet); } catch (_) { /* keep the message shown */ } }
      }
    }
  }

  document.addEventListener("click", e => {
    // Edit and Decline open their box on the card; Cancel closes it. Neither decides anything.
    const t = e.target.closest("[data-toggle]");
    if (t) {
      const target = document.getElementById(t.dataset.toggle);
      if (target) {
        e.preventDefault();
        const open = !target.classList.contains("open");
        target.classList.toggle("open", open);
        document.querySelectorAll(`[aria-controls="${target.id}"]`).forEach(b => b.setAttribute("aria-expanded", String(open)));
        if (open) { const first = target.querySelector("input:not([type=hidden]), textarea"); if (first) first.focus(); }
        return;
      }
    }
    const ed = e.target.closest("[data-edit-approve]");
    if (ed) {
      e.preventDefault();
      const f = ed.closest("[data-edit-form]");
      const edits = {};
      new FormData(f).forEach((v, k) => { if (k.startsWith("edit.")) edits[k.slice(5)] = String(v); });
      openSheet(f.dataset.editSheet, "POST", f.closest("[data-decision]"), { edits });
      return;
    }
    const gr = e.target.closest("[data-grant-approve]");
    if (gr) {
      e.preventDefault();
      const f = gr.closest("[data-grant-form]");
      const picked = f.querySelector("input[name=grant]:checked");
      openSheet(f.dataset.grantSheet, "POST", f.closest("[data-decision]"), { grant: picked ? picked.value : "kind" });
      return;
    }
    const s = e.target.closest("[data-sheet]");
    if (s) { e.preventDefault(); openSheet(s.dataset.sheet, "GET", s.closest("[data-decision]")); return; }
    const u = e.target.closest("[data-unfreeze]");
    if (u) { e.preventDefault(); openSheet(u.dataset.unfreeze, "POST", null); return; }
    if (e.target.closest("[data-stepup]")) { stepup(true); return; }
    if (e.target.closest("[data-stepup-back]")) { stepup(false); say(""); return; }
    const d = e.target.closest("[data-do-sign]");
    if (d && box.contains(d)) { approve(d.dataset.method === "totp"); }
  });

  // Decline without leaving the page: the decline route answers JSON with where to go.
  document.addEventListener("submit", async e => {
    const f = e.target.closest("form[data-decline]");
    if (!f) return;
    e.preventDefault();
    const c = f.closest("[data-decision]");
    // A reason and "Make this a rule" travel with the decline; a plain Decline sends nothing.
    const fields = new FormData(f);
    const reason = String(fields.get("reason") || "").trim();
    const rule = fields.get("rule") === "on";
    try {
      await json(f.action, { method: "POST", body: JSON.stringify({ reason, rule }) });
      if (c) { c.classList.add("done"); const l = c.querySelector(".done-line"); if (l) l.textContent = rule && reason ? `Declined · new rule for this WebAgent: "${reason}" · receipt written` : "Declined · nothing changed · receipt written"; }
      setTimeout(() => location.reload(), 900);
    } catch (err2) {
      if (c) { const l = c.querySelector(".done-line"); if (l) { l.textContent = err2.message; c.classList.add("done"); } }
    }
  });
}
