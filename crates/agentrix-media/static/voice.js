// Talk to your Chief WebAgent (07 §3.6). Hold to talk: the microphone streams PCM16 at 16 kHz
// to node, which relays it to the Model Gateway's realtime transcription; words appear as they
// are heard. On release the turn goes to /turn, which reads it with a fixed grammar: "stop
// everything" freezes at once, "deny" waits 3 seconds for a cancel, "allow once" opens the
// approval sheet (voice never approves), anything else is a question for the Chief. The Chief's
// answer is spoken with qwen3-tts, streamed, and the Chief's mark follows the real level of the
// audio playing (ADR-031: never simulated). No audio is kept, here or on the server.

const root = document.querySelector("[data-voice]");
if (root) start(root);

function start(root) {
  const api = root.dataset.api;
  const $ = (s) => root.querySelector(s);
  const state = $("[data-vstate]"), caps = $("[data-caps]"), err = $("[data-verror]");
  const talk = $("[data-talk]"), briefBtn = $("[data-brief]"), hush = $("[data-hush]");
  const decision = $("[data-vdecision]"), bars = [...root.querySelectorAll(".voice-level i")];
  let marks = null;
  import("/_agentrix/marks.js").then((m) => { marks = m; }).catch(() => {});

  // The Chief's team mark (W1: wa-mark[team] svg.wa), else the command bar's. Voice borrows it
  // and gives back the state it had ("idle" ends a borrowing).
  const markEl = () => document.querySelector("wa-mark[team] svg.wa, .askline svg, [data-chief-mark] svg");
  let lent = null;
  function markState(s) {
    const svg = markEl();
    if (!svg) return;
    if (s !== "idle" && lent === null) lent = svg.getAttribute("data-state") || "idle";
    const to = s === "idle" ? (lent || "idle") : s;
    if (s === "idle") lent = null;
    if (marks && marks.setState) marks.setState(svg, to); else svg.setAttribute("data-state", to);
  }
  function markLevel(level) {
    const svg = markEl();
    if (svg) { if (marks && marks.setLevel) marks.setLevel(svg, level); else svg.style.setProperty("--lvl", String(level)); }
    bars.forEach((b, i) => { const k = [0.5, 0.9, 0.7, 1, 0.6, 0.8][i % 6]; b.style.height = `${8 + Math.round(24 * level * k)}px`; });
  }
  function say(words, small) { state.firstChild.textContent = words; state.querySelector("small").textContent = small || ""; }
  function problem(words) { err.textContent = words || ""; }
  function caption(who, text, cls) {
    const p = document.createElement("p");
    p.className = "cap" + (cls ? " " + cls : "");
    const b = document.createElement("b"); b.textContent = who; p.append(b, document.createTextNode(text));
    caps.append(p);
    while (caps.children.length > 8) caps.firstElementChild.remove();
    return p;
  }

  // ---- Speaking (qwen3-tts, streamed PCM at 24 kHz) ----
  let playCtx = null, analyser = null, sources = [], playAt = 0, speaking = false, raf = 0;
  function stopSpeaking() {
    sources.forEach((s) => { try { s.stop(); } catch (_) {} });
    sources = []; speaking = false; cancelAnimationFrame(raf); markLevel(0); markState("idle");
    hush.hidden = true; root.dataset.mode = "idle";
  }
  function meter() {
    if (!speaking) return;
    const data = new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(data);
    let sum = 0; for (const v of data) sum += v * v;
    markLevel(Math.min(1, Math.sqrt(sum / data.length) * 4));
    raf = requestAnimationFrame(meter);
  }
  async function speak(text) {
    stopSpeaking();
    if (!text) return;
    playCtx = playCtx || new AudioContext();
    if (playCtx.state === "suspended") await playCtx.resume();
    if (!analyser) { analyser = playCtx.createAnalyser(); analyser.fftSize = 512; analyser.connect(playCtx.destination); }
    let res;
    try {
      res = await fetch(`${api}/speak`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text }), credentials: "same-origin" });
    } catch (_) { problem("The Chief can't speak right now. The words are on screen."); return; }
    if (!res.ok) { const j = await res.json().catch(() => ({})); problem((j.error && j.error.message) || "The Chief can't speak right now. The words are on screen."); return; }
    speaking = true; hush.hidden = false; root.dataset.mode = "speaking"; markState("speaking");
    say("Speaking", "Hold to talk to interrupt");
    playAt = playCtx.currentTime + 0.05; meter();
    const reader = res.body.getReader(), dec = new TextDecoder();
    let buf = "", pending = 0, ended = false;
    const finish = () => { if (ended && pending === 0 && speaking) { stopSpeaking(); say("Ready", "Hold to talk, or read your brief"); } };
    for (;;) {
      const { value, done } = await reader.read();
      if (done || !speaking) break;
      buf += dec.decode(value, { stream: true });
      let i;
      while ((i = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, i).trim(); buf = buf.slice(i + 1);
        if (!line.startsWith("data:")) continue;
        let ev; try { ev = JSON.parse(line.slice(5)); } catch (_) { continue; }
        if (ev.type === "speech.audio.delta" && ev.audio) {
          const bytes = Uint8Array.from(atob(ev.audio), (c) => c.charCodeAt(0));
          const pcm = new Int16Array(bytes.buffer, 0, bytes.length >> 1);
          if (!pcm.length) continue;
          const ab = playCtx.createBuffer(1, pcm.length, 24000), ch = ab.getChannelData(0);
          for (let k = 0; k < pcm.length; k++) ch[k] = pcm[k] / 0x8000;
          const src = playCtx.createBufferSource(); src.buffer = ab; src.connect(analyser);
          playAt = Math.max(playAt, playCtx.currentTime); src.start(playAt); playAt += ab.duration;
          pending++; sources.push(src);
          src.onended = () => { pending--; finish(); };
        } else if (ev.type === "speech.audio.error") {
          problem("The speech stopped partway. The words are on screen.");
        }
      }
    }
    ended = true; finish();
  }

  // ---- The cards waiting in the lane ----
  const laneCards = () => [...document.querySelectorAll("[data-decision][data-card]")];
  function cardOf(el) {
    if (!el) return null;
    const chip = el.querySelector(".from .chip");
    const risk = chip ? ([...chip.classList].find((c) => c.startsWith("chip-")) || "").slice(5) : "";
    // The words beside the mark ("Orders asks"), without the mark's monogram or the risk chip.
    const fromEl = el.querySelector(".from");
    const from = fromEl ? [...fromEl.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join("").trim() : "";
    const t = el.querySelector(".t");
    return { id: el.dataset.card, title: t ? t.textContent.trim() : "this decision", risk, from, risk_words: chip ? chip.textContent.trim() : "" };
  }

  // ---- A spoken decline waits 3 seconds for a cancel ----
  let countdown = 0;
  function cancelPending(words) {
    if (!countdown) return;
    clearInterval(countdown); countdown = 0; decision.hidden = true;
    if (words) caption("Chief WebAgent", words);
  }
  // A spoken decision waits 3 seconds for a cancel, then runs: a v2 card through /decide
  // (W5's decide_spoken), a the first release card through its own decline form in the lane.
  function decideAfter(act, heard) {
    const el = document.querySelector(`[data-decision][data-card="${CSS.escape(act.card)}"]`);
    const form = el && el.querySelector("form[data-decline]");
    if (act.via !== "decide" && !form) { problem("That card isn't on this page any more."); return; }
    const allow = act.do === "allow";
    decision.hidden = false;
    $("[data-vheard]").textContent = `Heard "${heard}"`;
    $("[data-vtitle]").textContent = act.title || "";
    $("[data-vmeta]").textContent = act.meta || "";
    let left = Math.ceil((act.after_ms || 3000) / 1000);
    const tick = () => { $("[data-vcount]").textContent = `${allow ? "Allowing" : "Declining"} in ${left} s`; };
    tick();
    countdown = setInterval(async () => {
      left -= 1;
      if (left > 0) return tick();
      clearInterval(countdown); countdown = 0; decision.hidden = true;
      if (act.via !== "decide") { if (form.requestSubmit) form.requestSubmit(); else form.submit(); return; }
      try {
        const res = await fetch(`${api}/decide`, { method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", body: JSON.stringify({ card: act.card, allow, heard }) });
        const j = await res.json().catch(() => ({}));
        const words = res.ok ? j.reply : ((j.error && j.error.message) || "That didn't go through. Nothing changed.");
        caption("Chief WebAgent", words); speak(words);
        if (res.ok && el) el.remove();
      } catch (_) { problem("You're offline. Nothing was decided."); }
    }, 1000);
  }
  $("[data-vcancel]").addEventListener("click", () => cancelPending("Cancelled. Nothing changed."));

  async function handle(said) {
    const card = cardOf(laneCards()[0]);
    let reply;
    try {
      const res = await fetch(`${api}/turn`, { method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", body: JSON.stringify({ said, card, waiting: laneCards().length }) });
      reply = await res.json();
      if (!res.ok) { problem((reply.error && reply.error.message) || "That didn't go through. Try again."); return; }
    } catch (_) { problem("You're offline. Nothing was decided."); return; }
    const act = reply.action || {};
    if (act.do === "freeze") {
      const f = document.querySelector("form.freeze-form");
      if (f) { if (f.requestSubmit) f.requestSubmit(); else f.submit(); }
    } else if (act.do === "cancel") {
      cancelPending();
    } else if (act.do === "open_sheet") {
      const el = document.querySelector(`[data-decision][data-card="${CSS.escape(act.card)}"] [data-sheet]`);
      if (el) el.click();
    } else if (act.do === "decline" || act.do === "allow") {
      decideAfter(act, said);
    }
    caption("Chief WebAgent", reply.reply);
    speak(reply.reply);
  }

  // ---- Hold to talk ----
  let ws = null, micCtx = null, stream = null, node = null, held = false, ownerCap = null, finalText = "", queue = [];
  async function press(ev) {
    if (ev) ev.preventDefault();
    if (held) return;
    held = true; problem(""); stopSpeaking(); talk.setAttribute("aria-pressed", "true");
    root.dataset.mode = "listening"; markState("listening"); say("Listening", "Let go when you've finished");
    ownerCap = caption("You", "…", "owner partial"); finalText = ""; queue = [];
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    } catch (_) {
      problem("The microphone is off for this page. Allow it in your browser, or type in the command bar.");
      held = false; talk.setAttribute("aria-pressed", "false"); cleanupMic(); finished(""); return;
    }
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}${api}/live`);
    ws.binaryType = "arraybuffer";
    let ready = false;
    ws.onmessage = (m) => {
      let ev; try { ev = JSON.parse(m.data); } catch (_) { return; }
      if (ev.type === "ready") { ready = true; queue.forEach((b) => ws.send(b)); queue = []; if (!held) ws.send(JSON.stringify({ type: "commit" })); }
      else if (ev.type === "partial") { finalText += ev.text; ownerCap.lastChild.textContent = finalText; }
      else if (ev.type === "final") { finished(ev.text); }
      else if (ev.type === "error" || ev.type === "closed") { problem(ev.message); finished(""); }
    };
    ws.onerror = () => { problem("Live voice isn't available right now. Type in the command bar instead."); finished(""); };
    micCtx = new AudioContext();
    await micCtx.audioWorklet.addModule("/static/media/pcm-capture.js");
    node = new AudioWorkletNode(micCtx, "pcm-capture");
    node.port.onmessage = (m) => { if (!ws) return; if (ready && ws.readyState === 1) ws.send(m.data); else queue.push(m.data); };
    micCtx.createMediaStreamSource(stream).connect(node);
    ws.wsReady = () => ready;
    // Let go while the microphone was still starting: stop it; the commit goes once ready.
    if (!held) cleanupMic();
  }
  function cleanupMic() {
    if (stream) stream.getTracks().forEach((t) => t.stop());
    if (micCtx) micCtx.close();
    stream = null; micCtx = null; node = null;
  }
  function release(ev) {
    if (ev) ev.preventDefault();
    if (!held) return;
    held = false; talk.setAttribute("aria-pressed", "false");
    cleanupMic();
    markState("thinking"); say("Thinking", "");
    if (ws && ws.readyState === 1 && ws.wsReady && ws.wsReady()) ws.send(JSON.stringify({ type: "commit" }));
  }
  function finished(text) {
    if (ws) { try { ws.close(); } catch (_) {} ws = null; }
    const said = (text || finalText || "").trim();
    if (ownerCap) { ownerCap.classList.remove("partial"); ownerCap.lastChild.textContent = said || "(nothing heard)"; ownerCap = null; }
    root.dataset.mode = "idle"; markState("idle"); say("Ready", "Hold to talk, or read your brief");
    if (said) handle(said);
  }
  talk.addEventListener("pointerdown", press);
  talk.addEventListener("pointerup", release);
  talk.addEventListener("pointerleave", release);
  talk.addEventListener("pointercancel", release);
  talk.addEventListener("keydown", (e) => { if ((e.key === " " || e.key === "Enter") && !e.repeat) press(e); });
  talk.addEventListener("keyup", (e) => { if (e.key === " " || e.key === "Enter") release(e); });

  // ---- Typed instead of spoken: the same grammar, the same rules ----
  const typed = $("[data-vtype]");
  typed.addEventListener("submit", (e) => {
    e.preventDefault();
    const input = typed.querySelector("input"), said = input.value.trim();
    if (!said) return;
    input.value = ""; problem(""); stopSpeaking();
    caption("You", said, "owner");
    handle(said);
  });

  // ---- Read the brief aloud ----
  briefBtn.addEventListener("click", async () => {
    problem("");
    try {
      const res = await fetch(`${api}/brief`, { credentials: "same-origin" });
      const j = await res.json();
      if (!res.ok) { problem((j.error && j.error.message) || "The brief didn't load."); return; }
      caption("Chief WebAgent", j.text);
      speak(j.text);
    } catch (_) { problem("You're offline. The brief is on your Today page."); }
  });
  hush.addEventListener("click", () => { stopSpeaking(); say("Ready", "Hold to talk, or read your brief"); });
}
