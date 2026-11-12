/* Agentrix · check a receipt's signature in the browser, without trusting the page (R10.3, R12.5).
   Signed bytes = RFC 8785 (JCS) canonical JSON of the receipt without its `signature` field (design §4.4).
   Key: Ed25519 from /.well-known/agentrix/keys. Uses WebCrypto Ed25519 where the browser has it. */
(() => {
  const box = document.getElementById("verify");
  const go = document.getElementById("verify-go");
  const out = document.getElementById("verify-out");
  if (!box || !go) return;
  const dec = s => { s = String(s).replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; const b = atob(s); return Uint8Array.from(b, c => c.charCodeAt(0)); };
  const hex = s => Uint8Array.from(String(s).match(/../g).map(h => parseInt(h, 16)));
  const bytes = s => /^[0-9a-f]+$/i.test(s) && s.length % 2 === 0 && (s.length === 64 || s.length === 128) ? hex(s) : dec(s);

  // RFC 8785: object keys sorted by UTF-16 code units; numbers and strings as ECMAScript JSON.stringify writes them.
  function jcs(v) {
    if (v === null || typeof v !== "object") return JSON.stringify(v);
    if (Array.isArray(v)) return `[${v.map(jcs).join(",")}]`;
    return `{${Object.keys(v).filter(k => v[k] !== undefined).sort().map(k => `${JSON.stringify(k)}:${jcs(v[k])}`).join(",")}}`;
  }
  const say = (cls, text) => { out.className = `small ${cls}`; out.textContent = text; };

  go.addEventListener("click", async () => {
    go.disabled = true; say("", "Fetching the signed receipt and our public key…");
    try {
      const [rr, kr] = await Promise.all([
        fetch(box.dataset.receipt, { headers: { Accept: "application/json" }, credentials: "same-origin" }),
        fetch(box.dataset.keys, { headers: { Accept: "application/json" } }),
      ]);
      if (!rr.ok || !kr.ok) throw new Error("Couldn't fetch the receipt or the key.");
      const receipt = await rr.json(), keys = await kr.json();
      // agentrix-types: `signature` is unpadded base64url; `issuer` names the key as
      // "ed25519:<16 hex>"; the key set is {keys: [{key_id, alg: "Ed25519", public_key}]}.
      const sigValue = receipt.signature;
      const kid = receipt.issuer;
      const list = Array.isArray(keys) ? keys : keys.keys || [];
      const key = list.find(k => k.key_id === kid);
      if (!key) throw new Error(`The key that signed this receipt (${kid}) isn't in our published key set.`);
      const pub = key.public_key;
      if (typeof sigValue !== "string" || !pub) throw new Error("The receipt or the key file isn't in the expected format.");
      const unsigned = { ...receipt }; delete unsigned.signature;
      const data = new TextEncoder().encode(jcs(unsigned));
      let k;
      try { k = await crypto.subtle.importKey("raw", bytes(pub), { name: "Ed25519" }, false, ["verify"]); }
      catch (_) { say("waitline", "This browser can't check Ed25519 signatures yet. The server's check above still stands; you can verify offline with the published key."); go.disabled = false; return; }
      const ok = await crypto.subtle.verify({ name: "Ed25519" }, k, bytes(sigValue), data);
      say(ok ? "okline" : "error", ok ? `Signature checks out in this browser, against key ${key.key_id}.` : "The signature does NOT match. Don't rely on this receipt, and tell us.");
    } catch (e) { say("error", e.message || "The check couldn't run."); }
    go.disabled = false;
  });
})();
