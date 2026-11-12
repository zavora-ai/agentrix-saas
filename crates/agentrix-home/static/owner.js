// Screen 5: the owner console as a walkthrough. Without this script each point shows its own
// screenshot under it. With it, the four screenshots share one frame beside the points: the
// frame moves on every 4 seconds with a crossfade while the section is on screen, the point it
// shows is marked, and a click or a key on a point shows that one. Hover or focus pauses it;
// with reduced motion it never moves by itself.
(() => {
  const list = document.querySelector("[data-walk]");
  const wrap = list && list.closest(".owner");
  if (!wrap) return;
  const ways = [...list.querySelectorAll(".owner-way")];
  const frames = ways.map((w) => w.querySelector(".owner-frame"));
  if (ways.length < 2 || frames.some((f) => !f)) return;

  const stage = document.createElement("div");
  stage.className = "owner-stage";
  frames.forEach((f) => stage.append(f));
  // Beside the points on a wide screen; on a narrow one, right under them.
  const wide = matchMedia("(min-width: 960px)");
  const place = () => (wide.matches ? wrap.append(stage) : list.after(stage));
  place();
  wide.addEventListener("change", place);
  wrap.classList.add("walk-on");

  // Each point's title becomes the button that shows its screenshot.
  const picks = ways.map((w, i) => {
    const title = w.querySelector("b");
    const pick = document.createElement("button");
    pick.type = "button";
    pick.className = "owner-pick";
    pick.setAttribute("aria-controls", frames[i].id);
    pick.append(...title.childNodes);
    title.append(pick);
    pick.addEventListener("click", () => show(i));
    w.addEventListener("click", (e) => { if (!e.target.closest("a, button")) show(i); });
    return pick;
  });

  let at = -1;
  function show(i) {
    at = i;
    ways.forEach((w, j) => w.classList.toggle("on", j === i));
    picks.forEach((p, j) => p.setAttribute("aria-pressed", String(j === i)));
    frames.forEach((f, j) => {
      f.classList.toggle("on", j === i);
      f.setAttribute("aria-hidden", String(j !== i));
    });
  }
  show(0);

  const still = matchMedia("(prefers-reduced-motion: reduce)");
  let hovered = false, focused = false, seen = false, timer = 0;
  function run() {
    clearInterval(timer);
    timer = 0;
    if (still.matches || hovered || focused || !seen || document.hidden) return;
    timer = setInterval(() => show((at + 1) % ways.length), 4000);
  }
  wrap.addEventListener("mouseenter", () => { hovered = true; run(); });
  wrap.addEventListener("mouseleave", () => { hovered = false; run(); });
  wrap.addEventListener("focusin", () => { focused = true; run(); });
  wrap.addEventListener("focusout", (e) => { if (!wrap.contains(e.relatedTarget)) { focused = false; run(); } });
  document.addEventListener("visibilitychange", run);
  still.addEventListener("change", run);
  if ("IntersectionObserver" in window) {
    new IntersectionObserver((seenNow) => { seen = seenNow[0].isIntersecting; run(); }, { threshold: 0.4 }).observe(stage);
  } else {
    seen = true;
    run();
  }
})();
