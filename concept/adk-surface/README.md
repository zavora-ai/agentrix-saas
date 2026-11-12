# adk-surface interface concepts

A copy of the design canvas "adk-surface interface concepts" (Spec 3.3),
(an internal board, not public), taken Sun 8 Nov 2026. Open the link to see the
screens rendered; these files are the source (Design canvas components, `.dc.html`).

It is the generic ADK-Rust agent home that the Agentrix owner console builds on: one kernel,
many surfaces, and one decision made once from anywhere. The examples use a personal agent team
(Chief of staff, Finance, Code, Research, Household); Agentrix maps them to a site's WebAgents
and the owner.

| File | Screen |
|---|---|
| `Main.dc.html` | One kernel, every surface: web home, terminal, chat, phone, desktop, voice; feed, projection, commands |
| `WebHome.dc.html` | Web home: agents list with states, a thread, the decision card (Allow once, Always allow for this person, Edit, Deny), decisions and coming up |
| `WebToday.dc.html` | While you were away: spend, quiet time, waiting count, a timeline of failed/done/suggestion/you decided/question, routines today |
| `WebTrust.dc.html` | Memory and rules: what agents remember (you said this, you confirmed this, a guess), what they can do (Just look, Suggest, Ask first, Go ahead), standing permissions with end dates, quiet time, daily spending limit |
| `PhoneInbox.dc.html` | Phone: the push "A decision is waiting", decisions list, a question, decided items with where they were decided |
| `PhoneDecide.dc.html` | Phone: a high-risk decision that needs a screen and a passkey; voice, chat and standing permissions can't approve it |
| `Channel.dc.html` | Chat channel (WhatsApp, Telegram): the morning message, a medium-risk card with Allow once or Deny, high-risk sent to the app, stop all |
| `Voice.dc.html` | Voice and video: "allow once" or "deny" by voice for everyday things only, a 3-second cancel, never keeping the recording |
| `Tray.dc.html` | Desktop tray: running state, spend, decisions, now and next, stop all |
| `Terminal.dc.html`, `TerminalLight.dc.html` | Terminal UI |

Rules every surface keeps (from `Main.dc.html`): Esc never decides. Typing never answers.
Unanswered decisions wait. Voice and chat can only allow once or deny. Agents never draw
approvals. No agent approves itself.
