// Renders the golden cases with assets/marks.js, the browser build of the mark generator.
//
//   node tests/render.mjs            prints [{ name, svg }] as JSON (the Rust test reads it)
//   node tests/render.mjs --check    compares each SVG's SHA-256 with tests/golden/digests.json
//
// The Rust test `javascript_matches_rust` runs the first form and compares every case byte for
// byte with agentrix_marks::render, so the two builds can't drift.
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, "..", "assets", "marks.js"), "utf8");
const marks = await import("data:text/javascript;base64," + Buffer.from(source).toString("base64"));
const cases = JSON.parse(readFileSync(join(here, "golden", "cases.json"), "utf8"));

const out = cases.map(c => ({
  name: c.name,
  svg: c.kind === "team"
    ? marks.teamSVG(c.team, { size: c.size, state: c.state, busy: c.busy, font: c.font })
    : marks.markSVG(c.spec, { size: c.size, state: c.state, font: c.font, uid: c.uid })
}));

if (process.argv.includes("--check")) {
  const pinned = JSON.parse(readFileSync(join(here, "golden", "digests.json"), "utf8"));
  let bad = 0;
  for (const { name, svg } of out) {
    const got = createHash("sha256").update(svg).digest("hex");
    if (pinned[name] !== got) { bad += 1; console.error(`${name}: ${got} (pinned ${pinned[name]})`); }
  }
  console.log(`${out.length - bad} of ${out.length} cases match the pinned digests`);
  process.exit(bad ? 1 : 0);
} else {
  process.stdout.write(JSON.stringify(out));
}
