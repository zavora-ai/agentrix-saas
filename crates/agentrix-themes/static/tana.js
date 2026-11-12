/* Tana Valley Times: Catch me up with device memory (news.html, 01a N-01), section follows and
   the digest contract's "You get" row. Memory lives in this browser's localStorage only, and
   only after the reader taps "Remember"; it is sent with a request only when she asks to catch
   up, as words in the question. Nothing is stored on the server. */
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const KEY = "tvt-memory";
  const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const toast = msg => window.WADesks && window.WADesks.toast ? window.WADesks.toast(msg) : null;
  const load = () => { try { return JSON.parse(localStorage.getItem(KEY) || "null"); } catch { return null; } };
  const store = m => { try { if (m) localStorage.setItem(KEY, JSON.stringify(m)); else localStorage.removeItem(KEY); } catch {} };
  const iso = d => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const parse = s => { const [y, m, d] = String(s).split("-").map(Number); return new Date(y, (m || 1) - 1, d || 1); };
  const label = s => { const d = parse(s); return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}`; };
  const short = s => { const d = parse(s); return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`; };
  const today = iso(new Date());
  const daysAgo = s => Math.round((parse(today) - parse(s)) / 86400000);

  // A remembered device notes the day of each visit; "Last visit" is the one before today.
  let mem = load();
  if (mem && mem.seen && mem.seen !== today) { mem.last = mem.seen; mem.seen = today; store(mem); }
  const saved = "Saved in this browser only. Sent with a request only when you ask to catch up.";

  // ---------- Catch me up (front page) ----------
  const box = $("#catch");
  if (box) {
    const dev = $("#devmem"), thread = $("#cmThread"), btn = $("#cmBtn"), all = $("#cmAll");
    const mark = () => $(".catch-top .wa-living", box);
    const chip = (text, cls = "") => { const s = document.createElement("span"); s.className = "memchip " + cls; s.textContent = text; return s; };
    const link = (text, on) => { const b = document.createElement("button"); b.type = "button"; b.className = "lnk"; b.textContent = text; b.addEventListener("click", on); return b; };
    const sinceWords = () => {
      if (!mem || !mem.last) return null;
      const n = daysAgo(mem.last);
      return n > 0 && n < 7 ? LONG[parse(mem.last).getDay()] : short(mem.last);
    };
    function render() {
      dev.textContent = "";
      if (mem) {
        const lead = document.createElement("span"); lead.textContent = "Remembered on this device:"; dev.appendChild(lead);
        if (mem.last) dev.appendChild(chip(`Last visit · ${short(mem.last)}`));
        (mem.follows || []).forEach(f => dev.appendChild(chip(`Follows · ${f}`)));
        if (!(mem.follows || []).length) { const h = document.createElement("span"); h.textContent = "Follow a section on its page and catch-ups focus on it."; dev.appendChild(h); }
        if (mem.sample) dev.appendChild(chip("Sample reader's memory", "sample"));
        dev.appendChild(link("Forget this device", () => { mem = null; store(null); render(); toast("Forgotten. Nothing about you was stored on our side."); }));
      } else {
        const lead = document.createElement("span"); lead.textContent = "Nothing remembered on this device."; dev.appendChild(lead);
        dev.appendChild(link("Remember my topics here", () => { mem = { seen: today, last: null, follows: [] }; store(mem); render(); toast(saved); }));
        const sl = box.dataset.sampleLast, sf = (box.dataset.sampleFollows || "").split("|").filter(Boolean);
        if (sl) dev.appendChild(link("Try the sample reader's memory", () => { mem = { seen: today, last: sl, follows: sf, sample: true }; store(mem); render(); toast(saved); }));
      }
      const w = sinceWords();
      btn.textContent = w ? `Catch me up since ${w}` : "Catch me up on this week";
    }
    render();
    const ask = async q => {
      if (!window.WADesks) return;
      thread.textContent = ""; btn.disabled = true; all.disabled = true;
      try { await window.WADesks.ask(thread, q, "newsroom", mark()); } finally { btn.disabled = false; all.disabled = false; }
    };
    btn.addEventListener("click", () => {
      const follows = mem && mem.follows && mem.follows.length ? `, following ${mem.follows.join(" and ")}` : "";
      ask(mem && mem.last ? `Catch me up since ${label(mem.last)}${follows}` : `Catch me up on this week${follows}`);
    });
    all.addEventListener("click", () => ask("What's on today's front page?"));
  }

  // ---------- Follow a section (section fronts) ----------
  const follow = $("#followBtn");
  if (follow) {
    const name = follow.dataset.section;
    const paint = () => {
      const on = !!(mem && (mem.follows || []).includes(name));
      follow.textContent = on ? `Following ${name} · stop` : `Follow ${name} on this device`;
      follow.setAttribute("aria-pressed", String(on));
    };
    follow.addEventListener("click", () => {
      if (!mem) { mem = { seen: today, last: null, follows: [] }; toast(saved); }
      const list = mem.follows || [];
      mem.follows = list.includes(name) ? list.filter(f => f !== name) : [...list, name];
      store(mem); paint();
    });
    paint();
  }

  // ---------- Digest: the "You get" row follows the chosen topics ----------
  document.querySelectorAll("form[data-digest]").forEach(form => {
    const row = form.querySelector("[data-template]");
    const boxes = [...form.querySelectorAll('input[type="checkbox"]')];
    if (mem && mem.follows && !boxes.some(b => b.checked)) boxes.forEach(b => { if (mem.follows.includes(b.dataset.name)) b.checked = true; });
    const paint = () => {
      if (!row) return;
      const chosen = boxes.filter(b => b.checked).map(b => b.dataset.name);
      row.textContent = row.dataset.template.replace("{topics}", chosen.length ? chosen.join(" and ").toLowerCase() : "front-page");
    };
    boxes.forEach(b => b.addEventListener("change", paint));
    paint();
  });
})();
