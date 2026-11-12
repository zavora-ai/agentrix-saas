// saas-web · the return page: asks the node where the subscription has got to every two seconds
// until PayPal says active and the signed webhook has booked the receipt.

const card = document.querySelector("[data-subscription]");

async function poll() {
  if (!card || card.dataset.done === "true" || !card.dataset.subscription) return;
  try {
    const res = await fetch(`/subscribe/status?subscription_id=${encodeURIComponent(card.dataset.subscription)}`, { headers: { accept: "application/json" } });
    const body = await res.json();
    card.querySelector("[data-words]").textContent = body.words || "";
    const s = body.status;
    if (s) {
      card.querySelector('[data-step="active"]').className = s.state === "active" ? "done" : "now";
      if (s.receipt) {
        card.querySelector("[data-receipt]").textContent = `PayPal ${s.receipt.paypal_id}${s.receipt.invoice_number ? ` · ${s.receipt.invoice_number}` : ""}`;
      }
    }
    if (body.done) {
      card.dataset.done = "true";
      card.classList.add("done");
      card.querySelector('[data-step="booked"]').className = "done";
      const h = document.querySelector("h1");
      if (h) h.textContent = "Your plan is active";
      return;
    }
  } catch {
    // Try again on the next tick.
  }
  setTimeout(poll, 2000);
}

setTimeout(poll, 1500);
