/* Agentrix OS · gestures.js
   A deterministic classifier and tracker over MediaPipe hand landmarks (21 points per hand).
   No model decides what a gesture means: plain rules turn landmarks into one of six gestures,
   or into nothing. Pure functions with no DOM, so the page and the Node tests run the same code. */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.AgentrixGestures = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // Shape thresholds. Calibration (07 §3.9) would tune these per owner; these are the defaults.
  const SHAPE = Object.freeze({
    extended: 0.90,      // finger straightness (chord / arc) above this: out. A relaxed open finger is ~0.97, a claw ~0.87
    curled: 0.60,        // below this: curled. In between: unsure, and an unsure finger means no gesture
    thumbTucked: 0.35,   // thumb tip near the index or middle finger, in palm lengths
    thumbOut: 0.45,      // thumb tip clear of them
    thumbStraight: 0.85, // thumb chord / arc, for a thumb up
    thumbAbove: 0.30,    // thumb tip above the index knuckle, in image palm lengths
    pointUp: 40,         // degrees from straight up
    victoryUp: 50,
    thumbUp: 45,
    palmUp: 40,
  });

  // Timing and motion. Speeds are in palm lengths per second, so distance from the camera matters less.
  const TRACK = Object.freeze({
    stableMs: 150,        // a shape must be steady this long before its ring starts
    palmStableMs: 300,
    graceMs: 250,         // a shape may drop out this long without cancelling the hold
    quickCancelMs: 400,   // a hold lost sooner than this cancels silently
    releaseMs: 500,       // after a commit, the shape must be gone this long before it can fire again
    stillSpeed: 1.0,
    palmStillSpeed: 0.5,
    speedWindowMs: 250,
    swipeWindowMs: 600,
    swipeMinPalms: 1.2,   // horizontal travel of the palm centre
    swipeMaxSlope: 0.5,   // vertical travel must stay under half the horizontal
    swipeCooldownMs: 800,
    returnSuppressMs: 1200,
  });

  const FINGERS = { index: [5, 6, 7, 8], middle: [9, 10, 11, 12], ring: [13, 14, 15, 16], pinky: [17, 18, 19, 20] };
  const FINGER_NAMES = ['index', 'middle', 'ring', 'pinky'];
  const CONNECTIONS = [[0, 1], [1, 2], [2, 3], [3, 4], [0, 5], [5, 6], [6, 7], [7, 8], [5, 9], [9, 10], [10, 11], [11, 12],
    [9, 13], [13, 14], [14, 15], [15, 16], [13, 17], [17, 18], [18, 19], [19, 20], [0, 17]];
  const NAMES = { point: 'point up', victory: 'two fingers', thumb: 'thumb up', fist: 'fist', palm: 'open palm' };

  const d3 = (a, b) => Math.hypot(a.x - b.x, a.y - b.y, (a.z || 0) - (b.z || 0));
  const d2 = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
  // Angle between a→b and straight up in the image (y grows downwards), in degrees.
  function upAngle(a, b) {
    const vx = b.x - a.x, vy = b.y - a.y, n = Math.hypot(vx, vy);
    if (!n) return 180;
    return Math.acos(Math.max(-1, Math.min(1, -vy / n))) * 180 / Math.PI;
  }

  /* Facts about one hand.
     lm: 21 image landmarks, normalised to the frame (x by width, y by height).
     world: 21 world landmarks in metres, if the tracker gives them; shapes use these because they
     don't change when the hand turns. aspect: frame width / height. */
  function features(lm, world, aspect) {
    aspect = aspect || 4 / 3;
    const W = world && world.length === 21 ? world : lm.map(p => ({ x: p.x * aspect, y: p.y, z: (p.z || 0) * aspect }));
    // The image as the owner sees it: mirrored, with x in the same units as y.
    const P = lm.map(p => ({ x: (1 - p.x) * aspect, y: p.y }));
    const palm = d3(W[0], W[9]) || 1e-6;
    const fingers = {};
    for (const name of FINGER_NAMES) {
      const [a, b, c, e] = FINGERS[name];
      const arc = d3(W[a], W[b]) + d3(W[b], W[c]) + d3(W[c], W[e]);
      const straight = arc ? d3(W[a], W[e]) / arc : 0;
      fingers[name] = { straight, state: straight > SHAPE.extended ? 'out' : straight < SHAPE.curled ? 'curled' : 'unsure' };
    }
    const tuck = Math.min(...[6, 7, 10, 11].map(i => d3(W[4], W[i]))) / palm;
    const thumbArc = d3(W[2], W[3]) + d3(W[3], W[4]);
    const thumbStraight = thumbArc ? d3(W[2], W[4]) / thumbArc : 0;
    const palm2d = d2(P[0], P[9]) || 1e-6;
    const ids = [0, 5, 9, 13, 17];
    const center = { x: ids.reduce((s, i) => s + P[i].x, 0) / 5, y: ids.reduce((s, i) => s + P[i].y, 0) / 5 };
    return {
      fingers, tuck, thumbStraight, palm, palm2d, center,
      up: { index: upAngle(P[5], P[8]), middle: upAngle(P[9], P[12]), thumb: upAngle(P[2], P[4]) },
      thumbAbove: (P[5].y - P[4].y) / palm2d,
    };
  }

  /* One of point, victory, thumb, fist, palm, or null. Abstains whenever a finger is unsure. */
  function classify(f, th) {
    const t = th ? Object.assign({}, SHAPE, th) : SHAPE;
    const s = FINGER_NAMES.map(n => f.fingers[n].state);
    const [i, m, r, k] = s;
    const open = s.filter(x => x !== 'curled').length >= 3;
    const thumb = f.tuck < t.thumbTucked ? 'tucked' : f.tuck > t.thumbOut ? 'out' : 'unsure';
    const all = st => s.every(x => x === st);
    let label = null;
    if (all('out')) label = f.up.middle <= t.palmUp ? 'palm' : null;
    else if (i === 'out' && m === 'out' && r === 'curled' && k === 'curled') label = f.up.index <= t.victoryUp && f.up.middle <= t.victoryUp ? 'victory' : null;
    else if (i === 'out' && m === 'curled' && r === 'curled' && k === 'curled') label = f.up.index <= t.pointUp ? 'point' : null;
    else if (all('curled')) {
      if (thumb === 'out' && f.thumbStraight > t.thumbStraight && f.up.thumb <= t.thumbUp && f.thumbAbove >= t.thumbAbove) label = 'thumb';
      else if (thumb === 'tucked') label = 'fist';
    }
    return { label, open, thumb };
  }

  /* Turns a stream of per-frame facts into begin / end / swipe events.
     update(frame, t, ctx): frame = { hands, label, center, palm2d, open } (label etc. only when hands === 1);
     t in ms; ctx.overlay = a sheet is open, so only the open palm (stop everything) counts.
     Events: { type: 'begin', g }, { type: 'end', g, quick }, { type: 'swipe', g }. */
  function createTracker(opts) {
    const o = Object.assign({}, TRACK, opts);
    let hist = [], cand = { label: null, since: 0 }, active = null, lastSwipe = null, lock = null, last = {};
    const stableFor = l => (l === 'palm' ? o.palmStableMs : o.stableMs);

    function speedNow(t) {
      const cur = hist[hist.length - 1];
      let ref = null;
      for (const h of hist) if (h !== cur && t - h.t <= o.speedWindowMs) { ref = h; break; }
      if (!ref && hist.length > 1) ref = hist[hist.length - 2];
      if (!ref || t - ref.t <= 0 || t - ref.t > 1000) return null;
      return d2(cur.c, ref.c) / ((t - ref.t) / 1000) / (cur.palm || 1e-6);
    }

    function update(frame, t, ctx) {
      const out = [];
      const overlay = !!(ctx && ctx.overlay);
      const one = !!(frame && frame.hands === 1 && frame.center && frame.palm2d > 0);
      const raw = one ? frame.label || null : null;
      if (one) {
        hist.push({ t, c: frame.center, palm: frame.palm2d, open: !!frame.open });
        while (hist.length && t - hist[0].t > Math.max(o.swipeWindowMs, 1000)) hist.shift();
      } else hist = [];
      const speed = one ? speedNow(t) : null;

      // Swipe: an open hand travelling sideways. Mirrored coordinates, so +x is the owner's right.
      let swiped = false;
      if (one && !overlay && frame.open) {
        const cur = hist[hist.length - 1];
        const suppressing = lastSwipe && t - lastSwipe.t < o.returnSuppressMs;
        if (suppressing && hist.length > 1) {
          const dx = cur.c.x - hist[hist.length - 2].c.x;
          // The hand coming back after a swipe is not a swipe the other way.
          if ((lastSwipe.dir === 'right' && dx < 0) || (lastSwipe.dir === 'left' && dx > 0)) hist = [cur];
        }
        // Movement during the cooldown is dropped, so a swipe can never fire late.
        if (lastSwipe && t - lastSwipe.t < o.swipeCooldownMs) hist = [cur];
        else {
          for (const h of hist) {
            if (h === cur || !h.open || t - h.t > o.swipeWindowMs) continue;
            const dx = cur.c.x - h.c.x, dy = cur.c.y - h.c.y, palm = (cur.palm + h.palm) / 2;
            if (Math.abs(dx) >= o.swipeMinPalms * palm && Math.abs(dy) <= o.swipeMaxSlope * Math.abs(dx)) {
              const dir = dx > 0 ? 'right' : 'left';
              if (suppressing && dir !== lastSwipe.dir) { hist = [cur]; break; }
              out.push({ type: 'swipe', g: dir === 'right' ? 'swipe_right' : 'swipe_left' });
              lastSwipe = { dir, t }; hist = [cur]; swiped = true;
              break;
            }
          }
        }
      }

      // After a commit, the same shape must be released before it can fire again.
      if (lock) {
        if (raw === lock.g) lock.absentSince = null;
        else if (lock.absentSince === null) lock.absentSince = t;
        else if (t - lock.absentSince >= o.releaseMs) lock = null;
      }

      // Holds: a shape that is steady and still.
      let label = raw, still = null;
      if (label) {
        still = speed !== null && speed <= (label === 'palm' ? o.palmStillSpeed : o.stillSpeed);
        if (!still) label = null;
      }
      if (overlay && label !== 'palm') label = null;
      if (swiped) label = null;
      if (label !== cand.label) cand = { label, since: t };
      const steady = label && t - cand.since >= stableFor(label);

      if (active) {
        if (label === active.label) active.seen = t;
        else if (swiped || t - active.seen > o.graceMs || steady) {
          out.push({ type: 'end', g: active.label, quick: active.seen - active.since < o.quickCancelMs });
          active = null;
        }
      }
      if (!active && steady && !(lock && lock.g === label)) {
        out.push({ type: 'begin', g: label });
        active = { label, since: t, seen: t };
      }
      last = { hands: frame ? frame.hands || 0 : 0, raw, speed, still, active: active ? active.label : null, seen: active ? active.seen : null };
      return out;
    }

    return {
      update,
      committed(g) { lock = { g, absentSince: null }; },
      status: () => last,
      reset() { hist = []; cand = { label: null, since: 0 }; active = null; lastSwipe = null; lock = null; last = {}; },
    };
  }

  /* One line for the live readout. */
  function describe(hands, f, c, st) {
    if (!hands) return 'No hand in view';
    if (hands > 1) return `${hands} hands in view · nothing fires`;
    const word = { out: 'out', curled: 'curled', unsure: '?' };
    const fingers = FINGER_NAMES.map(n => `${n} ${word[f.fingers[n].state]}`).join(' · ');
    const motion = st && st.speed != null ? (st.speed <= TRACK.palmStillSpeed ? 'still' : st.speed <= TRACK.stillSpeed ? 'steady' : 'moving') : '';
    return `1 hand · ${fingers} · thumb ${c.thumb} → ${c.label ? NAMES[c.label] : 'no shape'}${motion ? ' · ' + motion : ''}`;
  }

  return { SHAPE, TRACK, FINGERS, CONNECTIONS, NAMES, features, classify, createTracker, describe };
});
