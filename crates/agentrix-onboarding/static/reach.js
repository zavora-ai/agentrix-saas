/* Agentrix onboarding · "Where should your WebAgents reach you?" (W4). Not now hides the step
   for this site on this device; it's a convenience, so nothing on the server changes. */
(() => {
  const card = document.getElementById("reach");
  if (!card) return;
  const key = `agentrix.reach.later.${card.dataset.site}`;
  try { if (localStorage.getItem(key)) { card.hidden = true; return; } } catch (_) { /* storage off: show it */ }
  const later = document.getElementById("reach-later");
  if (later) later.addEventListener("click", () => {
    card.hidden = true;
    try { localStorage.setItem(key, "1"); } catch (_) { /* storage off: hidden until reload */ }
  });
})();
