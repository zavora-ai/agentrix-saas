# agentrix-messages

Every message the platform sends to owners and customers (W9): the catalogue, the outbox, the
Messages app in the owner console, notification settings, and delivery.

## Sending from another crate

```rust
let messages = agentrix_messages::from_context(ctx);
messages.send_to_site_owner(site_id, "owner.new_order", context, "Njeri paid for order KB-1042", Some(key)).await?;
messages.send(agentrix_messages::Outgoing { message: "customer.order_received".into(), .. }).await?;
```

Each message's values are in its example beside the template: `templates/<audience>/<name>.json`.
A missing value is an error, never a blank. Every message is recorded in the outbox first.

## Node wiring

| Where | Line |
|---|---|
| `MIGRATIONS` | `agentrix_messages::migrate` (tables `message_outbox`, `message_preferences`) |
| `MODULES` | `agentrix_messages::mount` (`/console/messages/*`, the bounce webhook, the quiet-hours release task) |
| The shell | `agentrix_shell::mount_with(ctx, vec![agentrix_messages::console_app(ctx)], …)` |
| Identity | `agentrix_identity::mount_with_mailer(ctx, agentrix_messages::identity_mailer(ctx))`, and `.with_mailer(…)` on customer accounts |

## Switching email on

Until this is done, every message stays in the outbox as "Not sent" and nothing leaves.
James set up Resend for `agentrix.website` (DKIM, the `send` and `rsend` records and DMARC
`p=none` on Vultr DNS) on Sun 8 Nov.

1. Give node the send-only key as `RESEND_API_KEY` in its environment, never a committed
   file: `deploy/deploy.sh <commit> --email` copies it (with `RESEND_WEBHOOK_SECRET` and
   `AGENTRIX_EMAIL_FROM` when set) from `./.env` into control's root-only
   `/etc/agentrix/secrets/node.env`, which node's unit reads. Without that flag the file isn't
   written and email stays off (found 8 Nov: the newsletter sign-up said "Email isn't switched
   on yet" on the live site). Optional: `AGENTRIX_EMAIL_FROM` (default
   `hello@agentrix.website`), `AGENTRIX_EMAIL_REPLY_TO` (an email address) and
   `RESEND_WEBHOOK_SECRET`. A JSON file named by `AGENTRIX_EMAIL_FILE` works too (see
   `src/email.rs`).
2. In Resend, a webhook for `email.bounced` and `email.complained` pointing at
   `https://agentrix.website/messages/email/webhook`, with its secret in `RESEND_WEBHOOK_SECRET`.
3. Restart node. The Messages page stops saying "Email isn't switched on yet", and new messages
   show Sent with Resend's id. Addresses at `example.com` are never delivered. At start, node
   sends what visitors asked agentrix.website for in the last week while email was off (the
   newsletter's confirm link, a kept report; `Messages::send_waiting`), once each. Anything
   asked for again after a try that never left (off, or refused by Resend) goes again then.
4. Move DMARC to `p=quarantine` once Resend's reports look clean.

## Checking it

```sh
scripts/wcargo test -p agentrix-messages
scripts/wcargo run -p agentrix-messages --example dump -- <dir>      # every message as files
scripts/wcargo run -p agentrix-messages --example console -- 18902   # the app in the shell
scripts/copy-lint crates/agentrix-messages/templates
```

A real send through Resend, one email per way the platform sends (codes, invitations, orders,
the digest; then the newsletter, a kept report and an issue from agentrix.website's own paths).
Resend's test inbox `delivered@resend.dev` takes them and delivers nowhere; each counts against
the daily limit (100 on the free plan). Pass the key without printing it:

```sh
export AGENTRIX_RESEND_CHECK_TO=delivered@resend.dev
RESEND_API_KEY="$(grep -E '^RESEND_API_KEY=' .env | cut -d= -f2-)" \
  scripts/wcargo test -p agentrix-messages --test resend_live -- --ignored --nocapture
RESEND_API_KEY="$(grep -E '^RESEND_API_KEY=' .env | cut -d= -f2-)" \
  scripts/wcargo test -p agentrix-home --test resend_live -- --ignored --nocapture
```
