// saas-web · one owner moment (link PayPal, sign the mandate, step up): asks the node for a
// challenge bound to what is signed, SHA-256(fingerprint ‖ nonce), gets the owner's passkey
// assertion, and posts it to be checked and sealed.

const card = document.querySelector("[data-ceremony]");

function b64url(buf) {
  return btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function fromB64url(s) {
  const b = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(b, (c) => c.charCodeAt(0));
}

async function sign(button) {
  const status = card.querySelector("[data-ceremony-status]");
  const record = card.querySelector("[data-ceremony-record]");
  button.disabled = true;
  status.textContent = "Asking for your passkey…";
  try {
    const res = await fetch(card.dataset.challengeUrl, { method: "POST", headers: { accept: "application/json" } });
    const body = await res.json();
    if (!res.ok) throw new Error(body.message || "The console couldn't start the passkey.");
    const pk = body.publicKey;
    pk.challenge = fromB64url(pk.challenge);
    for (const c of pk.allowCredentials || []) c.id = fromB64url(c.id);
    const cred = await navigator.credentials.get({ publicKey: pk });
    const done = await fetch(card.dataset.signUrl, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
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
      }),
    });
    const answer = await done.json().catch(() => ({}));
    if (!done.ok) throw new Error(answer.message || "It wasn't sealed.");
    status.textContent = answer.message || "Signed and sealed.";
    card.classList.add("sealed");
    if (answer.record) {
      record.textContent = JSON.stringify(answer.record, null, 2);
      record.hidden = false;
    }
    button.remove();
  } catch (e) {
    status.textContent = e.message || "Nothing was signed.";
    button.disabled = false;
  }
}

if (card) {
  const button = card.querySelector("[data-ceremony-sign]");
  if (button) button.addEventListener("click", () => sign(button));
}
