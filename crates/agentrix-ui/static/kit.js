// agentrix-ui · kit.js. The small behaviour every kit page shares. Plain ES module, loaded with
// `defer` by ui/base.html; pages work without it (links and forms post as usual).
//   - Night Time: switches the dark theme on by the site's quiet hours.
//   - Values the CSP won't let markup carry inline: meter widths and mark colours.
//   - Overlays and the approval sheet: open, close, Esc, focus return, the code fallback.
//   - Toasts: role="status", one line; server flashes too.
//   - Offline: a banner, and approvals blocked while there is no network.
// Server text is set with textContent only, never as HTML.
//
// Other islands can import it: `import { toast } from "/static/ui/kit.js";`

const root = document.documentElement;

/** Night Time: <html data-night-from="22:00" data-night-to="06:00" data-tz="Africa/Nairobi">.
 *  The server renders data-time="night" when the page loads inside quiet hours; this keeps it
 *  right while the page stays open. An owner's manual switch (the time pill) wins for the tab. */
function minutesNow(tz) {
  try {
    const parts = new Intl.DateTimeFormat("en-GB", { timeZone: tz, hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).formatToParts(new Date());
    const h = Number(parts.find((p) => p.type === "hour")?.value ?? 0);
    const m = Number(parts.find((p) => p.type === "minute")?.value ?? 0);
    return h * 60 + m;
  } catch {
    const d = new Date();
    return d.getHours() * 60 + d.getMinutes();
  }
}
function toMinutes(hhmm) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(hhmm || "");
  return m ? Number(m[1]) * 60 + Number(m[2]) : null;
}
function inQuietHours() {
  const from = toMinutes(root.dataset.nightFrom);
  const to = toMinutes(root.dataset.nightTo);
  if (from === null || to === null) return null;
  const now = minutesNow(root.dataset.tz);
  return from <= to ? now >= from && now < to : now >= from || now < to;
}
function override() {
  try { return sessionStorage.getItem("ax-time"); } catch { return null; }
}
function applyTime() {
  const forced = override();
  const night = forced ? forced === "night" : inQuietHours();
  if (night === null) return;
  if (night) root.dataset.time = "night";
  else delete root.dataset.time;
  for (const label of document.querySelectorAll("[data-time-label]")) {
    const words = night ? label.dataset.night : label.dataset.day;
    if (words) label.textContent = words;
  }
}
function toggleTime() {
  const next = root.dataset.time === "night" ? "day" : "night";
  try { sessionStorage.setItem("ax-time", next); } catch { /* private mode: this page only */ }
  if (next === "night") root.dataset.time = "night";
  else delete root.dataset.time;
  for (const label of document.querySelectorAll("[data-time-label]")) {
    const words = next === "night" ? label.dataset.night : label.dataset.day;
    if (words) label.textContent = words;
  }
}

/** Values markup can't carry under `style-src 'self'`: CSSOM writes are allowed. */
const HEX = /^#[0-9a-fA-F]{3,8}$/;
function applyValues(scope = document) {
  for (const el of scope.querySelectorAll("[data-c]")) {
    if (HEX.test(el.dataset.c)) el.style.setProperty("--c", el.dataset.c);
  }
  for (const el of scope.querySelectorAll("[data-site-tile]")) {
    if (HEX.test(el.dataset.siteTile)) el.style.setProperty("--site-tile", el.dataset.siteTile);
  }
  for (const el of scope.querySelectorAll("[data-orb-ink]")) {
    if (HEX.test(el.dataset.orbInk)) el.style.setProperty("--orb-ink", el.dataset.orbInk);
  }
  for (const bar of scope.querySelectorAll(".meter[data-value], .meter-mini[data-value]")) {
    const value = Number(bar.dataset.value);
    const max = Number(bar.dataset.max || 100);
    const fill = bar.querySelector(":scope > span, :scope > i");
    if (fill && max > 0 && Number.isFinite(value)) {
      fill.style.width = `${Math.max(0, Math.min(100, (value / max) * 100))}%`;
    }
  }
}

/** Overlays: a trigger with data-open="<id>" opens <div class="overlay" id="<id>">. */
let lastTrigger = null;
function openOverlay(id, trigger) {
  const overlay = document.getElementById(id);
  if (!overlay) return;
  lastTrigger = trigger || document.activeElement;
  overlay.classList.add("on");
  overlay.removeAttribute("hidden");
  const focusable = overlay.querySelector("[autofocus], button, [href], input, select, textarea");
  focusable?.focus();
}
function closeOverlay(overlay) {
  overlay.classList.remove("on");
  overlay.querySelector(".passkey")?.classList.remove("stepup", "signing");
  lastTrigger?.focus?.();
  lastTrigger = null;
}

/** Toasts: one line, announced politely. */
export function toast(text, { tone = "", action = null, timeout = 6000 } = {}) {
  let region = document.querySelector(".toasts");
  if (!region) {
    region = document.createElement("div");
    region.className = "toasts";
    region.setAttribute("role", "status");
    region.setAttribute("aria-live", "polite");
    document.body.append(region);
  }
  const item = document.createElement("div");
  item.className = `toast ${tone}`.trim();
  const words = document.createElement("span");
  words.textContent = text;
  item.append(words);
  if (action && action.label && typeof action.run === "function") {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "btn btn-sm";
    button.textContent = action.label;
    button.addEventListener("click", () => { action.run(); item.remove(); });
    item.append(button);
  }
  region.append(item);
  if (timeout > 0) setTimeout(() => item.remove(), timeout);
  return item;
}

/** Offline: say so, and hold approvals until the network is back. */
function applyNetwork() {
  const offline = !navigator.onLine;
  root.classList.toggle("is-offline", offline);
  for (const banner of document.querySelectorAll("[data-offline-banner]")) banner.hidden = !offline;
  for (const el of document.querySelectorAll("[data-needs-network]")) {
    if (offline) el.setAttribute("aria-disabled", "true");
    else el.removeAttribute("aria-disabled");
  }
}

function start() {
  root.classList.add("js");
  applyTime();
  setInterval(applyTime, 60_000);
  applyValues();
  applyNetwork();
  addEventListener("online", applyNetwork);
  addEventListener("offline", applyNetwork);

  // Server flashes rendered into the toast region fade after a while.
  for (const flash of document.querySelectorAll(".toasts .toast[data-flash]")) {
    setTimeout(() => flash.remove(), 8000);
  }

  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target) return;
    const opener = target.closest("[data-open]");
    if (opener) {
      event.preventDefault();
      openOverlay(opener.dataset.open, opener);
      return;
    }
    const closer = target.closest("[data-close]");
    if (closer) {
      const overlay = closer.closest(".overlay");
      if (overlay) { event.preventDefault(); closeOverlay(overlay); }
      return;
    }
    if (target.classList.contains("overlay")) { closeOverlay(target); return; }
    const stepup = target.closest("[data-stepup]");
    if (stepup) {
      event.preventDefault();
      const sheet = stepup.closest(".passkey");
      sheet?.classList.add("stepup");
      sheet?.querySelector(".stepup-code")?.focus();
      return;
    }
    const back = target.closest("[data-stepup-back]");
    if (back) {
      event.preventDefault();
      back.closest(".passkey")?.classList.remove("stepup");
      back.closest(".passkey")?.querySelector("[data-passkey]")?.focus();
      return;
    }
    const why = target.closest("[data-why]");
    if (why) {
      event.preventDefault();
      const card = why.closest(".ldec");
      const open = card?.classList.toggle("why") ?? false;
      why.setAttribute("aria-expanded", String(open));
      return;
    }
    if (target.closest("[data-time-toggle]")) { toggleTime(); return; }
    const blocked = target.closest('[data-needs-network][aria-disabled="true"]');
    if (blocked) {
      event.preventDefault();
      toast(blocked.dataset.offlineWords || "You're offline. This waits until you're back online.");
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    const open = document.querySelector(".overlay.on");
    if (open) closeOverlay(open);
  });
}

if (!window.__agentrixKit) {
  window.__agentrixKit = true;
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
}

export { applyValues, openOverlay };
