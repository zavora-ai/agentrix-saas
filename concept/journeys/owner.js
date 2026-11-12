/* Agentrix WebAgent Platform: owner-lane interactions for the dual journeys (07 §7.1).
   Illustrative only; nothing here talks to a server or a payment provider.
   journeys.js draws the chrome, tabs and living marks; console.js runs the approval sheet. */
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const reveal = sel => sel && sel.split(",").forEach(s => $$(s.trim()).forEach(el => el.classList.add("on")));
  const conceal = sel => sel && sel.split(",").forEach(s => $$(s.trim()).forEach(el => el.classList.remove("on")));

  // A step that takes a moment: <button data-next="#id" data-wait="1200" data-busy="Checking…" data-hide="#id">.
  // (Approval buttons use console.js's data-passkey + data-then instead: they reveal only after signing.)
  document.addEventListener("click", e => {
    const b = e.target.closest("[data-next]");
    if (!b || b.disabled) return;
    e.preventDefault();
    const wait = +b.dataset.wait || 0;
    const go = () => { reveal(b.dataset.next); conceal(b.dataset.hide); if (b.dataset.done) b.textContent = b.dataset.done; b.dispatchEvent(new CustomEvent("stepped", { bubbles: true })); };
    if (!wait) return go();
    const label = b.textContent;
    b.disabled = true; b.textContent = b.dataset.busy || "Working…";
    setTimeout(() => { b.disabled = false; b.textContent = b.dataset.done || label; go(); }, wait);
  });

  // Plans: one choice per group; the summary lines follow it.
  $$("[data-plans]").forEach(group => {
    const pick = btn => {
      $$(".o-plan", group).forEach(p => p.setAttribute("aria-pressed", String(p === btn)));
      $$("[data-plan-name]").forEach(el => el.textContent = btn.dataset.name);
      $$("[data-plan-price]").forEach(el => el.textContent = btn.dataset.price);
      $$("[data-plan-budget]").forEach(el => el.textContent = btn.dataset.budget);
      const r = $("[data-budget]"); if (r) { r.max = String(+btn.dataset.budget * 4); r.value = btn.dataset.budget; r.dispatchEvent(new Event("input")); }
    };
    group.addEventListener("click", e => { const b = e.target.closest(".o-plan"); if (b) pick(b); });
  });

  // The AI budget is a boundary the owner sets, not a meter.
  $$("[data-budget]").forEach(r => {
    const out = () => {
      const v = +r.value, inc = +($(".o-plan[aria-pressed='true']")?.dataset.budget || r.dataset.included || 0);
      $$("[data-budget-out]").forEach(el => el.textContent = "$" + v);
      $$("[data-budget-note]").forEach(el => el.textContent = v <= inc
        ? "Inside the included budget. Nothing above it is ever billed."
        : `$${v - inc} a month above the included budget, billed at cost plus a disclosed margin, and only up to this cap. Raising it later is a card.`);
    };
    r.addEventListener("input", out); out();
  });

  // Payment rails: the button names the provider whose page opens next.
  $$("[data-rails]").forEach(group => {
    const sync = () => {
      const c = $("input:checked", group);
      $$("[data-rail-name]").forEach(el => el.textContent = c ? c.dataset.label : "");
      $$("[data-rail-panel]").forEach(p => p.classList.toggle("on", !!c && p.dataset.railPanel === c.value));
    };
    group.addEventListener("change", sync); sync();
  });

  // Undo after go-live: one tap, with a confirm.
  document.addEventListener("click", e => {
    const u = e.target.closest("[data-undo]");
    if (!u) return;
    const card = u.closest(".o-card");
    if (card) { card.classList.add("is-undone"); card.classList.remove("is-done"); }
    reveal(u.dataset.undo);
  });

  // When console.js signs a decision, owner cards with data-decision show their next state.
  document.addEventListener("signed", e => reveal(e.detail));
})();
