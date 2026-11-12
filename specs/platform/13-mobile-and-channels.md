# 13 · Mobile use and channels (W13)

Owner: W13, "Mobile and channels". Started Sun 8 Nov 2026, 10:30 EAT. Crate: `agentrix-channels`
(new); Flutter workspace: `apps/`. Migrations 1300–1399 through the crate's own migrator.

**Sources.** `concept/console/phone.html` (the phone, the WhatsApp brief and the surface table),
`concept/console/index.html`, `concept/journeys/customer-*.html`, docs/05 §7 (surfaces and the
channels row: Telegram and Slack "as WhatsApp"), docs/07 §3.8, §4.1 to §4.4 and principle 9
("Hands-free never means sign-free"), ADR-014 (one decision contract), ADR-006 (passkey-signed
approvals), 01b C34–C39 and §4.3, and the crates this builds on: `agentrix-messages` (W9),
`agentrix-decisions` and `agentrix-shell` (W5), `agentrix-identity` and `agentrix-effects` (W6),
`agentrix-media` (W11), `agentrix-kernel` (W1), `agentrix-samples` (W2), `agentrix-host`.

## 1. What ships when

| | Before the freeze (Sun 8 Nov, 17:00 EAT) | After the freeze (Horizon 2) |
|---|---|---|
| **Telegram** | Kilele's bot, live with its WebAgents on the local node: the owner links in Settings → Channels; the morning brief, "a decision is waiting" and questions the desks pass on; decline, pause and STOP in chat; approve only through `/d/{card}` in the console; customers ask the Concierge and the Orders desk the same way Ask answers on the site | One bot per site from the owner's own BotFather token, pasted in Settings and held by the Effects Gateway; webhooks on the live host |
| **WhatsApp, email** | The same decisions through W9's catalogue and the one rule table (§3); WhatsApp drawn and ready behind W9's setting; email links only | WhatsApp Cloud API through the Effects Gateway once a number is verified |
| **Phone (web)** | W5's `/console` at phone width (§6) | Same |
| **Flutter apps** | This spec | Customer app (three flavours), owner app, push (§7 to §10) |

## 2. Users and surfaces

| User | Surface | What they do there |
|---|---|---|
| **Owner** (Wanjiru) | Console PWA, phone width (W5) | Everything, with a passkey or an authenticator code for anything consequential |
| | Owner app (Flutter) | The same as the phone PWA, natively: Needs you, decisions, brief, WebAgents, voice, Freeze |
| | Telegram, WhatsApp | The brief, alerts, questions passed on; ask, decline, pause, STOP; approving opens the console |
| | Email | The brief and the weekly review, with links. Decides nothing |
| **Team member** (staff) | The owner app with a role (§8) | What their role allows; approvals only where W6's roles let them |
| **Customer** | Site (W2), Ask | Browse, ask, bag, checkout, orders, account |
| | Customer app (Flutter, per site) | The same, natively, in the site's own brand |
| | Telegram | Ask the site's WebAgents: the Concierge, the desks, the Orders desk for order status |

## 3. One rule table for every surface

ADR-014 and docs/05 §7 put the powers of every surface in one table (`rules::allowed_answers` in
`adk-surface`). The workspace doesn't link `adk-surface` yet, so `agentrix_channels::rules` holds
the same table and every channel asks it before offering a button. W5's `decide_spoken` enforces
the same limits on the server, so a surface that offers too much is still refused.

| Surface | Approve | Allow once | Decline | Pause one WebAgent | Stop all (freeze) | Unfreeze |
|---|---|---|---|---|---|---|
| Console PWA, owner app | Passkey, or authenticator code, bound to the card's fingerprint | Low-risk one-tap cards | Yes | Yes | Yes | Passkey or code |
| Telegram, WhatsApp, other chats | **Never.** "Approve" is a link to `/d/{card}` | Designed for low-risk one-tap cards (phone.html); **off** for the submission (James's brief: chat approves nothing) | Yes, any card | Yes | Yes (STOP) | Never |
| Voice (W11) | Never; opens the sheet on screen | Low-risk one-tap, after a 3 s cancel window | Yes | No | Yes, at once | Never |
| Email | Never | Never | Never; links only | Never | Never | Never |
| Gesture (W5) | Never | Never | Never | Yes | Yes, after a 5 s countdown | Never |
| Notification (lock screen, push) | Never | Never | Never | Never | Never | Never |

A chat decline is a real decision: W5's `decide_spoken(site, account, card, allow: false, "chat",
heard)` writes `decision.declined` with `how: "chat"` and the words the owner sent, and never counts
toward promotion. A STOP is W5's Freeze (`shell_freezes`, `how: "chat"`); only the console unfreezes.

## 4. Telegram

### 4.1 The bot and its token

- A Telegram Bot API bot, made with @BotFather. For the submission, one bot for Kilele Bakery's
  sample (`kilele-bakery-sample`); James makes it and saves its token himself (board ask
  `a-w13-telegram-bot`).
- **The token never appears** in the repository, logs, the outbox, receipts, error messages or the
  console. Node reads it from a JSON file only node's user can read, named by
  `AGENTRIX_TELEGRAM_FILE` (`{"bots": [{"site": "kilele-bakery-sample", "token": "…"}]}`), the way
  it reads the PayPal and email credentials. A file readable by others is refused with a log line
  that names the path, never the content. Errors from the Bot API are logged with the method name
  only, because Telegram's URLs carry the token.
- `getMe` at start gives the bot's username for deep links; `setMyCommands` sets the menu.
- **Locally** (the VMs are off, so there's no public HTTPS) node long-polls `getUpdates` with a
  stored offset, so each update is handled once even across restarts. Only one process may poll a
  bot: the local node that has the file does; a second gets Telegram's 409 and backs off.
- **On deploy** the same handler serves `POST /channels/telegram/{bot}/webhook` on the platform
  host, checks Telegram's `X-Telegram-Bot-Api-Secret-Token` header against a secret derived from
  the token, and answers 200 at once. `AGENTRIX_TELEGRAM_MODE=webhook` switches polling off and
  calls `setWebhook`; that needs a `deploy/` line from the control-plane session.
- Horizon 2: each owner pastes their own bot token in Settings → Channels; the Effects Gateway
  holds it, and the webhook path is per bot.

### 4.2 Linking the owner

1. In the console, Settings → **Channels** (W13's console app, `/console/{site}/apps/channels`),
   the signed-in owner taps **Link Telegram**.
2. Node makes a one-time code (128 bits, stored only as its SHA-256), bound to the account and the
   site, valid 15 minutes, single use, and shows `https://t.me/<bot>?start=<code>` as a button and
   a QR code. A new code replaces any unused one.
3. The owner opens it; Telegram sends `/start <code>`. The bot checks the code (unused, unexpired,
   private chat), links the chat to the account and site, marks the code used, writes a
   `channel.changed` receipt (`{action: "linked", channel: "telegram", by}`; no chat id or name in
   the receipt), and answers with what the chat will get and what it can do.
4. **Unlinking** is one tap in Settings (**Unlink**) or `/stop` in the chat. Both end the link,
   write `channel.changed` (`unlinked`), and confirm in the chat. A chat that blocks the bot
   (Telegram answers 403) is unlinked the same way.
5. One link per (site, owner, channel); linking again from a new chat moves it. A code that is
   wrong, used or expired gets one plain line ("That link has expired. Make a new one in Settings
   → Channels.") and no hint about whether it ever existed.

### 4.3 What the owner gets

Every one of these is a W9 catalogue message, recorded in W9's outbox before it goes (§4.6):

| Message | When | Buttons |
|---|---|---|
| `owner.morning_brief` | Right after linking (the latest brief), then each morning when quiet hours end | Open the console |
| `owner.decision_waiting` | A new waiting card on the site (W5's v2 cards and the first release's held orders), once per card | **Open in the console** (`/d/{card}`), **Decline** |
| `owner.question_passed` | A desk passes a visitor's question it couldn't answer | Answer it (console) |
| `owner.frozen` | Someone freezes the site, from any surface | Open the console |

Quiet hours hold P2 and P3 messages until they end (W9's rule); P1 rings through.

### 4.4 What the owner can do in chat

| They send | What happens |
|---|---|
| Tap **Decline** on a card, or "decline" / "no" right after one | W5's `decide_spoken(…, false, "chat", heard)`; the message is edited to "Declined. Nothing changed." with the receipt's short id |
| Tap **Open in the console** | `/d/{card}`: sign in if needed, then the card with its exact effect, approved with a passkey or authenticator code |
| "approve", "yes", "allow" | "Money and anything signed needs your passkey or an authenticator code, so it opens in the console:" and the `/d/{card}` link. Nothing is approved |
| **STOP**, "stop everything", `/freeze` | Freezes every WebAgent at once (W5's Freeze, `how: "chat"`); the site keeps serving its signed release; unfreezing needs the console |
| `/pause <WebAgent>` | Lowers each of that WebAgent's jobs to Watch through W1's dials (`Dials::lower`, a receipt each; lowering never needs approval). Raising it again is a decision card in the console |
| `/brief` | The latest brief |
| `/decisions` | Waiting cards, each with Open and Decline |
| `/site <question>` | The question asked of the site exactly as a customer would, so the owner sees what customers see |
| `/stop` | Unlinks this chat (§4.2) |
| Anything else | A question for the Chief WebAgent, answered from the brief and the waiting cards through the Model Gateway (purpose `brief`), gated by W1's `owner-questions` dial; it can't approve anything and says so |

Words come from the same grammar W11 uses for voice (`agentrix_media::intent::read`), so "stop
everything" means the same spoken or typed.

### 4.5 Customers

A chat that isn't linked as an owner is a customer of the bot's site.

- Each text message is asked of the site's own Ask (`POST /ask` on the site's host, over loopback,
  exactly as the page asks it), so the answer, the desk that answers, the citations, the refusals
  ("This site doesn't say, so I won't guess.") and the order-status instant answers are the same.
  Horizon 2 swaps the loopback call for an in-process port W2 exposes.
- The reply names the desk ("Cakes desk · Kilele Bakery"), keeps the citations as numbered links
  to the page, turns the answer's real actions into buttons (links open the site; "Yes, ask
  Wanjiru" passes the question through the site's own pass route), and ends with "Answered by
  Kilele Bakery's WebAgents from the site's pages" so AI is never hidden. The sample band's word
  "Sample" is in the bot's name and its first reply.
- `/start` without a code greets the customer, says the site's WebAgents answer from its pages,
  and offers three starter questions from the site's own desks.
- Customers decide nothing for the owner and can't reach the back office (docs/05 §7).
- **Rate limits:** 20 messages a minute per chat; past that, one line saying when to try again.
  Messages over 400 characters are cut to 400, as Ask does.
- **What's kept:** for each turn, the chat id, the desk, the question cut to 200 characters, the
  answer's text and Telegram's message ids, for 30 days, so the owner's Live view and support can
  see what was said. Erasure: `/forget` deletes the chat's turns at once; the owner's data export
  includes them.

### 4.6 Messages, outbox and receipts

- **Every notification** goes through W9: `Messages::send` with the recipient's Telegram chat as
  the Chat destination (`telegram:<chat id>`), rendered from the catalogue template's chat
  version, recorded in the outbox (status, attempts, Telegram's `message_id` as `provider_ref`),
  held for quiet hours by W9's rule and released by W9's loop. W13 registers a Telegram chat
  transport with `agentrix-messages`, so every crate's `Messages` delivers Chat through it.
- **Conversational replies** (the answer to what the person just sent) aren't catalogue messages;
  they're logged per turn (§4.5) with Telegram's message id as the proof of delivery.
- **Receipts:** `channel.changed` for link and unlink; `decision.declined` for a chat decline (W5);
  pausing writes W1's dial receipts; Freeze records who and how in W5's `shell_freezes`.

### 4.7 Failure words

| What broke | What the chat says |
|---|---|
| The Model Gateway is down (owner question) | "I couldn't answer that just now. Try again in a minute, or open the console." |
| Ask failed (customer) | "Answers are paused right now. The site still works: <link>" |
| A card was already decided | "Already decided in the console. Nothing changed here." |
| The card expired | "That decision expired, so nothing was done." |
| The site is frozen | Owner: "Every WebAgent is frozen. Unfreeze in the console." Customer: the site's resting answer |

## 5. WhatsApp and email

- **WhatsApp** is W9's Chat channel behind its setting ("WhatsApp isn't connected yet" until a
  number and the Cloud API token exist). The Channels app shows its state and the same rule table;
  the Telegram transport and a WhatsApp transport sit behind one chat registration, so when
  WhatsApp switches on it offers exactly what Telegram does.
- **Email** carries the brief, the weekly review and receipts, with links into the console and
  `/d/{card}`; it decides nothing (ADR-014). `owner.decision_waiting` has no email version by
  design.

## 6. The phone on the web: the split with W5

W5 owns the web phone view: `/console` at phone width with Now, Decisions, Live and Apps, the
decision page, the approval sheet, Freeze and unfreeze. W13 adds, without editing `agentrix-shell`:

| W13 | Where |
|---|---|
| The **Channels** console app (C9): link and unlink Telegram, WhatsApp and email state, the rule table | `agentrix-channels`, in the dock; W5 links it from Settings |
| `GET /d/{card}`: the short link chats and email carry | Platform host: signed in and owning the card's site → `/console/{site}/decisions/{id}`; else sign-in with `next` |
| The mobile JSON API (§9) | `/api/m/v1/…` on the platform host |

## 7. The Flutter apps

One Flutter workspace in `apps/`: shared packages and two apps.

| Path | What |
|---|---|
| `apps/packages/agentrix_kit` | Design tokens from `crates/agentrix-ui/tokens/*.css` and the brand themes from `crates/agentrix-themes/static/{kilele,classy,tana}.css`, generated into Dart (`tool/tokens.dart`), with Manrope and DM Sans bundled; buttons, chips, cards, the effect box, the approval sheet, the living mark |
| `apps/packages/agentrix_api` | Typed clients for §9, token storage in the Keychain |
| `apps/customer` | The customer app, flavours `kilele`, `classy`, `tana` (bundle ids `website.agentrix.sample.kilele` …), each with its site's host, brand theme, name and icon |
| `apps/owner` | Agentrix OS on the phone |

### 7.1 Customer app (per site)

| Screen | Concept | API |
|---|---|---|
| Home: the site's front, desks, Sample band | customer-browse | `GET /api/app/v1/site` |
| Browse a catalogue section, product | customer-browse | `GET /api/app/v1/catalogue`, `…/catalogue/{sku}` |
| Ask the site's WebAgents (desk, citations, actions, hand-over) | customer-agent | `POST /ask` (the site's own SSE) |
| Bag | customer-order | `GET/POST /api/app/v1/bag` |
| Checkout: details, collection or delivery, PayPal | customer-order, customer-pay | `POST /api/app/v1/checkout` → PayPal approval in the system browser → back to the app |
| Orders and an order (status timeline, change, help) | customer-after | `GET /api/app/v1/orders`, `…/orders/{id}` |
| Account: sign in with passkey or email code, details, addresses | customer-account | W6's `/account/api/*` on the site host |
| Choices, security, devices, export, delete | customer-security | W6's `/account/api/*` |

### 7.2 Owner app

| Screen (phone.html) | API |
|---|---|
| Pair with the console (QR or code from Settings → Channels → Phone app) | `POST /api/m/v1/pair` |
| Now: greeting, brief, Needs you stack (swipe = snooze, tap = open, never swipe to approve), live strip, mission widget | `GET /api/m/v1/sites/{site}/now` |
| Decisions: one card at a time, exact effect, why this risk | `GET …/cards`, `…/cards/{id}` |
| Approve with passkey, or authenticator code, bound to the fingerprint; Decline | `GET …/cards/{id}/sheet`, `POST …/cards/{id}/approve`, `…/decline` |
| Apps: WebAgents with presence, Store, Activity, Missions, Trust; hold to talk; Freeze | `GET …/webagents`, `POST …/freeze`; voice through W11's `/api/voice/{site}/*` |
| Lock screen notifications ("2 decisions are waiting", never what) | Local notifications from `…/now` while open; APNs and FCM later (§10) |

Gestures in the app are the phone's: swipe a card to snooze, long-press for the verb menu,
hold the team mark to talk. Swiping never approves.

## 8. Staff and team members

Team members don't get their own app. W6's team roles (People app: owner, manager, viewer,
billing) decide what a person may see and do, and the owner app shows that person only what their
role allows: a viewer sees Now and Decisions read-only; a manager decides the cards their role
covers, with their own passkey or code; billing sees invoices only. One app, one sign-in, one rule table. A
separate staff app would split the decision contract across two codebases.

## 9. APIs and auth

### 9.1 Owner app: `/api/m/v1` on the platform host

- **Pairing**, not passwords. A signed-in owner taps **Pair a phone** in Settings → Channels; node
  shows a QR code and an eight-character code (one-time, 10 minutes, hashed at rest). The app sends
  it to `POST /api/m/v1/pair` with a device name and gets an opaque bearer token (256 bits; only
  its SHA-256 is stored) bound to the account, listed in Channels with the device name, last used,
  and **Remove**. Tokens expire after 30 days unused.
- `Authorization: Bearer <token>` on every call; no cookies; CORS closed.
- **A token never approves on its own.** Approve calls carry the same body as the frozen approve
  contract (`{method: "totp", code}` or `{method: "passkey", credential}`), verified by node's
  `Approvals` against the card's fingerprint, exactly as the console does. A stolen phone token
  can decline and freeze, never move money.
- Native passkeys need the app's associated domain on the platform host
  (`/.well-known/apple-app-site-association` with `webcredentials`), so they come with the deploy;
  until then the app approves with an authenticator code, and the receipt says so.

### 9.2 Customer app: on each site's own host

- Sign-in is W6's visitor accounts: `POST /account/api/code/start` and `…/code/verify` (email
  code), `…/passkey/login/*` (passkeys, rpId = the site's host, needing the app's associated domain
  on that host). The session W6 issues is the app's token, kept in the Keychain; W6's devices list
  and "sign out everywhere" end it.
- `/api/app/v1/*` (site, catalogue, bag, checkout, orders) is JSON over W2's records, on the site
  host, served by W2's router (W2 owns the records and the commerce rules; W13 writes the handlers
  with W2's agreement).

## 10. Notifications

- **First:** local notifications in the owner app, from `…/now` when the app is opened or
  refreshed in the background, with the doorbell's words only ("2 decisions are waiting").
- **Later:** APNs and FCM through W9's Push channel and the Effects Gateway holding the keys; the
  payload is empty ("something is waiting"), the app fetches the rest after unlock (07 §4.3).

## 11. Look and words

- Colours, type and radii come from W7's tokens (`agentrix-ui/tokens`) and the samples' brand
  themes (`agentrix-themes/static`), generated into Dart, never retyped. Night Time is the dark
  theme. Manrope for headings, DM Sans for text.
- Every string follows `voice.md` and passes `scripts/copy-lint` (the apps' strings live in ARB
  files the lint reads; Telegram replies live in `crates/agentrix-channels/words/` and in W9's
  templates).

## 12. Data (03-data.md catalogue lines)

All in `control.db`, owned by `agentrix-channels`, migrator table `_sqlx_migrations_agentrix_channels`.

| Table | Holds |
|---|---|
| `channel_link_codes` (1300) | One-time codes for linking a chat or pairing a phone: SHA-256 of the code, purpose, account, site, expiry, used at |
| `channel_links` (1300) | Linked chats: channel, external chat id, account, site, label, linked and unlinked at |
| `channel_cursors` (1300) | The `getUpdates` offset per bot |
| `channel_alerts` (1300) | Which card or question was announced to which link, so each goes once |
| `channel_turns` (1301) | Customer and owner chat turns: chat, desk, question cut to 200 characters, answer, message ids; 30 days |
| `channel_devices` (1302) | Owner app tokens: SHA-256, account, device name, created, last used, removed |

## 13. Wiring (W0)

| Where | Line |
|---|---|
| `MIGRATIONS` | `agentrix_channels::migrate` |
| `MODULES` | `agentrix_channels::mount` (`/d/{card}`, the webhook, the poller, `/api/m/v1`) |
| The shell's apps | `agentrix_channels::console_app(ctx)` beside W9's Messages |
| `agentrix-types` | `ReceiptKind::ChannelChanged` (`channel.changed`) |
| Settings | `AGENTRIX_TELEGRAM_FILE` (local: `~/.agentrix/telegram.json`); deploy later |
