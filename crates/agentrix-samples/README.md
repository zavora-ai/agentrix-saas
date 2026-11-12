# agentrix-samples

The three sample websites (W2): **Kilele Bakery** (`kilele-bakery-sample.<platform host>`),
**Classy Perfumes** (`classy-perfumes-sample.…`) and **Tana Valley Times**
(`tana-valley-times-sample.…`). Every page is server-rendered from `agentrix-records`, marked
Sample, in the look of its concept page.

## Run it

```sh
cargo run -p agentrix-samples --example preview      # CARGO_TARGET_DIR=…/.agentrix-platform-target
open http://kilele-bakery-sample.localhost:18200/
open http://localhost:18200/samples/kilele-bakery-sample/        # owner apps
```

The preview reads templates and bundles from the repository on every request
(`AGENTRIX_SAMPLES_TEMPLATES`, `AGENTRIX_SAMPLES_BUNDLES`), so markup and seed edits show on
reload; Rust changes need a restart. `AGENTRIX_SAMPLES_RESEED=1` reloads bundles at start.

## How it fits together

| Piece | Where |
|---|---|
| Records (catalogue, price rules, stock, slots, carts, orders, holds, stories, subscribers) | `crates/agentrix-records` (own `records.db`, migrations 0200+) |
| Bundles (seed data, relative times, no totals) | `fixtures/samples/<site>/`, written by `fixtures/samples/tools/*.py` |
| CSS and script islands | `crates/agentrix-themes/static/` served at `/_s/<file>?v=…` |
| Router by host, site context, cookies, filters | `src/web.rs` |
| Shell (desks with live facts, Ask bar, rail, Sample band) | `src/shell.rs`, `templates/base.html`, `templates/macros.html`, `templates/<brand>/layout.html` |
| Ask (instant answers, retrieval, gateway, quote fallback) | `src/ask.rs`; per-site hooks `instant`, `sections`, `desk_facts` |
| Kilele pages | `src/bakery.rs`, `templates/kilele/` |
| Kilele cake studio and simple cake form | `src/studio.rs`, `templates/kilele/studio.html` |
| Bag, checkout, pay, receipts, orders, account pages, stand-in sign-in | `src/commerce.rs`, `templates/shop/` |
| Classy Perfumes | `src/classy.rs`, `templates/classy/` |
| Tana Valley Times | `src/news.rs`, `templates/tana/` |
| Owner apps (platform host, `/samples/<site>/…`) | `src/owner.rs`, `templates/owner/` |
| Agent door (MCP with the D9 tools, AI catalog, `/agents`) | `src/door.rs`, `templates/door/` |

### Conventions

- Handlers take `State(app): State<App>` and `Extension(ctx): Extension<Ctx>` (site, host,
  visitor, path, cart cookie) and return `web::Page`. Render with `app.render("dir/file.html",
  &view)`; every view carries `shell: app.shell(&ctx, current_desk, title, description)`.
- Templates run in minijinja strict mode: a field the template uses must exist in the view.
  Pages for a brand extend `<brand>/layout.html` and fill `{% block main %}`; shared pages
  (bag, orders, account) use `{% extends shell.layout %}`.
- Money is minor units; format with the `ksh`, `usd`, `money(currency)` filters or
  `agentrix_records::money`. Times are formatted in Rust with the site's offset
  (`agentrix_records::time::fmt_day`, `fmt_clock`).
- Never hard-code a figure that has a source. Stock, slot use, totals and prices come from
  records; facts (hours, address, policies, desks) from the bundle's `manifest.json`.
- Words follow `specs/platform/voice.md`. AI is labelled; the owner is a person one tap away.
- Ports in `src/ports.rs` take W6's visitor accounts and payments and the Model Gateway; each
  has a stand-in so the samples work before those land.
