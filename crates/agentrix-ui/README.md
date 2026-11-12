# agentrix-ui

The shared UI kit (contract C10). Every page people see on agentrix.website, the owner console,
the control plane and the sample sites renders with it, so they look like one product and match
`concept/`. Owner: W7 (Design system and words). Ask W7 for a component or token instead of
writing your own colours, sizes or fonts (02-architecture.md §5.3).

## Use it

```rust
// One environment for your crate: strict undefined, the escaping formatter, the kit's templates.
let mut env = agentrix_ui::environment();
minijinja_embed::load_templates!(&mut env); // your own templates, under your own prefix
// …or, on an environment you already built:
agentrix_ui::install(&mut env)?;

// Routes: the kit's files under /static/ui/. agentrix_ui::mount(ctx) puts them on the platform
// host; on your own hosts (ops., the -sample sites) merge them into your router:
let router = my_routes.merge(agentrix_ui::assets());
```

```jinja
{% extends "ui/layouts/page.html" %}
{% import "ui/kit.html" as ui %}
{% block title %}Assess · Agentrix{% endblock %}
{% block main %}
  {{ ui.button("Check my site", kind="primary", type="submit") }}
{% endblock %}
```

Porting a concept page: copy its markup. The kit's stylesheets use the concepts' class names
(`btn btn-primary`, `chip chip-ok`, `card`, `fcard`, `ldec`, `os-bar`, `dock`, `lane`, `tl`,
`o-field`, `wa-*` …), so ported markup looks the same with no page CSS. Replace hard-coded data
with view fields, inline `style=""` with a class or a `data-` value (see below), and drawn
components with the macro when one exists.

## Files served (`/static/ui/…`, add `?v={{ ui_v }}`)

| File | What | Linked by |
|---|---|---|
| `tokens.css` | Fonts (`@font-face`), Agentrix tokens, the dark theme and Night Time, `--os-*`, scope markers, the host sets `--wa-*`. Built from `tokens/brand.css` + `tokens/extended.css`, minified | every layout |
| `brand.css` | The Agentrix layer alone (about 4 KB, minified): Manrope, DM Sans and JetBrains Mono (latin), the tokens, dark and Night Time; no shell surfaces. For pages with a tight byte budget that bring their own components (the home page) | pages that don't use a layout |
| `kit.css` | Base, buttons, chips, cards, forms, lines, tables, KPI tiles, timeline, mark slot, ladder, decision card, approval sheet, toasts, banners, states, Sample band | every layout |
| `shell.css` | The Agentrix OS shell: status bar, dock, field, lane, command bar, team mark, phone layout | `ui/layouts/shell.html` |
| `site.css` | Site hosts: AI chip, peeks, Ask dock, threads, citations, rail, sheets, artifacts | `ui/layouts/site.html` |
| `kit.js` | Night Time on schedule, meter widths and colours from `data-`, overlays and the approval sheet, toasts (`import { toast } from "/static/ui/kit.js"`), offline | every layout |
| `fonts/*.woff2`, `fonts/OFL.txt` | Manrope, DM Sans, JetBrains Mono; Fraunces, Cormorant Garamond, Newsreader, Libre Caslon Text for the samples. SIL OFL 1.1 | `tokens.css` |

No inline scripts or style attributes anywhere (`default-src 'self'`). Values markup can't carry
inline go in `data-`: `data-c="#hex"` (a mark's colour), `data-site-tile`, `data-orb-ink`,
`data-value`/`data-max` on `.meter`; `kit.js` applies them.

## Layouts

| Template | For | Blocks |
|---|---|---|
| `ui/base.html` | Everything (through a layout) | `title`, `description`, `html_attrs`, `color_scheme`, `styles`, `head`, `body_class`, `body`, `toasts`, `scripts` |
| `ui/layouts/page.html` | Platform pages: start, Assess, sign-up, sign-in, plans, claim, the Move, go-live | `banner`, `nav`, `topright`, `main_class` (`page`, `page wide`, `wrap`), `main`, `foot` |
| `ui/layouts/shell.html` | Owner console (site scope), control plane (portfolio, operator) | `scope`, `os_class` (`no-lane`, `no-cmd`, `frozen`), `bar`, `dock_label`, `dock`, `banner`, `field`, `lane_label`, `lane`, `cmd`, `overlays` |
| `ui/layouts/site.html` | Site hosts: samples, customer accounts, checkout | `theme`, `site_styles`, `header`, `main`, `footer` |
| `ui/error.html` | 404, 403, 500 on the platform host | (whole page) |

Optional page variables (base): `lang`, `theme` (`light`/`dark`), `night` (true renders Night
Time), `quiet_hours` (`{from, to, tz}`: kit.js keeps Night Time right while the page is open),
`flash` (`{text, tone}`). Site layout requires `site = {key, name, sample}`; `key` is `kilele`,
`classy`, `tana`, `kestrel` or `zavora` for the samples' brands, `site` otherwise. `sample: true`
puts the Sample band on every page.

**Themes.** Light by default. One dark theme, used when the system asks (unless
`theme="light"`), when `theme="dark"`, and inside any element with `data-time="night"`: Night
Time is the dark theme on a schedule, not a third theme. Site hosts are light only for now.

## Macros (`{% import "ui/kit.html" as ui %}`)

Names and arguments are the contract: a macro may gain optional arguments, never lose or rename
one. Views pass display-ready strings; templates never format money or dates.

| Group | Macros |
|---|---|
| Primitives | `button(label, href, kind, size, type, name, value, form, disabled, open, attrs)`, `chip(text, tone)`, `badge(n, label)`, `status(state, words)`, `state_line(state, words)`, `source(text, live)`, `cite(n, href)`, `meter(value, max, tone, label, mini)`, `kv(rows)`, `tabs(items, label)`, `segmented(items, label)`, `kbd(key)`, `logo(href)`, `person(name, initial)` |
| Surfaces | `card(title, tone, tight, raised, id, level)` and `fcard(title, sub, who, accent, span2, id)` with `{% call %}`, `note(text)`, `line(tone, text)` (`ok`, `wait`, `error`, `notice`) |
| Forms | `field(name, label, type, value, hint, error, required, autocomplete, inputmode, placeholder, attrs)`, `code_field(name, label, hint, error)`, `textarea(…)`, `select(name, label, options, value, hint, error)`, `check(name, label, checked, value)`, `switch(name, label, on)`, `or_line(text)` |
| Data | `data_table(columns, rows, caption, empty)`, `kpi({label, value, unit, window, source, href})`, `timeline(items, label)`, `steps(items)`, `receipt_row(r)`, `sealed_chip(parts)` |
| WebAgents | `mark(agent, size, state, audience, show_name)`, `team_mark(site, size, waiting, state)`, `ai_chip()`, `ladder(current, ceiling, name, lowered, why, layout, label)` |
| Decisions | `decision_card(c, size)` (`lane` or `full`), `approval_sheet(s)` |
| Shell | `site_chip(site)`, `scope_chip(scope, name, sub, href)`, `app_icon(app)`, `pill(text, kind, value, href, count)`, `time_pill(day, night)`, `lane_section(title, count)` with `{% call %}`, `quiet(text)`, `command_bar(action, placeholder, label)` |
| States | `banner(kind, text, action)`, `toast(text, tone)`, `empty(title, body, action, level)`, `loading(label)`, `skeleton(lines)`, `error_state(title, body, ref, action, level)`, `offline(saved_at)`, `read_only(reason)` |
| Site hosts | `sample_band(site)`, `provider(name, body)`, `disclosure()` |

The argument shapes are documented above each macro in `templates/ui/kit.html`, and
`tests/kit.rs` renders every one.

**The living mark** (ADR-031, W1's `agentrix-marks`). Pass the WebAgent's MarkSpec as
`agent.spec` and `ui.mark` renders the real mark inside `<wa-mark webagent=… state=…>`, which
`/_agentrix/marks.js` animates from presence frames; `agent.state` is one of the ten states,
`agent.label` the running tool ("Checking today's stock"), `agent.owner` whose OK it waits for.
`audience="visitor"` adds the AI chip and shows only what a visitor may see (asleep looks idle,
paused and frozen look resting, never with the reason). The status words come from
`agentrix_marks::words`. Without a spec the slot draws a plain squircle from `agent.color` and
`agent.glyph`. `ui.team_mark(site)` does the same with `site.team` (a TeamMark) and `site.busy`.
The functions are also there for your own markup: `wa_mark(spec, size, state, audience, font)`,
`wa_team(team, size, state, busy, font)`, `wa_state(state, audience)`,
`wa_words(state, audience, label, owner, next_wake, reason)`. Every layout links
`/_agentrix/marks.css` and `marks.js`, which `agentrix-kernel` serves on every host.

## Words

`words` (from `words/en.json`) holds the fixed lines no page or desk may rephrase: the AI
disclosure, the footer line, "Waiting for {owner}'s OK. Nothing charged.", the resting line,
mark states per audience, the ladder, decision and approval words, error pages, the Sample band.
In templates: `{{ words.visitor.disclosure }}`, `{{ words.visitor.waiting_ok|replace("{owner}", owner) }}`.
In Rust (emails, WhatsApp, SSE frames): `agentrix_ui::word("en", "visitor.disclosure")`.
Every string follows `specs/platform/voice.md`; `scripts/copy-lint` checks them.

## Review

```sh
CARGO_TARGET_DIR=… cargo run -p agentrix-ui --example gallery   # http://localhost:18700
```

`/` platform page (`?theme=dark`), `/shell` (`?night=1`, `?day=1`), `/site/kilele` (also
`classy`, `tana`, `kestrel`, `zavora`), `/error/404` (`403`, `500`).
