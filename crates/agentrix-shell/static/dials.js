// Owner console: each job's dial on the WebAgents page (01b C12; W1's dial routes).
// Lowering applies at once and writes a receipt. Raising is a decision: the approval sheet
// opens over W1's change fingerprint, and nothing changes until the owner approves.
import { toast } from "/static/ui/kit.js";
import { openSheetData, openSheetUrl } from "/static/os/approve.js";

const LEVEL = { watch: "watch", suggest: "suggest", ask: "ask-first", tell: "do-and-tell", budget: "within-budget" };

document.addEventListener("submit", async (e) => {
  const form = e.target.closest("form[data-dial]");
  if (!form) return;
  e.preventDefault();
  const key = e.submitter && e.submitter.value;
  if (!key || !LEVEL[key] || e.submitter.getAttribute("aria-pressed") === "true") return;
  const site = form.dataset.site;
  let data = {};
  try {
    const r = await fetch(`/api/webagents/${encodeURIComponent(site)}/dials`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify({ webagent: form.dataset.webagent, job: form.dataset.job, level: LEVEL[key] }),
    });
    data = await r.json().catch(() => ({}));
    if (!r.ok) { toast(data.error || data.message || "The dial didn't move. Nothing changed; try again.", { tone: "risk" }); return; }
  } catch {
    toast("We couldn't reach your site's console. Nothing changed; try again.", { tone: "risk" });
    return;
  }
  if (data.outcome === "applied") {
    toast("Lowered now, with a receipt. Raising it again needs your approval.", { tone: "ok" });
    setTimeout(() => location.reload(), 1200);
    return;
  }
  // With card v2 mounted, a raise is a decision card: open it in the approval sheet, which
  // approves through /decisions/v2/{id}/approve; W1's runner applies the level.
  if (data.outcome === "card" && data.card && data.card.id) {
    openSheetUrl(`/console/${encodeURIComponent(site)}/cards/${encodeURIComponent(data.card.id)}/sheet`, form.closest(".job"));
    return;
  }
  if (data.outcome === "needs-approval" && data.change) {
    const c = data.change;
    openSheetData({
      title: c.words,
      sign_words: c.words,
      fingerprint: c.fingerprint,
      approve_url: `/api/webagents/${encodeURIComponent(site)}/dials/${encodeURIComponent(c.change_id)}/approve`,
      passkey_options: data.passkey ? JSON.stringify(data.passkey) : null,
      body_format: "proof",
    }, form.closest(".job"));
  }
});
