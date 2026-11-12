// saas-web · the operators' activity summary, for any page: put
//   <div data-activity-summary data-src="/activity/summary"></div>
// where it goes and load this script. It fetches the panel (same origin, the operator's session)
// and fetches it again every five seconds. The panel is server-rendered and escaped.

const EVERY = 5000;

for (const box of document.querySelectorAll("[data-activity-summary]")) {
  const src = box.dataset.src || "/activity/summary";
  let busy = false;
  const load = async () => {
    if (busy || document.hidden) return;
    busy = true;
    try {
      const res = await fetch(src, { headers: { accept: "text/html" }, credentials: "same-origin" });
      if (res.status === 401) {
        box.textContent = "Sign in as an operator to see the live activity.";
        return;
      }
      if (!res.ok) throw new Error(String(res.status));
      box.innerHTML = await res.text();
    } catch {
      if (!box.firstChild) box.textContent = "The live activity didn't answer; trying again.";
    } finally {
      busy = false;
    }
  };
  load();
  setInterval(load, EVERY);
}
