// saas-web · Link PayPal: reads the draft binding saas-badge's desk built from PayPal's own
// answer, shows what the passkey will sign, gets the owner's assertion over the fingerprint and
// sends it to the desk to seal the binding.

const card = document.querySelector("[data-paypal-link]");

function b64url(buf) {
  return btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function fromB64url(s) {
  const b = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(b, (c) => c.charCodeAt(0));
}
function el(tag, text) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  return e;
}
function last4(v) {
  return v ? `…${String(v).slice(-4)}` : "";
}
/** `did:web:host:agents:cashier` → `cashier`. */
function role(did) {
  const i = String(did).lastIndexOf(":agents:");
  return i >= 0 ? String(did).slice(i + 8) : String(did);
}

const STATE = {
  awaiting_paypal: "PayPal hasn't sent you back yet. Finish on PayPal, then come back here.",
  awaiting_passkey: "Check it, then approve with your passkey.",
  bound: "Linked. The binding is sealed with the site's owner key and shown in every agent's badge.",
  revoked: "This link was revoked. Linking again starts a new binding.",
};

// The draft is saas-core's PayPalBinding as PayPal's Vault answered it (saas-badge's
// BindingDraft): nothing on this page is typed in.
async function load() {
  const status = card.querySelector("[data-link-status]");
  const res = await fetch(`${card.dataset.api}/${encodeURIComponent(card.dataset.binding)}`, { headers: { accept: "application/json" } });
  const d = await res.json().catch(() => ({}));
  if (!res.ok) {
    status.textContent =
      d.message || "That link isn't waiting for you: it was used, it expired, or PayPal linking isn't connected on this node.";
    return null;
  }
  const draft = d.draft || {};
  const paypal = draft.paypal || {};
  const rows = [
    ["Site", draft.site],
    ["PayPal", draft.environment === "live" ? "live" : draft.environment ? `${draft.environment} (no real money)` : ""],
    ["PayPal account", paypal.payer_email_masked],
    ["Payer id", paypal.payer_id],
    ["Saved payment (vault)", last4(paypal.payment_token)],
    ["Customer id", paypal.customer_id],
    ["Agents allowed", (draft.agents || []).map(role).join(", ")],
    ["PayPal's answer", paypal.response_digest],
  ].filter(([, v]) => v);
  const dl = card.querySelector("[data-link-draft]");
  dl.replaceChildren(...rows.flatMap(([k, v]) => [el("dt", k), el("dd", v)]));
  dl.hidden = rows.length === 0;
  if (d.fingerprint) {
    const fp = String(d.fingerprint);
    card.querySelector("[data-link-fp-value]").textContent = fp.startsWith("sha256:") ? fp : `sha256:${fp}`;
    card.querySelector("[data-link-fp]").hidden = false;
  }
  if (d.state === "bound" && d.signed) {
    const pre = card.querySelector("[data-link-signed]");
    pre.textContent = JSON.stringify(d.signed, null, 2);
    pre.hidden = false;
  }
  if (d.state !== "awaiting_passkey") {
    status.textContent = STATE[d.state] || `This link is ${d.state}.`;
    return null;
  }
  status.textContent = d.passkey_options ? STATE.awaiting_passkey : "Set up your passkey first, on the Passkey page, then open this link again.";
  return d;
}

async function approve(button, d) {
  const status = card.querySelector("[data-link-status]");
  button.disabled = true;
  try {
    const pk = d.passkey_options.publicKey || d.passkey_options;
    pk.challenge = fromB64url(pk.challenge);
    for (const c of pk.allowCredentials || []) c.id = fromB64url(c.id);
    const cred = await navigator.credentials.get({ publicKey: pk });
    const credential = {
      id: cred.id,
      rawId: b64url(cred.rawId),
      type: cred.type,
      response: {
        authenticatorData: b64url(cred.response.authenticatorData),
        clientDataJSON: b64url(cred.response.clientDataJSON),
        signature: b64url(cred.response.signature),
        userHandle: cred.response.userHandle ? b64url(cred.response.userHandle) : null,
      },
      extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
    };
    const res = await fetch(`${card.dataset.api}/${encodeURIComponent(card.dataset.binding)}/approve`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({ method: "passkey", credential }),
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(body.message || "Not linked.");
    status.textContent = STATE.bound;
    const pre = card.querySelector("[data-link-signed]");
    pre.textContent = JSON.stringify(body.binding, null, 2);
    pre.hidden = false;
    button.remove();
  } catch (e) {
    status.textContent = e.message || "Not linked.";
    button.disabled = false;
  }
}

if (card && card.dataset.binding) {
  load().then((d) => {
    const button = card.querySelector("[data-link-approve]");
    if (d && d.passkey_options && button) {
      button.hidden = false;
      button.addEventListener("click", () => approve(button, d));
    }
  });
}
