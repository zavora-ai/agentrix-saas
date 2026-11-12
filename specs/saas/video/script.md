# The film for the PayPal entry

Under 3:00 (Stage One rule). Built by [`build.py`](build.py) from [`beats.json`](beats.json):
screen captures of the judged build, four slides, a replay of the unattended run, and a narration
track. The Chief of Agentrix (an AI agent) presents, in the first person.

## Honesty rules

- **Live means live.** Every product shot is the deployed build at `paypal.agentrix.website`,
  recorded by [`rec.py`](rec.py) as it ran. A ring is drawn around the thing being talked about;
  nothing on a page is changed.
- **The replay is the record.** [`replay.html`](replay.html) plays [`record.json`](record.json),
  which [`record.py`](record.py) reads back from the live pages and from PayPal's sandbox by
  capture id. It says how many hours it compresses and when the record was read.
- **Numbers come from the record or the page on screen.** Read them again before the final cut;
  the agents keep working, so they move.
- **The terminal shots are run for real** a moment before recording
  ([`verify.sh`](verify.sh), [`impostor.sh`](impostor.sh)); anyone can run them.
- **Sped-up page captures** (at most 1.6x) only shorten scrolling. The replay and the terminal
  shots play at their own speed.
- **The voice** is a text-to-speech model speaking words written from the run's record. The end
  card says so. The owner view is the sample's public one, and its banner stays in shot.
- **Sandbox.** No real money moves; the businesses and shoppers are made up; the end card says so.

## Beats

| # | Picture | What it shows |
|---|---|---|
| b01 | Title slide | Who is presenting |
| b02–b03 | The replay | The night in 18 seconds: orders, payments, held decisions, settlements, bills |
| b04 | Live: Kilele's website | A visitor's question answered by the front desk, with its sources |
| b05 | Live: an order from the agent door | A shopper agent's signed request, checked against its own site |
| b06 | Live: the owner's morning | The Chief's count from the records; decisions held for the owner |
| b07 | Live: Kilele's money console | The mandate; the four usage invoices settled with Agentrix overnight, each with its PayPal id |
| b08–b09 | Live: Run the day on the judges' page | A scripted day pressed for real: an invoice settled, an instalment plan and its first collection, a payout, a bill held for the passkey, a changed payee refused. Labelled as scripted, with real PayPal sandbox calls |
| b10 | Terminal: `verify.sh` | An agent's badge checked from outside |
| b11 | Terminal: `impostor.sh` | A borrowed name with a made-up signature is refused; nothing moves |
| b12 | Slide | The same identity accepted by AWS, AgentCore, Google Cloud and Entra Agent ID |
| b13 | Live: Proofs | The signed audit log and the ten attacks |
| b14 | Close slide | Links and the disclosures |

**Run the day is not the overnight run.** On the live build the button plays the day on a node
of its own: scripted model answers, its own in-memory books, and real calls to PayPal's sandbox.
The film says "a scripted day" in words and in the badge. The overnight run in b02–b07 is the
live companies: real models, the books in Zavora ERA, PayPal's sandbox.

## Making it

```sh
cd specs/saas/video
python3 record.py                                  # record.json and record.js, from the live build and PayPal
for s in replay ask door today settle runday verify impostor proofs; do python3 rec.py $s clips/$s.mp4; done
python3 build.py --final                           # the film; refuses to build if a clip is missing
```

`AGX_VIDEO_CLIPS` and `AGX_VIDEO_WORK` move the clips and work files elsewhere. The narration
needs `VULTR_INFERENCE_API_KEY` in the repository's `.env`; `record.py` needs the PayPal sandbox
keys there to read each capture back.

## To re-record once they change on the server

- The badge page's own Verify button (its script is not served on the sample hosts yet), in
  place of the terminal shot in b10.
- The owner's passkey approving a real step-up, if one is set up in time.
- Run the day in b08–b09, once the page no longer says its entries are "in ERA" (fixed on
  `main`, not deployed when this was written). The button starts at most one run every five
  minutes.
