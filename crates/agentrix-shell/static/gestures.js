// Owner console: hand gestures (01b C42; concept/console/gestures.html, made real).
// gestures-core.js (plain rules over MediaPipe's 21 hand points, tested in concept/console) decides
// which shape a hand makes; this island turns held shapes into the shell's own verbs on the real
// lane. A gesture never approves: thumb up only opens the approval sheet, and the passkey or a
// typed code does the rest. Frames never leave the browser.
import { toast } from "/static/ui/kit.js";

const AG = window.AgentrixGestures;
const root = document.getElementById("gos");
if (AG && root) {
  const $ = (s, r = document) => r.querySelector(s);
  const gbar = $("[data-gbar]", root), fill = $("[data-fill]", root);
  const gtitle = $("[data-gtitle]", root), gsub = $("[data-gsub]", root), glyph = $("[data-glyph]", root);
  const log = $("[data-log]", root), readoutEl = $("[data-readout]", root), opened = $("[data-opened]", root);
  const freezeUrl = root.dataset.freezeUrl || "";
  const home = root.dataset.home || "/os";
  const C = 144.5;
  const G = {
    swipe_left: { glyph: "👈", name: "Swipe left", verb: "Previous card", hold: 0 },
    swipe_right: { glyph: "👉", name: "Swipe right", verb: "Next card", hold: 0 },
    point: { glyph: "☝️", name: "Point up", verb: "Open the focused card", hold: 800 },
    thumb: { glyph: "👍", name: "Thumb up", verb: "Review and approve: opens the approval sheet", hold: 1200 },
    fist: { glyph: "✊", name: "Fist", verb: "Snooze until the next brief", hold: 1200 },
    victory: { glyph: "✌️", name: "Two fingers", verb: "Open Front of house", hold: 800 },
    palm: { glyph: "✋", name: "Open palm, still", verb: "Stop everything", hold: 2000 },
  };
  let camMode = "off", camOn = false, focus = -1, muted = false, ac = null, raf = 0, holding = null, holdSrc = "sim", seenAt = 0, rearm = {};

  // ---- sound (WebAudio, created on first interaction) ----
  function tone(f, d, type = "sine", vol = 0.05, glide) {
    if (muted) return;
    try {
      ac = ac || new (window.AudioContext || window.webkitAudioContext)();
      const o = ac.createOscillator(), g = ac.createGain();
      o.type = type; o.frequency.value = f; if (glide) o.frequency.linearRampToValueAtTime(glide, ac.currentTime + d);
      g.gain.value = vol; g.gain.exponentialRampToValueAtTime(0.0001, ac.currentTime + d);
      o.connect(g).connect(ac.destination); o.start(); o.stop(ac.currentTime + d);
    } catch { /* no sound is fine */ }
  }
  const sTick = () => tone(880, 0.06), sDing = () => { tone(988, 0.12); setTimeout(() => tone(1319, 0.18), 90); }, sThunk = () => tone(180, 0.14, "triangle", 0.06);
  $("[data-mute]", root)?.addEventListener("click", (e) => { muted = !muted; e.currentTarget.setAttribute("aria-pressed", String(muted)); });

  // ---- the bar: what will happen, before it happens ----
  function hud(state, t, sub, g, pct) {
    gbar.dataset.state = state; gtitle.textContent = t; gsub.textContent = sub; glyph.textContent = g;
    fill.style.strokeDashoffset = String(C * (1 - (pct || 0)));
    gbar.querySelector("[data-cam]")?.toggleAttribute("hidden", camMode !== "off");
  }
  const idle = () => camOn
    ? hud("idle", "Camera on · show a hand", "Hold a shape still until the ring fills. Lower your hand to cancel.", "🖐️", 0)
    : hud("off", "Gestures are off", "Turn on the camera. Recognition runs on this device; no frames leave it.", "📷", 0);
  function record(gesture, verb, outcome) {
    if (log.firstElementChild && log.firstElementChild.textContent === "No gestures yet") log.replaceChildren();
    const li = document.createElement("li");
    const b = document.createElement("b");
    b.textContent = G[gesture] ? G[gesture].name : gesture;
    li.append(new Date().toTimeString().slice(0, 5) + " ", b, ` · ${verb} · ${outcome}`);
    log.prepend(li);
  }

  // ---- verbs: the same ones the keys and taps use, on the real lane ----
  const cards = () => [...document.querySelectorAll('.lane [data-lane="day"] .ldec[data-card]')].filter((c) => !c.classList.contains("done"));
  function setFocus(i) {
    const cs = cards(); if (!cs.length) return false;
    focus = (i + cs.length) % cs.length;
    document.querySelectorAll(".ldec.focused").forEach((c) => c.classList.remove("focused"));
    cs[focus].classList.add("focused"); cs[focus].scrollIntoView({ block: "nearest" });
    return true;
  }
  const focused = () => cards()[focus] || null;
  function verb(name) {
    const c = focused();
    switch (name) {
      case "swipe_right": return setFocus(focus + 1) ? "next card" : "no cards";
      case "swipe_left": return setFocus(focus < 0 ? 0 : focus - 1) ? "previous card" : "no cards";
      case "point": {
        if (!c) return setFocus(0) ? "first card" : "no cards";
        const title = c.querySelector(".t")?.textContent.trim() || "This card";
        const line = c.querySelector("p")?.textContent.trim() || "";
        const link = c.querySelector(".t a")?.getAttribute("href");
        const b = document.createElement("b"); b.textContent = title;
        const p = document.createElement("p"); p.className = "small"; p.textContent = line;
        const hint = document.createElement("p"); hint.className = "small muted"; hint.textContent = "Opened by a gesture. To approve, hold a thumb up: it opens the approval sheet.";
        const parts = [b, p, hint];
        if (link) { const a = document.createElement("a"); a.className = "btn btn-sm"; a.href = link; a.textContent = "See what changes"; parts.push(a); }
        opened.replaceChildren(...parts);
        return "open";
      }
      case "thumb": {
        if (!c) return setFocus(0) ? "first card" : "no cards";
        const approve = c.querySelector("[data-sheet]");
        if (approve) approve.click();
        return "approval sheet opened";
      }
      case "fist":
        toast("Snoozing a card isn't available yet, so nothing changed.");
        return "not available";
      case "victory":
        location.assign(`${home}?world=front`);
        return "Front of house";
      case "palm":
        stopAll();
        return "stop everything";
    }
    return "";
  }

  // ---- stop everything after a 5-second countdown ----
  let stopT = null;
  const stopwin = document.querySelector("[data-stopwin]"), count = document.querySelector("[data-count]");
  async function freezeNow() {
    if (!freezeUrl) {
      hud("cancelled", "Freeze isn't on for this site", "Freezing isn't switched on for this site yet, so nothing was stopped.", "✋", 0);
      record("palm", "stop everything", "not available");
      return;
    }
    try {
      const body = new URLSearchParams({ how: "gesture", back: location.pathname });
      const r = await fetch(freezeUrl, { method: "POST", credentials: "same-origin", headers: { Accept: "application/json", "Content-Type": "application/x-www-form-urlencoded" }, body });
      if (!r.ok) throw new Error();
      record("palm", "stop everything", "frozen");
      location.reload();
    } catch {
      hud("cancelled", "The stop didn't go in", "Press Freeze in the bar above. Nothing was stopped by the gesture.", "✋", 0);
      record("palm", "stop everything", "failed");
    }
  }
  function stopAll() {
    if (stopT) return;
    let n = 5; count.textContent = n; stopwin.classList.add("on");
    stopT = setInterval(() => { n--; count.textContent = n; if (n <= 0) { clearInterval(stopT); stopT = null; stopwin.classList.remove("on"); freezeNow(); } }, 1000);
  }
  function cancelStop() { if (!stopT) return false; clearInterval(stopT); stopT = null; stopwin.classList.remove("on"); sThunk(); record("palm", "stop everything", "cancelled in the countdown"); return true; }
  document.querySelector("[data-stopcancel]")?.addEventListener("click", cancelStop);

  // ---- hold to confirm: time decides, and a camera hold fills only while the shape is seen ----
  let holdStart = 0;
  function commit(g) {
    const d = G[g];
    holding = null; cancelAnimationFrame(raf); rearm[g] = true;
    if (holdSrc === "camera" && tracker) tracker.committed(g);
    const v = verb(g);
    hud("committed", `${d.name} · done`, g === "thumb" ? "The approval sheet is open. Only you can approve: your passkey, or a code." : g === "palm" ? "5 seconds to cancel: press Esc or tap Cancel." : d.verb, d.glyph, 1);
    sDing(); if (g !== "palm") record(g, v, "done");
    setTimeout(() => { if (!holding) idle(); }, 1400);
  }
  function begin(g, src) {
    if (src === "camera" && !camOn) return;
    if (rearm[g]) { hud("idle", "Let go first", "Lower your hand or let go, then make the gesture again.", G[g].glyph, 0); return; }
    const d = G[g];
    sTick();
    if (!d.hold) { const v = verb(g); hud("committed", `${d.name} · ${d.verb}`, "Reversible: swipe back to undo.", d.glyph, 1); sDing(); record(g, v, "done"); setTimeout(() => { if (!holding) idle(); }, 900); return; }
    cancelAnimationFrame(raf);
    const t0 = performance.now(); holding = g; holdSrc = src; holdStart = seenAt = t0;
    let last = 0;
    const step = (now) => {
      if (holding !== g) return;
      const p = Math.max(0, Math.min(1, ((holdSrc === "camera" ? seenAt : now) - t0) / d.hold));
      if (p - last > 0.2) { tone(440 + p * 440, 0.05, "sine", 0.03); last = p; }
      hud("progress", `${d.name} · ${d.verb}`, holdSrc === "camera" ? "Keep holding still… lower your hand to cancel." : "Keep holding… let go to cancel.", d.glyph, p);
      if (p >= 1) { commit(g); return; }
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
  }
  function end(g, src) {
    rearm[g] = false;
    if (holding !== g) return;
    if ((holdSrc === "camera" ? seenAt : performance.now()) - holdStart >= G[g].hold) { commit(g); rearm[g] = false; return; }
    holding = null; cancelAnimationFrame(raf);
    if (src === "quiet") { idle(); return; }
    hud("cancelled", `${G[g].name} · cancelled`, src === "camera" ? "The shape went away before the ring filled, so nothing happened." : "You let go before the ring filled, so nothing happened.", G[g].glyph, 0);
    sThunk(); record(g, "nothing", "cancelled"); setTimeout(() => { if (!holding) idle(); }, 1100);
  }
  root.querySelectorAll(".sim-pad button").forEach((b) => {
    const g = b.dataset.g;
    b.addEventListener("pointerdown", (e) => { e.preventDefault(); b.classList.add("held"); begin(g, "sim"); });
    ["pointerup", "pointerleave", "pointercancel"].forEach((ev) => b.addEventListener(ev, () => { if (b.classList.contains("held")) { b.classList.remove("held"); end(g, "sim"); } }));
    b.addEventListener("keydown", (e) => { if ((e.key === " " || e.key === "Enter") && !e.repeat) { e.preventDefault(); b.classList.add("held"); begin(g, "sim"); } });
    b.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") { b.classList.remove("held"); end(g, "sim"); } });
  });

  // ---- the real camera: MediaPipe's hand tracker in this browser; frames are dropped ----
  const MP = "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.14";
  const MODEL = "https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task";
  const camtile = $("[data-camtile]", root), video = $("[data-camvideo]", root), canvas = $("[data-camcanvas]", root), g2 = canvas.getContext("2d");
  let landmarker = null, loading = null, stream = null, gen = 0, tracker = null, aspect = 4 / 3;
  let lastVideoTime = -1, lastHandAt = 0, frames = 0, fps = 0, fpsT = 0, lastReadout = 0;
  const FAIL = {
    NotAllowedError: { title: "Camera blocked", sub: "Allow the camera for this site in your browser's settings, then turn gestures on again. The keys and the buttons on this page still work." },
    NotFoundError: { title: "No camera found", sub: "Connect a camera and try again. The keys and the buttons on this page still work." },
    NotReadableError: { title: "The camera is busy", sub: "Another app is using it. Close that app, then turn gestures on again." },
    insecure: { title: "Gestures need a secure page", sub: "Open the console over https, or from localhost." },
    model: { title: "The hand tracker didn't load", sub: "Check the connection and try again. No frame was sent anywhere, and the keys still work." },
    ended: { title: "The camera stopped", sub: "It was unplugged or taken by another app. Turn gestures on again when it is back." },
    other: { title: "The camera didn't start", sub: "Try again, or use the keys and the buttons on this page." },
  };
  FAIL.SecurityError = FAIL.NotAllowedError; FAIL.OverconstrainedError = FAIL.NotFoundError; FAIL.AbortError = FAIL.NotReadableError;
  const OFF = {
    noHand: { title: "Camera off", sub: "No hand for 2 minutes, so it turned itself off." },
    hidden: { title: "Camera off", sub: "You left the tab, so it turned itself off." },
  };
  function loadLandmarker() {
    if (landmarker) return Promise.resolve(landmarker);
    if (!loading) {
      loading = (async () => {
        const { FilesetResolver, HandLandmarker } = await import(`${MP}/vision_bundle.mjs`);
        const files = await FilesetResolver.forVisionTasks(`${MP}/wasm`);
        const make = (delegate) => HandLandmarker.createFromOptions(files, {
          baseOptions: { modelAssetPath: MODEL, delegate },
          runningMode: "VIDEO", numHands: 2,
          minHandDetectionConfidence: 0.6, minHandPresenceConfidence: 0.6, minTrackingConfidence: 0.5,
        });
        try { return await make("GPU"); } catch { return await make("CPU"); }
      })().then((l) => (landmarker = l), (err) => { loading = null; throw err; });
    }
    return loading;
  }
  const readout = (text) => { readoutEl.textContent = text; };
  function setMode(mode, reason) {
    camMode = mode; camOn = mode === "on";
    document.querySelectorAll(".pill.camera").forEach((p) => {
      p.setAttribute("aria-pressed", String(camOn));
      const t = p.querySelector("[data-cam-text]"); if (t) t.textContent = camOn ? "Camera on · gestures" : mode === "starting" ? "Starting camera…" : "Gestures off";
    });
    camtile.hidden = !camOn;
    if (mode === "starting") hud("idle", "Starting the camera…", "Allow the camera when your browser asks. The first time, the hand tracker takes a few seconds to load.", "📷", 0);
    else if (mode === "off" && reason) hud("off", reason.title, reason.sub, "📷", 0);
    else idle();
  }
  async function startCamera() {
    const my = ++gen;
    if (!window.isSecureContext || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) { stopCamera(FAIL.insecure); return; }
    setMode("starting"); sTick();
    readout("Starting the camera and loading the hand tracker…");
    const [cam, lm] = await Promise.allSettled([
      navigator.mediaDevices.getUserMedia({ audio: false, video: { facingMode: "user", width: { ideal: 640 }, height: { ideal: 480 } } }),
      loadLandmarker(),
    ]);
    if (cam.status === "fulfilled" && (my !== gen || lm.status === "rejected")) cam.value.getTracks().forEach((t) => t.stop());
    if (my !== gen) return;
    if (cam.status === "rejected") { stopCamera(FAIL[cam.reason && cam.reason.name] || FAIL.other); return; }
    if (lm.status === "rejected") { stopCamera(FAIL.model); return; }
    stream = cam.value;
    const track = stream.getVideoTracks()[0];
    if (track) track.addEventListener("ended", () => { if (my === gen) stopCamera(FAIL.ended); });
    video.srcObject = stream;
    setMode("on");
    try { await video.play(); } catch { /* autoplay may need a gesture; frames still arrive */ }
    if (my !== gen) return;
    const vw = video.videoWidth || 640, vh = video.videoHeight || 480;
    aspect = vw / vh;
    canvas.width = 160; canvas.height = Math.round((160 * vh) / vw);
    tracker = AG.createTracker();
    lastVideoTime = -1; frames = 0; fps = 0; fpsT = lastHandAt = performance.now();
    readout("Camera on · looking for a hand");
    loop(my);
  }
  function stopCamera(reason) {
    gen++;
    if (stream) stream.getTracks().forEach((t) => t.stop());
    stream = null; video.srcObject = null; tracker = null;
    g2.clearRect(0, 0, canvas.width, canvas.height);
    if (holding && holdSrc === "camera") { holding = null; cancelAnimationFrame(raf); }
    rearm = {};
    setMode("off", reason);
    readout(reason ? `${reason.title} · ${reason.sub}` : "Camera off");
  }
  function loop(my) {
    if (my !== gen || !stream) return;
    const now = performance.now();
    if (video.readyState >= 2 && video.currentTime !== lastVideoTime) {
      lastVideoTime = video.currentTime;
      let res;
      try { res = landmarker.detectForVideo(video, now); } catch { stopCamera(FAIL.model); return; }
      onFrame(res, now);
      if (my !== gen) return;
    }
    requestAnimationFrame(() => loop(my));
  }
  function onFrame(res, now) {
    const lms = res.landmarks || [], n = lms.length;
    let frame = { hands: n }, f = null, c = null;
    if (n) lastHandAt = now;
    if (n === 1) {
      f = AG.features(lms[0], res.worldLandmarks && res.worldLandmarks[0], aspect);
      c = AG.classify(f);
      frame = { hands: 1, label: c.label, center: f.center, palm2d: f.palm2d, open: c.open };
    }
    const evs = tracker.update(frame, now, { overlay: !!document.querySelector(".overlay.on") });
    const st = tracker.status();
    if (holding && holdSrc === "camera" && st.active === holding) seenAt = st.seen;
    for (const ev of evs) {
      if (ev.type === "end") end(ev.g, ev.quick ? "quiet" : "camera");
      else if (stopT) continue;
      else if (ev.type === "swipe") { begin(ev.g, "camera"); end(ev.g, "camera"); }
      else begin(ev.g, "camera");
    }
    draw(lms, !!(c && c.label));
    frames++;
    if (now - fpsT >= 1000) { fps = Math.round((frames * 1000) / (now - fpsT)); frames = 0; fpsT = now; }
    if (now - lastReadout >= 200) { lastReadout = now; readout(AG.describe(n, f, c, st) + (fps ? ` · ${fps} fps` : "")); hint(n, c); }
    if (now - lastHandAt > 120000) stopCamera(OFF.noHand);
  }
  function hint(n, c) {
    if (!camOn || gbar.dataset.state !== "idle") return;
    const sheet = !!document.querySelector(".overlay.on");
    const [t, s] = !n ? ["Camera on · show a hand", "Hold a shape still until the ring fills. Lower your hand to cancel."]
      : n > 1 ? ["Two hands in view", "Nothing fires with two hands. Lower one."]
      : sheet && !(c && c.label === "palm") ? ["A sheet is open", "Gestures wait until it closes; confirm or cancel with a tap. An open palm still stops everything."]
      : c && c.label ? [`${G[c.label].name}?`, "Hold it still…"]
      : ["Hand found", "Make one of the six shapes and hold it still, or swipe sideways with an open hand."];
    gtitle.textContent = t; gsub.textContent = s;
  }
  function draw(lms, recognised) {
    const w = canvas.width, h = canvas.height;
    g2.clearRect(0, 0, w, h);
    g2.strokeStyle = g2.fillStyle = lms.length > 1 ? "#ffb020" : recognised ? "#ff8aa2" : "rgba(255,255,255,.9)";
    g2.lineWidth = 1.5;
    for (const lm of lms) {
      g2.beginPath();
      for (const [a, b] of AG.CONNECTIONS) { g2.moveTo(lm[a].x * w, lm[a].y * h); g2.lineTo(lm[b].x * w, lm[b].y * h); }
      g2.stroke();
      for (const p of lm) { g2.beginPath(); g2.arc(p.x * w, p.y * h, 1.8, 0, Math.PI * 2); g2.fill(); }
    }
  }

  // ---- camera opt-in, and the rules that turn it off ----
  document.addEventListener("click", (e) => {
    if (e.target.closest("[data-cam]")) { if (camMode === "off") { const s = document.getElementById("camsheet"); s.hidden = false; s.classList.add("on"); } else stopCamera(); }
    if (e.target.closest("[data-camyes]")) { const s = document.getElementById("camsheet"); s.classList.remove("on"); startCamera(); }
  });
  document.addEventListener("visibilitychange", () => { if (document.hidden && camMode !== "off") stopCamera(OFF.hidden); });
  window.addEventListener("pagehide", () => { if (camMode !== "off") stopCamera(); });

  // ---- keyboard equivalents, without the camera ----
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && cancelStop()) return;
    const tgt = e.target instanceof Element ? e.target : document.body;
    // Typing in a visible field is typing; a field left focused in a closed sheet isn't.
    if (tgt.closest("input, textarea, select, .sim-pad") && tgt.offsetParent !== null) return;
    if (document.querySelector(".overlay.on")) return;
    const map = { ArrowRight: "swipe_right", ArrowLeft: "swipe_left", Enter: "point", a: "thumb", s: "fist", w: "victory" };
    if (e.key === "F" && e.shiftKey) { e.preventDefault(); stopAll(); return; }
    const g = map[e.key]; if (!g) return;
    e.preventDefault();
    const v = verb(g); record(g, v, "key");
  });

  idle();
}
