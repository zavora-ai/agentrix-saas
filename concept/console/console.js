/* Agentrix OS concepts: shared behaviour. Illustrative only; nothing here talks to a server. */
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];

  // Worlds (Front of house / Back office) and times (Day Time / Night Time)
  function refreshViews(os) {
    const world = os.dataset.world || "back";
    const time = os.dataset.time || "day";
    const key = world === "front" ? "front" : (time === "night" ? "night" : "back");
    $$(".view", os).forEach(v => v.classList.toggle("on", v.dataset.view === key));
    $$(".worlds button", os).forEach(b => b.setAttribute("aria-pressed", String(b.dataset.world === world)));
    $$("[data-lane]", os).forEach(l => l.hidden = l.dataset.lane !== (time === "night" ? "night" : "day"));
    const label = $("[data-time-label]", os);
    if (label) label.textContent = time === "night" ? "Night Time · until 06:00" : "Day Time · until 18:00";
  }
  $$(".os").forEach(os => {
    refreshViews(os);
    os.addEventListener("click", e => {
      const w = e.target.closest(".worlds button");
      if (w) { os.dataset.world = w.dataset.world; refreshViews(os); return; }
      if (e.target.closest("[data-time-toggle]")) { os.dataset.time = os.dataset.time === "night" ? "day" : "night"; refreshViews(os); return; }
      const go = e.target.closest("[data-go-world]");
      if (go) { os.dataset.world = go.dataset.goWorld; refreshViews(os); return; }
    });
  });

  // Freeze: one tap stops every WebAgent; the site keeps serving its signed release.
  document.addEventListener("click", e => {
    const f = e.target.closest("[data-freeze]");
    if (!f) return;
    const scope = f.closest(".os") || document.body;
    const on = !scope.classList.contains("frozen");
    scope.classList.toggle("frozen", on);
    $$("[data-freeze]", scope).forEach(b => { b.setAttribute("aria-pressed", String(on)); const t = b.querySelector("[data-freeze-text]") || b; if (b.dataset.on && b.dataset.off) t.textContent = on ? b.dataset.on : b.dataset.off; });
    $$("[data-frozen-note]", scope).forEach(n => n.hidden = !on);
    $$(".freeze-big", scope).forEach(b => b.classList.toggle("on", on));
  });

  // Overlays
  document.addEventListener("click", e => {
    const o = e.target.closest("[data-open]");
    if (o) { const el = document.getElementById(o.dataset.open); if (el) el.classList.add("on"); return; }
    const c = e.target.closest("[data-close]");
    if (c) { c.closest(".overlay")?.classList.remove("on"); return; }
    if (e.target.classList?.contains("overlay")) e.target.classList.remove("on");
  });
  document.addEventListener("keydown", e => { if (e.key === "Escape") $$(".overlay.on").forEach(o => o.classList.remove("on")); });

  // Passkey: the approval is a WebAuthn assertion over the effect's fingerprint. Without a passkey,
  // a single-use authenticator code is bound to the same fingerprint (02 §8).
  let pending = null;
  function resetStepup(box) {
    const b = $(".sig b", box); if (b) b.textContent = "Your passkey signs:";
    const code = $(".stepup-code", box); if (code) { code.value = ""; code.classList.remove("bad"); code.placeholder = "6 digits"; }
  }
  document.addEventListener("click", e => {
    const p = e.target.closest("[data-passkey]");
    if (p) {
      const ov = document.getElementById(p.dataset.passkey || "passkey");
      if (!ov) return;
      pending = p;
      const box = $(".passkey", ov);
      box.classList.remove("signing", "signed", "stepup");
      resetStepup(box);
      $("[data-sign-what]", ov).textContent = p.dataset.sign || "This exact action";
      $("[data-sign-fp]", ov).textContent = p.dataset.fp || "H(site, webagent, tool, args, amount, expiry)";
      ov.classList.add("on");
      return;
    }
    const up = e.target.closest("[data-stepup]");
    if (up) {
      const box = up.closest(".passkey");
      box.classList.add("stepup");
      const b = $(".sig b", box); if (b) b.textContent = "Your code approves:";
      $(".stepup-code", box)?.focus();
      return;
    }
    if (e.target.closest("[data-stepup-back]")) { const box = e.target.closest(".passkey"); box.classList.remove("stepup"); resetStepup(box); return; }
    const s = e.target.closest("[data-do-sign]");
    if (s) {
      const box = s.closest(".passkey");
      const byCode = s.dataset.method === "code";
      if (byCode) {
        const code = $(".stepup-code", box);
        if (!/^\d{6}$/.test(code.value.trim())) { code.classList.add("bad"); code.placeholder = "Enter the 6-digit code"; code.focus(); return; }
      }
      const ok = $(".ok", box);
      if (ok) { ok.dataset.base = ok.dataset.base || ok.textContent; ok.textContent = byCode ? "Approved with an authenticator code · runs once · the receipt names the method" : ok.dataset.base; }
      box.classList.add("signing");
      setTimeout(() => {
        box.classList.remove("signing"); box.classList.add("signed");
        setTimeout(() => {
          box.closest(".overlay").classList.remove("on");
          if (pending) {
            const card = pending.closest("[data-decision]");
            if (card) card.classList.add("done");
            const line = card && $(".done-line", card);
            if (line && byCode) line.textContent = line.textContent.replace(/signed( with your passkey)?( on [^·.]*[^·.\s])?/i, "with an authenticator code");
            if (pending.dataset.then) document.dispatchEvent(new CustomEvent("signed", { detail: pending.dataset.then }));
            const os = pending.closest(".os");
            const n = os && $(".pill.trust .n", os);
            if (card && n) { const v = Math.max(0, (+n.textContent || 0) - 1); n.textContent = v; if (!v) n.hidden = true; }
          }
          pending = null;
        }, 900);
      }, 1000);
    }
  });

  // Lane verbs that need no passkey
  document.addEventListener("click", e => {
    const w = e.target.closest("[data-why]");
    if (w) { w.closest(".ldec, .fcard")?.classList.toggle("why"); return; }
    const d = e.target.closest("[data-dismiss]");
    if (d) { const c = d.closest("[data-decision]"); if (c) { c.classList.add("done"); const l = $(".done-line", c); if (l && d.dataset.dismiss) l.textContent = d.dataset.dismiss; } }
  });

  // The command bar shows what will happen before it happens.
  $$(".askline input").forEach(inp => {
    const plan = inp.closest(".cmd")?.querySelector(".plan");
    if (!plan) return;
    const show = () => plan.classList.toggle("on", inp.value.trim().length > 3);
    inp.addEventListener("input", show);
    inp.addEventListener("keydown", e => { if (e.key === "Enter") { plan.classList.add("on"); } });
    $$("[data-fill]", inp.closest(".os") || document).forEach(b => b.addEventListener("click", () => { inp.value = b.dataset.fill; show(); inp.focus(); }));
  });
  document.addEventListener("click", e => {
    const c = e.target.closest("[data-plan-close]");
    if (c) c.closest(".plan").classList.remove("on");
  });

  // Ladders (autonomy per job)
  document.addEventListener("click", e => {
    const b = e.target.closest(".ladder button");
    if (!b || b.disabled) return;
    // Lowering is immediate; raising is always a passkey-signed card.
    const l = b.closest(".ladder");
    const btns = $$("button", l);
    const cur = btns.findIndex(x => x.getAttribute("aria-pressed") === "true");
    const next = btns.indexOf(b);
    if (next === cur) return;
    const out = l.parentElement.querySelector("[data-ladder-note]");
    if (next < cur || l.closest(".store-os")) {
      btns.forEach(x => x.setAttribute("aria-pressed", String(x === b)));
      l.classList.remove("lowered");
      if (out) out.textContent = l.closest(".store-os") ? "Starting level set. It is signed with everything else when you add Sales." : "Lowered now, no signature needed. Raising it again needs your passkey.";
    } else if (out) {
      out.textContent = b.dataset.note || "Raising needs your passkey: a card is waiting in your lane. Nothing changes until you sign.";
    }
  });

  // The team mark's amber dot follows delivered cards in Needs you (07 §3.2, C13). It shows on the
  // marks the owner looks to (command bar, phone header, dock), not on every card that quotes it.
  // Screens without live decision cards (the phone mock-ups) keep the dot their markup sets.
  function syncTeamMark() {
    $$(".os").forEach(scope => {
      const cards = $$("[data-decision]", scope);
      if (!cards.length) return;
      const waiting = cards.some(c => !c.classList.contains("done") && !c.closest("[hidden]"));
      $$(".cmd .orb, .app-ico .orb", scope).forEach(o => o.toggleAttribute("data-waiting", waiting));
    });
  }
  syncTeamMark();
  new MutationObserver(syncTeamMark).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["class", "hidden"] });

  // Receipts: verify
  document.addEventListener("click", e => {
    const v = e.target.closest("[data-verify]");
    if (v) { e.preventDefault(); v.closest(".receipt")?.classList.toggle("open"); }
  });
})();
