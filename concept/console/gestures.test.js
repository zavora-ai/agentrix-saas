/* Tests for gestures.js with synthetic hands: node concept/console/gestures.test.js
   Hands are built from joint angles (forward kinematics), so these test the rules, not the tracker.
   The product's targets are measured on recorded landmark clips (07 §3.9); this is not that corpus. */
'use strict';
const AG = require('./gestures.js');

let failed = 0;
function check(name, ok, detail) {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ' · ' + detail : ''}`);
  if (!ok) failed++;
}

// ---- a synthetic hand -------------------------------------------------------
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const mid = (a, b) => mul(add(a, b), 0.5);
const norm = a => mul(a, 1 / Math.hypot(...a));
const rad = d => d * Math.PI / 180;
// Palm facing the camera, fingers up. Metres; y grows downwards like the image; -z is towards the camera.
const U = [0, -1, 0], R = [1, 0, 0], N = [0, 0, -1];
const MCP = { index: [-0.025, -0.085, 0], middle: [0, -0.09, 0], ring: [0.02, -0.085, 0], pinky: [0.038, -0.075, 0] };
const LEN = { index: [0.040, 0.025, 0.020], middle: [0.045, 0.028, 0.022], ring: [0.042, 0.026, 0.021], pinky: [0.032, 0.020, 0.018] };
const SPLAY = { index: -8, middle: 0, ring: 6, pinky: 12 };
const CMC = [-0.022, -0.025, -0.005];
const OPEN = [5, 5, 3], RELAXED = [5, 20, 10], CURL = [85, 100, 60], HALF = [30, 45, 25];

function finger(name, bends) {
  const base = norm(add(mul(U, Math.cos(rad(SPLAY[name]))), mul(R, Math.sin(rad(SPLAY[name])))));
  let p = MCP[name], acc = 0;
  const out = [p];
  for (let j = 0; j < 3; j++) {
    acc += bends[j];
    p = add(p, mul(add(mul(base, Math.cos(rad(acc))), mul(N, Math.sin(rad(acc)))), LEN[name][j]));
    out.push(p);
  }
  return out;
}
function thumbAlong(dir) {
  const d = norm(dir), mcp = add(CMC, mul(d, 0.035)), ip = add(mcp, mul(d, 0.03));
  return [CMC, mcp, ip, add(ip, mul(d, 0.025))];
}
function thumbOnto(target) { // the tip rests on the outside of the curled fingers
  const tip = add(target, mul(N, 0.012));
  const mcp = add(CMC, mul(norm([0.1, -0.6, -0.8]), 0.035));
  return [CMC, mcp, add(mid(mcp, tip), mul(N, 0.008)), tip];
}
function hand(b, thumb) {
  const fs = {};
  for (const n of ['index', 'middle', 'ring', 'pinky']) fs[n] = finger(n, b[n]);
  const th = thumb === 'out' ? thumbAlong([-0.7, -0.7, -0.15])
    : thumb === 'side' ? thumbAlong([-1, -0.15, -0.1])
    : thumb === 'side-up' ? thumbAlong([-0.8, -0.6, -0.1])
    : thumbOnto(mid(fs[thumb][1], fs[thumb][2]));
  return [[0, 0, 0], ...th, ...fs.index, ...fs.middle, ...fs.ring, ...fs.pinky];
}
const fingers = (i, m, r, k) => ({ index: i, middle: m, ring: r, pinky: k });

// Turn the thumb straight up in the image: the angle that maps the thumb's direction to (0, -1).
const thumbUpTurn = w => { const v = [w[4][0] - w[1][0], w[4][1] - w[1][1]]; return -90 - Math.atan2(v[1], v[0]) * 180 / Math.PI; };

let seed = 7;
const rnd = () => { seed = (seed * 1103515245 + 12345) % 2147483648; return seed / 2147483648; };
const gauss = () => Math.sqrt(-2 * Math.log(rnd() + 1e-12)) * Math.cos(2 * Math.PI * rnd());

function frameOf(world, { rot = 0, scale = 1.6, aspect = 4 / 3, noise = 0, imgNoise = 0 } = {}) {
  const c = Math.cos(rad(rot)), s = Math.sin(rad(rot));
  const W = world.map(([x, y, z]) => [x * c - y * s, x * s + y * c, z]).map(p => p.map(v => v + (noise ? gauss() * noise : 0)));
  const lm = W.map(([x, y, z]) => ({ x: 0.5 + x * scale / aspect + (imgNoise ? gauss() * imgNoise : 0), y: 0.55 + y * scale + (imgNoise ? gauss() * imgNoise : 0), z: z * scale / aspect }));
  return { lm, world: W.map(([x, y, z]) => ({ x, y, z })), aspect };
}
const labelOf = (world, opts) => { const fr = frameOf(world, opts); return AG.classify(AG.features(fr.lm, fr.world, fr.aspect)).label; };

// ---- shapes ----------------------------------------------------------------
const thumbUp = hand(fingers(CURL, CURL, CURL, CURL), 'side');
const thumbUp2 = hand(fingers(CURL, CURL, CURL, CURL), 'side-up');
const cases = [
  ['open palm', hand(fingers(OPEN, OPEN, OPEN, OPEN), 'out'), 0, 'palm'],
  ['relaxed open hand', hand(fingers(RELAXED, RELAXED, RELAXED, RELAXED), 'out'), 0, 'palm'],
  ['fist', hand(fingers(CURL, CURL, CURL, CURL), 'index'), 0, 'fist'],
  ['fist, turned 90°', hand(fingers(CURL, CURL, CURL, CURL), 'index'), 90, 'fist'],
  ['point up', hand(fingers(OPEN, CURL, CURL, CURL), 'middle'), 0, 'point'],
  ['point up, leaning 25°', hand(fingers(OPEN, CURL, CURL, CURL), 'middle'), 25, 'point'],
  ['two fingers', hand(fingers(OPEN, OPEN, CURL, CURL), 'ring'), 0, 'victory'],
  ['thumb up', thumbUp, thumbUpTurn(thumbUp), 'thumb'],
  ['thumb up, thumb angled', thumbUp2, thumbUpTurn(thumbUp2), 'thumb'],
  ['half-bent hand (a claw)', hand(fingers(HALF, HALF, HALF, HALF), 'out'), 0, null],
  // A finger near the 0.90 cut (HALF is ~0.87) can tip either way under noise; a hooked one must not.
  ['point with a hooked index', hand(fingers([40, 60, 30], CURL, CURL, CURL), 'middle'), 0, null],
  ['open palm, fingers sideways', hand(fingers(OPEN, OPEN, OPEN, OPEN), 'out'), 90, null],
  ['point, sideways', hand(fingers(OPEN, CURL, CURL, CURL), 'middle'), 70, null],
  ['thumb out, pointing sideways', thumbUp, thumbUpTurn(thumbUp) + 90, null],
  ['thumb up, upside down', thumbUp, thumbUpTurn(thumbUp) + 180, null],
  ['three fingers', hand(fingers(OPEN, OPEN, OPEN, CURL), 'pinky'), 0, null],
];
for (const [name, w, rot, want] of cases) {
  const got = labelOf(w, { rot });
  check(`shape: ${name}`, got === want, `want ${want}, got ${got}`);
}

// Noise: about MediaPipe's jitter, plus small turns and distances. Correct most of the time,
// and never another gesture.
for (const [name, w, rot, want] of cases) {
  const tally = { right: 0, abstain: 0, wrong: 0 }, wrongs = {};
  for (let n = 0; n < 300; n++) {
    const got = labelOf(w, { rot: rot + (rnd() - 0.5) * 20, scale: 1.2 + rnd(), noise: 0.003, imgNoise: 0.002 });
    if (got === want) tally.right++;
    else if (got === null) tally.abstain++;
    else { tally.wrong++; wrongs[got] = (wrongs[got] || 0) + 1; }
  }
  const pct = Math.round(tally.right / 3);
  const ok = tally.wrong === 0 && (want === null || pct >= 90);
  check(`noise: ${name}`, ok, `${pct}% right, ${tally.abstain} abstained, ${tally.wrong} wrong ${JSON.stringify(wrongs)}`);
}

// Random hands: how often does a random pose land on each gesture? (Informational.)
{
  const count = {};
  const kinds = ['out', 'side', 'index', 'middle'];
  for (let n = 0; n < 5000; n++) {
    const b = () => [rnd() * 100, rnd() * 110, rnd() * 80];
    const w = hand(fingers(b(), b(), b(), b()), kinds[Math.floor(rnd() * 4)]);
    const got = labelOf(w, { rot: (rnd() - 0.5) * 360, noise: 0.003 }) || 'none';
    count[got] = (count[got] || 0) + 1;
  }
  console.log('info random poses (5000):', JSON.stringify(count));
}

// ---- tracker ---------------------------------------------------------------
const STEP = 33; // about 30 fps
function run(tr, from, to, fn, ctx) {
  const evs = [];
  for (let t = from; t < to; t += STEP) for (const e of tr.update(fn(t), t, typeof ctx === 'function' ? ctx(t) : ctx)) evs.push({ ...e, t });
  return evs;
}
const at = (label, x = 1, y = 0.5, open = label === 'palm') => ({ hands: 1, label, center: { x, y }, palm2d: 0.15, open });
const none = () => ({ hands: 0 });
const fmt = evs => evs.map(e => `${e.type}:${e.g}${e.quick ? '(quick)' : ''}@${e.t}`).join(' ') || '—';

{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 1500, () => at('point'));
  const b = evs.find(e => e.type === 'begin');
  check('tracker: a steady point begins once after ~150 ms', evs.length === 1 && b && b.t >= 150 && b.t <= 250, fmt(evs));
  const lost = run(tr, 1500, 2000, none);
  check('tracker: losing it ends the hold after the grace, not quick', lost.length === 1 && lost[0].type === 'end' && !lost[0].quick && lost[0].t >= 1500 + 250 && lost[0].t <= 1500 + 300, fmt(lost));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 600, t => (t < 100 ? at('point') : none()));
  check('tracker: a 100 ms flicker never begins', evs.length === 0, fmt(evs));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 2000, t => (t > 600 && t < 800 ? none() : at('fist')));
  check('tracker: a 200 ms drop-out is forgiven', evs.length === 1 && evs[0].type === 'begin', fmt(evs));
  const tr2 = AG.createTracker();
  const evs2 = run(tr2, 0, 2500, t => (t > 900 && t < 1250 ? none() : at('fist')));
  check('tracker: a 350 ms drop-out ends the hold, then it begins again', fmt(evs2).split(' ').map(s => s.split('@')[0]).join(' ') === 'begin:fist end:fist begin:fist', fmt(evs2));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 1000, t => (t < 350 ? at('thumb') : none()));
  const end = evs.find(e => e.type === 'end');
  check('tracker: a hold lost within 400 ms cancels quietly', end && end.quick, fmt(evs));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 1000, t => at('palm', 1 + t / 1000 * 0.15 * 0.8)); // 0.8 palm/s: too fast for the palm
  check('tracker: a palm drifting at 0.8 palm/s does not begin', evs.length === 0, fmt(evs));
  const tr2 = AG.createTracker();
  const evs2 = run(tr2, 0, 1000, () => at('palm'));
  check('tracker: a still palm begins after ~300 ms', evs2.length === 1 && evs2[0].t >= 300 && evs2[0].t <= 400, fmt(evs2));
  const tr3 = AG.createTracker();
  const evs3 = run(tr3, 0, 1000, t => at('point', 1 + t / 1000 * 0.15 * 2)); // 2 palm/s
  check('tracker: a moving point does not begin', evs3.length === 0, fmt(evs3));
}
{
  // Swipe right: 1.5 palms in 300 ms, then the hand comes back, then rests, then a real swipe left.
  const tr = AG.createTracker();
  const P = 0.15;
  const x = t => t < 300 ? 1 + 1.5 * P * t / 300
    : t < 700 ? 1 + 1.5 * P - 1.5 * P * (t - 300) / 400
    : t < 2200 ? 1
    : t < 2500 ? 1 - 1.5 * P * (t - 2200) / 300 : 1 - 1.5 * P;
  const evs = run(tr, 0, 3000, t => at(null, x(t), 0.5, true));
  check('tracker: swipe right, the return stroke ignored, then a swipe left', fmt(evs).split(' ').map(s => s.split('@')[0]).join(' ') === 'swipe:swipe_right swipe:swipe_left', fmt(evs));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 1600, t => at(null, 1 + 1.5 * 0.15 * t / 1500, 0.5, true));
  check('tracker: a slow drift is not a swipe', evs.length === 0, fmt(evs));
  const tr2 = AG.createTracker();
  const evs2 = run(tr2, 0, 400, t => at(null, 1 + 1.5 * 0.15 * t / 300, 0.5 + 1.5 * 0.15 * t / 300, true));
  check('tracker: a diagonal sweep is not a swipe', evs2.length === 0, fmt(evs2));
  const tr3 = AG.createTracker();
  const evs3 = run(tr3, 0, 400, t => at('fist', 1 + 1.5 * 0.15 * t / 300, 0.5, false));
  check('tracker: a closed hand moving sideways is not a swipe', evs3.length === 0, fmt(evs3));
}
{
  // Two swipes right 450 ms apart: the cooldown keeps it to one. 1100 ms apart: two.
  const sweep = (t0) => t => t < t0 ? null : t < t0 + 250 ? 1.5 * 0.15 * (t - t0) / 250 : 1.5 * 0.15;
  const mk = gap => {
    const a = sweep(0), b = sweep(gap);
    return t => at(null, 1 + (a(t) || 0) + (b(t) || 0), 0.5, true);
  };
  const near = run(AG.createTracker(), 0, 1500, mk(450));
  check('tracker: a second swipe inside the 800 ms cooldown is dropped', near.filter(e => e.type === 'swipe').length === 1, fmt(near));
  const far = run(AG.createTracker(), 0, 2000, mk(1100));
  check('tracker: two swipes 1.1 s apart both count', far.filter(e => e.type === 'swipe').length === 2, fmt(far));
}
{
  const tr = AG.createTracker();
  const evs = run(tr, 0, 1000, () => ({ hands: 2 }));
  check('tracker: two hands, nothing fires', evs.length === 0, fmt(evs));
  const tr2 = AG.createTracker();
  const evs2 = run(tr2, 0, 1000, t => (t < 500 ? at('fist') : { hands: 2 }));
  check('tracker: a second hand ends the hold', fmt(evs2).startsWith('begin:fist') && evs2.some(e => e.type === 'end'), fmt(evs2));
}
{
  const evs = run(AG.createTracker(), 0, 1000, () => at('point'), { overlay: true });
  check('tracker: with a sheet open, a point is ignored', evs.length === 0, fmt(evs));
  const evs2 = run(AG.createTracker(), 0, 1000, () => at('palm'), { overlay: true });
  check('tracker: with a sheet open, a still palm still begins', evs2.length === 1 && evs2[0].g === 'palm', fmt(evs2));
}
{
  // After a commit, a 300 ms flicker is not a release: the fist can't fire twice.
  const tr = AG.createTracker();
  const a = run(tr, 0, 1500, () => at('fist'));
  tr.committed('fist');
  const b = run(tr, 1500, 3000, t => (t > 1800 && t < 2100 ? none() : at('fist')));
  check('tracker: after a commit, a flicker does not re-arm the same shape', a.length === 1 && b.filter(e => e.type === 'begin').length === 0, fmt(a) + ' | ' + fmt(b));
  const c = run(tr, 3000, 4500, t => (t < 3700 ? none() : at('fist')));
  check('tracker: after a real release (≥ 500 ms) it can fire again', c.filter(e => e.type === 'begin').length === 1, fmt(c));
}
{
  // Switching from one shape straight into another ends the first and begins the second.
  const evs = run(AG.createTracker(), 0, 1500, t => at(t < 900 ? 'point' : 'victory'));
  check('tracker: point then two fingers', fmt(evs).split(' ').map(s => s.split('@')[0]).join(' ') === 'begin:point end:point begin:victory', fmt(evs));
}

console.log(failed ? `\n${failed} failed` : '\nall passed');
process.exit(failed ? 1 : 0);
