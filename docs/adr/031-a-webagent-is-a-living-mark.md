# ADR-031: A WebAgent is a living mark

Status: Accepted by James, 7 Nov 2026. Prototype: [`concept/identity/marks.html`](../../concept/identity/marks.html).

## Context

[06 §4.3](../06-website-experience.md#43-presence-without-the-bot-costume) and [07 §3.5](../07-owner-console.md) already settled several things:
- A WebAgent is a mark: a rounded square in the site's colours with a role glyph.
- It has no face, human name or stock avatar.
- One grammar of states, with shared motion values, runs across the site and the console.

06's research shows that animated presence draws people in, and that a body or character is a liability on a business site.

On 7 Nov James asked for a WebAgent's look to be unique and brandable, possibly as a customisable animated SVG. The existing mark gives every Cakes desk the same shape, differing only in colour and glyph. Nothing in it is unique to one WebAgent, and the owner has nothing to tune beyond colour.

## Decision

### Four layers

| Layer | What it is | Who decides |
|---|---|---|
| **Silhouette** | A squircle whose squareness starts from the site's corner radius, read at the Move. **Never a circle: circles mean a person**, such as the owner's chip when they join a thread | Owner, within the squircle range |
| **Signature** | A pattern generated from the site, the WebAgent and a seed. **One family per site** (orbits, grain, strata, weave or facets), so a site's WebAgents read as one team, and **one seed per WebAgent**, so each is unique. It shows from 28 px up, drawn in the glyph's colour at no more than 30% over the fill. **It moves only while the WebAgent is working**; idle stays still | Owner: the family for the site, a new seed per WebAgent |
| **Glyph** | A role line icon, or a monogram in the site's display face | Owner |
| **State** | The halo, the trace line, the amber dot and the grey wash, with 06 and 07's values. It adds **`speaking`**: the halo follows the real voice level from the TTS stream (U7), never a simulated one | Platform |

### What the owner chooses

- **Colour,** from the site's palette or any colour that keeps 3:1 against the site background, with the glyph at 3:1 against the mark **with its signature drawn** (WCAG 1.4.11; 06 §4.3). The signature is in the glyph's colour, so it lowers that contrast where they overlap; the check uses the signature at its strongest. The editor offers the nearest shade that passes both checks.
- **Corners, glyph and name.**
- **The site's signature family, and a fresh seed** for any WebAgent.

### What the platform keeps

- **What each motion means,** on every site and in the console.
- **The AI label** beside every mark a visitor sees.
- **No face**, eyes, human name, photo or circle.
- **The contrast minimums.** Off the site's own background, a mark can fall under 3:1, for example on the console's dark surfaces. There the host surface draws a 2 px keyline in its own ink around the squircle, and never recolours the mark (07 C12).
- **Stillness when idle.**
- **Reduced motion:** every animation stops, and each state still reads by its shape.
- **Motion settles after 5 seconds in one state.** Without reduced motion, any animation still running after 5 seconds settles into its reduced-motion look, and the label stays (06, after WCAG 2.2 SC 2.2.2). Speaking is the exception: it lasts as long as the audio, and the stop control ends both.

### The Chief WebAgent's mark is the team mark

- The body is in the site's ink, with the site's monogram.
- There is one dot per WebAgent, in that WebAgent's colour, and each dot pulses while its WebAgent works.
- The mark shows the amber dot while a delivered card waits in Needs you (07 C13).
  - Cards the daily cap holds for the brief don't light it, so the cap's quiet holds.
  - P1 cards and decisions a waiting customer is blocked on are always delivered, so they always light it.
  - A single WebAgent's own mark still shows `waiting` whenever its process waits on the owner.
- It replaces the console's old orb. The old orb was a circle with a simulated pulse, which broke two of the rules above. The owner holds the team mark to talk (07).

### Spec and generator

- **A `MarkSpec` lives in the WebAgent's spec.** Its fields are the version, squareness, fill colour, glyph (role id, or the monogram letters) and signature (family and seed). Changing it publishes with the next release.
- **The pattern comes from a seeded generator** keyed on the site, the WebAgent and the seed, so a mark is the same everywhere and can be rebuilt from its spec.
- **One algorithm, two builds.**
  - The release build renders static SVG into the signed release ([ADR-005](005-static-signed-releases.md)), and PNG for channels that take only images.
  - The console runs the same algorithm for live preview.
  - Golden tests keep the two identical.
- **State is one attribute.** Presence frames set `data-state`, and all motion is CSS on transform and opacity.

### Where the mark appears

- **On the site:** desks, peeks and threads, the agent door page, and link previews of shared artifacts. A link preview shows the site's name, the desk's mark and a generic line, never the artifact's title or contents, because messaging apps fetch previews without asking the visitor (06).
- **In the console:** the dock, WebAgent cards, decision cards and the Chief's orb.
- **Off the site:**
  - WhatsApp briefs and cards, as images padded for round crops;
  - the A2A agent card's icon;
  - MCP server and tool icons.

### Uploads

**The first release offers role icons and monograms only.** Owner-drawn glyphs come later. An uploaded drawing is hostile input: a sealed room ([ADR-003](003-sandbox-plane.md)) reduces it to plain single-colour paths, with no scripts, links or embedded images.

## Consequences

- **Every WebAgent is recognisable across the site, the console and other channels,** while each site's team keeps its own brand. The meaning of motion cannot be branded away.
- **06 §4.3 and 07 §3.5 adopt the four layers and the `speaking` state.** The presence enum gains `speaking`.
- **The console's WebAgent card gains an Appearance editor.**
- **The generator is platform code,** written in Rust for releases and in JavaScript for the console. The prototype holds the reference algorithm and the five pattern families.
