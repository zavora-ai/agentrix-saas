# ADR-006: Approvals are bound to effect fingerprints: a passkey signature, or an MFA step-up

Status: Proposed, 7 Nov 2026. Amended 7 Nov 2026 for James's decision that passkeys come first, with authenticator MFA as the fallback (d-auth-passkey-first; [02 §8](../02-architecture.md)).

## Context

Cookie-session approvals can be replayed, forged by a stolen session or applied to the wrong effect, and an approval decided once did not guarantee the effect ran once.

Not every owner, agency person or operator will have a passkey on day one. James decided that passkeys are primary on every surface. Without one, people sign in with email and password, or with Google, LinkedIn or GitHub, always with authenticator-app MFA. There are no magic links.

## Decision

Each effect request is canonicalised to the fingerprint `H(site, webagent, tool, canonical_args, amount, expiry)`. Every approval is bound to that fingerprint, by one of two methods:

| Method | What the approver does | What the Effects Gateway verifies | What it proves |
|---|---|---|---|
| **Passkey (primary)** | A WebAuthn assertion whose challenge is the fingerprint | The assertion, against the approver's registered public key | The approver's own signature. Anyone can check it offline |
| **MFA step-up (without a passkey)** | Reads the exact effect on the card and enters a fresh authenticator code | `agentrix-control`'s signed attestation that the code was valid, unused and entered for this fingerprint | The approver held their authenticator at that moment, on the platform's word |

- **Either way,** the Effects Gateway verifies the approval against the fingerprint, uses the fingerprint as the idempotency key, runs the effect exactly once, and records the method in the receipt.
- **A step-up code is single-use.** Control refuses a code already used in its time window. The attestation names the fingerprint, the approver, the time and the method. The Effects Gateway can verify that attestation key but cannot sign with it.
- **WhatsApp and other chat taps on a consequential card** open the passkey confirmation, or the step-up without one. They never approve silently ([ADR-014](014-one-decision-contract.md)).
- **The same two methods sign everything else that needs a person's authority:**
  - standing rules and promotions ([ADR-012](012-autonomy-per-job.md), U6);
  - plans;
  - support grants ([ADR-023](023-support-grants-and-break-glass.md));
  - tenant-path grants ([ADR-018](018-tenant-path.md));
  - infrastructure changes ([ADR-021](021-infrastructure-changes-are-effects.md)).

## Consequences

- An approval cannot be moved to a different effect or replayed, whichever method made it.
- **The two methods are not equally strong, and receipts say which was used.**
  - A passkey approval is the owner's own signature, so a compromised control plane cannot forge one.
  - A step-up approval rests on control's attestation, so a control compromise could forge one. An authenticator code can also be phished in real time, which a passkey cannot.
  - Control counts step-up attestations per site and alarms on unusual rates.
  - The console invites people without a passkey to add one, at sign-in and on every step-up.
- Magic links are gone everywhere. Viewing needs a signed-in session like everything else.
