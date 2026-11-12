# ADR-014: One decision contract on every surface; consequential decisions need a passkey or an MFA step-up

Status: Proposed, 7 Nov 2026. Amended 7 Nov 2026: a passkey or, without one, the MFA step-up (d-auth-passkey-first, [ADR-006](006-passkey-signed-approvals.md)).

## Context

Owners meet their WebAgents on the console PWA, WhatsApp, email and voice; teams and agencies may use Slack or Telegram. `adk-surface` renders the kernel's sessions, decisions and activity for every surface through one `SurfaceEngine`, and one rule table (`rules::allowed_answers`) decides which answers each surface may give. adk-kernel's `ChannelGateway` never lets chat decide money.

## Decision

- Every owner surface is an `adk-surface` view over the site's kernel. Surfaces hold no state (K-I17).
- Consequential decisions (money, high risk, publishing a new page, DNS, go-live) are approved only on a console surface: the PWA, or the confirmation page a WhatsApp tap opens. Approval is a passkey, as a WebAuthn assertion over the effect fingerprint, or without one an MFA step-up bound to that fingerprint ([ADR-006](006-passkey-signed-approvals.md)). WhatsApp and other chats may Allow once or Deny at low and medium risk and link to the console for the rest. Voice may ask, deny and stop all. Email carries briefs and reviews and never approves.
- Alarms route by urgency through `AlarmRouter` and the owner's quiet hours, in the owner's time zone.
- Visitors and AI agents are surfaces too, served by the public desks through the edge, with AI disclosure on every reply.

## Consequences

A stolen chat session or forwarded email cannot move money: it has neither the passkey nor the authenticator. The owner gets the same card everywhere, with the same risk and the same effect preview. `adk-surface`'s words are English only, so localisation (U5) is needed for the Kenyan pilot. See [05-webagents.md §7](../05-webagents.md#7-surfaces).
