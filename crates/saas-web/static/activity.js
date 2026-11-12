// saas-web · operators' live activity: new rows arrive over server-sent events from
// /activity/stream (the page's filters ride along), newest on top; the totals strip and the
// sources line update with each read.

const table = document.querySelector("[data-activity]");
const body = table?.querySelector("[data-rows]");
const live = document.querySelector("[data-live]");
const liveWords = document.querySelector("[data-live-words]");
const KEEP = 500;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined && text !== null) e.textContent = text;
  return e;
}

function chip(text, tone) {
  return el("span", `chip chip-${tone}`, text);
}

function rowOf(r) {
  const tr = el("tr", `src-${r.source}${r.refusal ? " refusal" : ""} fresh`);
  tr.id = r.anchor;
  const td = (cls, ...kids) => {
    const c = el("td", cls);
    c.append(...kids.filter(Boolean));
    tr.append(c);
    return c;
  };
  td("mono small", r.when);
  const who = el("span");
  who.append(el("b", "", r.company), r.agent ? ` · ${r.agent}` : "");
  td("", who);
  td("small", r.woke_by || "—");
  const what = el("span", "", r.what);
  td("", what, r.amount ? el("span", "mono small", ` ${r.amount}`) : null);
  td("small mono", r.usage || "");
  td("small mono", r.tool || "");
  const decision = el("span");
  if (r.decision) {
    decision.append(chip(r.decision, r.refusal ? "risk" : r.decision === "held" ? "wait" : "ok"));
    if (r.reason) decision.append(" ", el("span", "small", r.reason));
  }
  td("", decision);
  const pp = el("span");
  if (r.paypal_id) {
    const a = el("a", "", r.paypal_id);
    a.href = r.paypal_href;
    a.target = "_blank";
    a.rel = "noopener noreferrer";
    pp.append(a);
  }
  td("mono small", pp);
  td("mono small", r.era || "");
  td("small", r.checkpoint ? chip(r.checkpoint, r.checkpoint === "matches" ? "ok" : "risk") : null);
  return tr;
}

function setTotals(t) {
  const put = (k, v) => {
    const n = document.querySelector(`[data-t="${k}"]`);
    if (n) n.textContent = v;
  };
  put("money", t.money == null ? "not connected" : t.money.join(" · "));
  put("held", t.held == null ? "not connected" : String(t.held));
  put("refusals", String(t.refusals));
  put("spend", t.spend == null ? "not connected" : t.spend);
  put("cap", t.cap ? `cap ${t.cap}` : "no cap set on this node");
  put("reconciliation", t.reconciliation ? t.reconciliation.words : "none yet");
}

function setSources(list) {
  const p = document.querySelector("[data-sources]");
  if (!p) return;
  p.replaceChildren(
    ...list.map((s) => el("span", `src ${s.connected ? "ok" : "off"}`, s.connected ? s.name : `${s.name}: ${s.note}`)),
  );
}

if (table && body && window.EventSource) {
  const url = new URL(table.dataset.stream, location.href);
  url.searchParams.set("after", table.dataset.after || "0");
  const es = new EventSource(url);
  es.onopen = () => {
    live?.classList.add("on");
    if (liveWords) liveWords.textContent = "Live";
  };
  es.onerror = () => {
    live?.classList.remove("on");
    if (liveWords) liveWords.textContent = "Reconnecting…";
  };
  es.addEventListener("row", (e) => {
    const r = JSON.parse(e.data);
    body.querySelector("[data-empty]")?.remove();
    body.prepend(rowOf(r));
    while (body.rows.length > KEEP) body.lastElementChild.remove();
  });
  es.addEventListener("totals", (e) => setTotals(JSON.parse(e.data)));
  es.addEventListener("sources", (e) => setSources(JSON.parse(e.data)));
}
