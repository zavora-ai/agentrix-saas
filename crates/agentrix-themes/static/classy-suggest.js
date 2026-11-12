/* Classy Perfumes: suggestions under the search box while typing (C-02, search.html:371-381).
   Products come from the release's static index (/search/index.json), matched in the browser
   with no model; the last row asks the whole question. Text is set with textContent only. */
(() => {
  const input = document.getElementById("sq"), box = document.getElementById("suggest");
  if (!input || !box) return;
  const form = input.form;
  let index = null, loading = null;
  const load = () => loading || (loading = fetch("/search/index.json", { credentials: "same-origin" })
    .then(r => r.ok ? r.json() : { items: [] }).then(j => { index = j.items || []; }).catch(() => { index = []; }));
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
  const close = () => { box.classList.remove("open"); input.setAttribute("aria-expanded", "false"); };
  const marked = (text, words) => {
    const frag = document.createDocumentFragment();
    if (!words.length) { frag.appendChild(document.createTextNode(text)); return frag; }
    const re = new RegExp("(" + words.map(w => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|") + ")", "gi");
    let last = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > last) frag.appendChild(document.createTextNode(text.slice(last, m.index)));
      frag.appendChild(el("mark", "", m[0])); last = re.lastIndex;
    }
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    return frag;
  };
  const render = () => {
    const v = input.value.trim();
    if (!v || !index) { close(); return; }
    const words = v.toLowerCase().split(/\s+/).filter(w => w.length > 2);
    const hits = words.length ? index.filter(p => words.some(w => (p.name + " " + p.notes + " " + p.family).toLowerCase().includes(w))).slice(0, 4) : [];
    box.textContent = "";
    const g1 = el("div", "grp"); g1.appendChild(document.createTextNode("Products ")); g1.appendChild(el("em", "", "matched by words, without AI")); box.appendChild(g1);
    if (hits.length) {
      hits.forEach(p => {
        const a = el("a"); a.href = p.href; a.setAttribute("role", "option");
        const ph = el("span", "sw tone-" + (p.tone || "vanilla")); a.appendChild(ph);
        const name = el("span"); name.appendChild(marked(p.name, words)); a.appendChild(name);
        a.appendChild(el("small", "", " · " + p.price + (p.out ? " · sold out" : "")));
        box.appendChild(a);
      });
    } else box.appendChild(el("span", "none", "No product names match"));
    box.appendChild(el("div", "grp", "Ask"));
    const ask = el("button", "", "Ask: “" + v + "”"); ask.type = "button"; ask.setAttribute("role", "option");
    ask.addEventListener("click", () => { close(); form.submit(); });
    box.appendChild(ask);
    box.classList.add("open"); input.setAttribute("aria-expanded", "true");
  };
  input.setAttribute("aria-controls", "suggest"); input.setAttribute("aria-expanded", "false");
  input.addEventListener("focus", load, { once: true });
  input.addEventListener("input", () => { if (index) render(); else load().then(render); });
  input.addEventListener("keydown", e => { if (e.key === "Escape") close(); });
  document.addEventListener("click", e => { if (!e.target.closest(".s-search")) close(); });
})();

/* The search page's answer from the desks' model (C-02). The page arrives at once, read and
   answered by rules; when it carries data-agent, the Fragrances desk's reading and answer are
   fetched and replace it. If they don't come, the rules' results stay, and say so. The fragment
   is this site's own server-rendered markup. */
(() => {
  const body = document.getElementById("sbody");
  if (!body || !body.dataset.agent) return;
  const answer = document.getElementById("answer");
  const note = document.createElement("div"); note.className = "wa-progress"; note.setAttribute("role", "status");
  note.textContent = "Reading your question…";
  const slot = body.querySelector(".agentic");
  if (answer) answer.hidden = true;
  if (slot) slot.prepend(note);
  fetch(body.dataset.agent, { credentials: "same-origin" })
    .then(r => r.ok ? r.text() : Promise.reject(new Error(String(r.status))))
    .then(html => {
      const t = document.createElement("template"); t.innerHTML = html.trim();
      const fresh = t.content.querySelector("#sbody");
      if (!fresh) throw new Error("no body");
      body.replaceWith(fresh);
      if (window.WAMarks) window.WAMarks.draw(fresh);
    })
    .catch(() => {
      if (answer) answer.hidden = false;
      note.textContent = "These results come from rules. The desks' AI answers are paused right now.";
    });
})();
