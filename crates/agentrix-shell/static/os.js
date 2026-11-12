// Owner console: the shell's own behaviour, on top of agentrix-ui's kit.js (which owns overlays,
// Esc, the time pill and Night Time on schedule). Worlds only change what this page shows; the
// command bar only opens pages. Nothing here decides anything: approvals are approve.js's.
import { openOverlay } from "/static/ui/kit.js";

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const os = $(".os");

// ---- Worlds: Front of house / Back office (the field's views follow the class and the time) ----
const hasWorlds = $$(".view[data-view]").length > 1;
if (os && hasWorlds) {
  for (const w of $$(".worlds [data-world]")) {
    w.addEventListener("click", (e) => {
      e.preventDefault();
      os.classList.toggle("world-front", w.dataset.world === "front");
      os.classList.toggle("world-back", w.dataset.world !== "front");
      for (const b of $$(".worlds [data-world]")) b.setAttribute("aria-pressed", String(b === w));
      history.replaceState(null, "", w.getAttribute("href"));
    });
  }
}
if (location.hash === "#activity") openOverlay("activity");

// ---- Command bar: find a page or a decision. It opens things; it never acts. ----
const form = $("[data-cmd]");
const panel = $("[data-results]");
let commands = [];
try { commands = JSON.parse(($("[data-commands]") || {}).textContent || "[]"); } catch { commands = []; }
if (form && panel) {
  const input = $("input", form);
  const list = $("[data-matches]", panel);
  const none = $("[data-nomatch]", panel);
  let matches = [];
  let sel = 0;
  const norm = (s) => s.toLowerCase().normalize("NFKD").replace(/[^\w\s]/g, "");
  const show = () => {
    const q = norm(input.value.trim());
    if (!q) { panel.classList.remove("on"); return; }
    const words = q.split(/\s+/);
    matches = commands.filter((c) => words.every((w) => norm(c.label).includes(w) || norm(c.kind).includes(w))).slice(0, 7);
    sel = 0;
    list.replaceChildren(...matches.map((c, i) => {
      const li = document.createElement("li");
      const a = document.createElement("a");
      a.href = c.href;
      a.textContent = c.label;
      const k = document.createElement("small");
      k.textContent = c.kind;
      a.append(k);
      if (i === sel) a.setAttribute("aria-selected", "true");
      li.append(a);
      return li;
    }));
    none.hidden = matches.length > 0;
    panel.classList.add("on");
  };
  const move = (d) => {
    if (!matches.length) return;
    sel = (sel + d + matches.length) % matches.length;
    $$("a", list).forEach((a, i) => a.toggleAttribute("aria-selected", i === sel));
  };
  input.addEventListener("input", show);
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); move(1); }
    if (e.key === "ArrowUp") { e.preventDefault(); move(-1); }
    if (e.key === "Escape") panel.classList.remove("on");
  });
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    show();
    if (matches[sel]) location.assign(matches[sel].href);
  });
  for (const b of $$("[data-fill]")) b.addEventListener("click", () => { input.value = b.dataset.fill; show(); input.focus(); });
  panel.addEventListener("click", (e) => { if (e.target.closest("[data-plan-close]")) panel.classList.remove("on"); });
  document.addEventListener("keydown", (e) => {
    const typing = /^(input|textarea|select)$/i.test(document.activeElement?.tagName || "");
    if (e.key === "/" && !typing && !$(".overlay.on")) { e.preventDefault(); input.focus(); }
  });
}
