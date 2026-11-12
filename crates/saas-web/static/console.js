// saas-web · the owner console's approvals. kit.js opens the approval sheet; this binds its
// passkey button. The ceremony itself is slice C's: when the node serves a WebAuthn challenge
// for the decision (GET <action>/challenge answers {publicKey}), the browser signs it with the
// owner's passkey and the assertion is posted with the approval. Without one (the demo node),
// the approval is posted without proof and the node decides whether that is enough.

function b64url(buf) {
  return btoa(String.fromCharCode(...new Uint8Array(buf))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function fromB64url(s) {
  const b = atob(s.replace(/-/g, "+").replace(/_/g, "/"));
  return Uint8Array.from(b, (c) => c.charCodeAt(0));
}

async function assertion(action) {
  if (!window.PublicKeyCredential) return null;
  const res = await fetch(`${action}/challenge`, { headers: { accept: "application/json" } });
  if (!res.ok) return null;
  const { publicKey } = await res.json();
  return sign(publicKey);
}

/** The owner's passkey over `publicKey` (WebAuthn options), as webauthn-rs's JSON. */
async function sign(publicKey) {
  publicKey.challenge = fromB64url(publicKey.challenge);
  for (const c of publicKey.allowCredentials || []) c.id = fromB64url(c.id);
  const cred = await navigator.credentials.get({ publicKey });
  return {
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
}

// Setting up the owner's passkey for this console (RP ID: this host).
async function setUp(button) {
  const status = document.querySelector("[data-passkey-status]");
  button.disabled = true;
  try {
    const res = await fetch(button.dataset.optionsUrl || "/console/passkey/options", { method: "POST", headers: { accept: "application/json" } });
    const body = await res.json();
    if (!res.ok) throw new Error(body.message || "The console couldn't start a passkey.");
    const pk = body.publicKey;
    pk.challenge = fromB64url(pk.challenge);
    pk.user.id = fromB64url(pk.user.id);
    for (const c of pk.excludeCredentials || []) c.id = fromB64url(c.id);
    const cred = await navigator.credentials.create({ publicKey: pk });
    const done = await fetch(button.dataset.registerUrl || "/console/passkey", {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        id: cred.id,
        rawId: b64url(cred.rawId),
        type: cred.type,
        response: {
          attestationObject: b64url(cred.response.attestationObject),
          clientDataJSON: b64url(cred.response.clientDataJSON),
          transports: cred.response.getTransports ? cred.response.getTransports() : undefined,
        },
        extensions: cred.getClientExtensionResults ? cred.getClientExtensionResults() : {},
      }),
    });
    const answer = await done.json().catch(() => ({}));
    if (!done.ok) throw new Error(answer.message || "The passkey wasn't saved.");
    if (status) status.textContent = answer.message || "Passkey set up.";
    setTimeout(() => location.reload(), 900);
  } catch (e) {
    if (status) status.textContent = e.message || "No passkey was set up.";
    button.disabled = false;
  }
}

for (const b of document.querySelectorAll("[data-passkey-setup]")) {
  b.addEventListener("click", () => setUp(b));
}

for (const button of document.querySelectorAll("[data-passkey]")) {
  button.addEventListener("click", async () => {
    const action = button.dataset.action;
    const sheet = button.closest(".overlay");
    const status = sheet && sheet.querySelector(".ok");
    button.disabled = true;
    try {
      let proof = null;
      try {
        proof = await assertion(action);
      } catch {
        proof = null;
      }
      const res = await fetch(action, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ approve: true, proof }),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.message || "The approval was refused.");
      if (status) status.textContent = body.message || "Approved.";
      sheet && sheet.classList.add("signed");
      setTimeout(() => location.reload(), 900);
    } catch (e) {
      if (status) status.textContent = e.message;
      button.disabled = false;
    }
  });
}

// Switching a revoked agent back on: only with the owner's passkey, over that revocation.
for (const button of document.querySelectorAll("[data-restore]")) {
  button.addEventListener("click", async () => {
    const action = button.dataset.restore;
    const status = button.closest("li")?.querySelector("[data-restore-status]");
    button.disabled = true;
    try {
      const res = await fetch(`${action}/challenge`, { headers: { accept: "application/json" } });
      const options = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(options.message || "It can't be switched back on right now.");
      const proof = await sign(options.publicKey);
      const done = await fetch(action, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ proof }),
      });
      const body = await done.json().catch(() => ({}));
      if (!done.ok) throw new Error(body.message || "Not switched on.");
      if (status) status.textContent = body.message || "Switched back on.";
      setTimeout(() => location.reload(), 900);
    } catch (e) {
      if (status) status.textContent = e.message || "Not switched on.";
      button.disabled = false;
    }
  });
}
