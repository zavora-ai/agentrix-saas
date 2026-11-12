/* Agentrix onboarding · the plan island (W4): pick a plan, how often to pay, and the AI budget
   as a limit. Prices come from the page (W3's catalogue, with its one 12-month discount); this
   only moves the pressed card, the hidden plan_id, the prices shown and the budget note. The
   form posts as it is, so it works without this. */
(() => {
  const form = document.getElementById("plan");
  if (!form || !document.getElementById("budget")) return;
  const $$ = s => [...document.querySelectorAll(s)];
  const range = document.getElementById("budget");
  const note = document.getElementById("budget-note");
  const out = document.getElementById("budget-out");
  const pressed = () => document.querySelector('.o-plan[aria-pressed="true"]');
  const annual = () => form.querySelector('input[name="period"]:checked')?.value === "annual";
  function budget() {
    const v = +range.value, inc = +(pressed()?.dataset.budget || 0);
    out.textContent = `$${v}`;
    note.textContent = v <= inc
      ? "Inside the included budget. Nothing above your limit is ever billed."
      : `$${v - inc} a month above the included budget, billed at cost plus a disclosed margin, and only up to this cap. Raising it later is a card.`;
  }
  function prices() {
    const d = pressed()?.dataset;
    if (!d) return;
    const set = (sel, text) => $$(sel).forEach(el => el.textContent = text);
    set("[data-plan-price]", d.price);
    set("[data-plan-annual]", d.annual);
    set("[data-plan-year]", d.year);
    set("[data-plan-saving]", d.saving);
    set("[data-plan-charge]", annual() && d.annual ? `${d.annual} for 12 months` : `${d.price} a month`);
  }
  function pick(btn) {
    $$(".o-plan").forEach(p => p.setAttribute("aria-pressed", String(p === btn)));
    $$("[data-plan-name]").forEach(el => el.textContent = btn.dataset.name);
    $$("[data-plan-budget]").forEach(el => el.textContent = btn.dataset.budget);
    document.getElementById("plan-id").value = btn.dataset.id;
    range.max = String(+btn.dataset.budget * 4 || 20); range.value = btn.dataset.budget;
    prices();
    budget();
  }
  $$(".o-plan").forEach(b => b.addEventListener("click", () => pick(b)));
  $$('input[name="period"]').forEach(r => r.addEventListener("change", prices));
  range.addEventListener("input", budget);
})();
