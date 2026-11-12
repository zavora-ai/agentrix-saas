# Badges: evidence

## Gate "a did.json resolves locally" (met Tue 10 Nov 2026, 20:49 EAT)

Command: `scripts/wcargo run -p saas-badge --example badge_local`, with `--serve` to keep the hosts up for `agentrix-badge verify`.

**Setup.** Agentrix (`agentrix.localhost:18182`, the operator) and Kilele Bakery (`kilele.localhost:18182`, a sample site) are served by `Host` on one listener. They use the same keys the sandbox settlement doors sign with.

**What it showed:**
- **The DID document.** `did:web:kilele.localhost%3A18182:agents:cashier` resolves from `http://kilele.localhost:18182/agents/cashier/did.json`:
  - its key is the Kilele door's `agent:payables` key (thumbprint `nBNUFwxQgZGlcAMRYg1ICIIepvf6sCwATnKWqbfQHxw`)
  - `alsoKnownAs` lists `urn:adk:agent:kilele-bakery/receivables` and `urn:adk:agent:kilele-bakery/payables`
- **The PayPal binding.** It was built from PayPal sandbox's own read-back (`GET /v3/vault/payment-tokens/5an08729le1870910`, Agentrix's app) of the wallet Kilele saved: payment method …0910, response digest `sha256:172df90f4681e90cb18ebfbc7ba4ac0237345db7e994beb2acce82b8805a7ccf`.
  - It was approved by the owner key file (`local-owner-key`; no phone here; a passkey on the node).
  - It was signed by `did:web:kilele.localhost%3A18182#owner` and written to the (in-memory) audit log.
- **Verified from the DID alone:**
  - identifier
  - badge signature (the site's owner key)
  - badge
  - agent key (the DID document, the badge and the key directory agree)
  - names
  - status
  - voucher (signed by `did:web:agentrix.localhost%3A18182`, after checking the binding)
  - PayPal link (signature, cited digest, this agent allowed, approval for exactly these terms)
  - signed A2A card

  Result: **verified**.

## Re-run on saas-core's contract types and face's ceremony (Tue 10 Nov, 21:04 EAT)

The same command, after the switch to `saas_core::identity` and the `link-paypal` ceremony. The binding `bind_01a1c6a25cef742d9eeb31fcda1fd254` was built from PayPal's answer to `GET /v3/vault/payment-tokens/5an08729le1870910` (`sha256:172df90f…7ccf`) and sealed at audit seq 1.

**How the stand-in approval is handled.** The ceremony was answered by the owner key file, shaped like a passkey assertion and checked the same way offline (challenge, client data, RP ID hash, signature).
- **The labels:** the record says `local-owner-key`, never `passkey`.
- **The default verifier refuses it:** it reports "approved by the owner (local-owner-key), not a passkey".
- **Where it's accepted:** only the local run's verifier accepts it, and only by name.

All ten checks passed: **verified**.

## Gate "a badge verifies on the real hosts" (Wed 14:00)

Not yet. It waits on the node mounting `saas_badge::site::router` per site host (slice J) and on the deploy.
