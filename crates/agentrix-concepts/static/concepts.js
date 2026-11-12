// "Choose your new look": follows the run over SSE, lights the pipeline strip, fills the three
// columns in, and refreshes the Challenge 1 evidence from the run's records.
(function () {
  "use strict";
  var cols = document.getElementById("c-cols");
  var words = document.getElementById("c-words");
  var pipe = document.getElementById("c-pipe");
  var reqs = document.getElementById("c-reqs");
  var ORDER = ["facts", "plan", "agents", "check", "choose", "move"];

  function lightPipe(stage) {
    if (!pipe) return;
    var cur = ORDER.indexOf(stage);
    pipe.dataset.stage = stage;
    pipe.querySelectorAll(".c-stepbox").forEach(function (el, i) {
      el.classList.toggle("is-done", i < cur || (stage === "move" && i <= cur));
      el.classList.toggle("is-on", i === cur && stage !== "move");
      if (i === cur) el.setAttribute("aria-current", "step");
      else el.removeAttribute("aria-current");
    });
  }
  if (pipe) lightPipe(pipe.dataset.stage || "facts");

  document.querySelectorAll(".c-phone").forEach(function (b) {
    b.addEventListener("click", function () {
      var col = document.getElementById("c-col-" + b.dataset.n);
      var on = col.classList.toggle("is-phone");
      b.setAttribute("aria-pressed", on ? "true" : "false");
    });
  });

  function chip(text, kind) {
    var s = document.createElement("span");
    s.className = "c-chip" + (kind ? " " + kind : "");
    s.textContent = text;
    return s;
  }

  function facts(a) {
    var box = document.getElementById("c-facts-" + a.n);
    if (!box) return;
    box.textContent = "";
    if (a.digest) box.appendChild(chip("sha256 " + a.digest.slice(0, 10), "c-mono"));
    if (a.writes) box.appendChild(chip(a.writes + " save" + (a.writes === 1 ? "" : "s")));
    Object.keys(a.refused || {}).forEach(function (k) {
      box.appendChild(chip(k + " refused ×" + a.refused[k], "c-warn"));
    });
    if (a.rounds) box.appendChild(chip(a.rounds + " fix round" + (a.rounds === 1 ? "" : "s")));
    (a.missing || []).forEach(function (m) { box.appendChild(chip("missing: " + m, "c-warn")); });
  }

  var evTimer = null;
  function refreshEvidence() {
    if (!reqs) return;
    fetch("/sites/" + encodeURIComponent(reqs.dataset.site) + "/concepts/evidence", { credentials: "same-origin" })
      .then(function (r) { return r.ok ? r.text() : null; })
      .then(function (html) { if (html !== null) reqs.innerHTML = html; })
      .catch(function () {});
  }

  if (!cols) return;
  var site = cols.dataset.site;
  var state = cols.dataset.state;
  if (state !== "planning" && state !== "designing" && state !== "revising") return;
  evTimer = setInterval(refreshEvidence, 3000);
  var shown = {};
  var lastDraft = {};
  var es = new EventSource("/sites/" + encodeURIComponent(site) + "/concepts/live");
  es.onmessage = function (ev) {
    var s;
    try { s = JSON.parse(ev.data); } catch (e) { return; }
    if (words && s.words) words.textContent = s.words;
    if (s.stage) lightPipe(s.stage);
    var agents = s.agents || [];
    if (agents.length && String(agents.length) !== cols.dataset.agents) {
      es.close();
      location.reload();
      return;
    }
    agents.forEach(function (a) {
      var col = document.getElementById("c-col-" + a.n);
      if (!col) return;
      col.dataset.status = a.status;
      var st = document.getElementById("c-state-" + a.n);
      if (st) st.textContent = a.words;
      var tail = document.getElementById("c-tail-" + a.n);
      if (tail) {
        tail.textContent = a.tail || "";
        tail.hidden = !a.tail;
        tail.scrollTop = tail.scrollHeight;
      }
      facts(a);
      var bar = document.getElementById("c-bar-" + a.n);
      if (bar) bar.style.width = a.status === "done" && !a.busy ? "100%" : Math.min(95, a.chars / 400) + "%";
      var frame = document.getElementById("c-frame-" + a.n);
      var busy = a.status !== "done" || a.busy;
      ["c-choose-", "c-guide-", "c-send-"].forEach(function (p) {
        var el = document.getElementById(p + a.n);
        if (el) el.disabled = busy;
      });
      var key = a.digest + ":" + a.status;
      if (a.status === "done" && !a.busy && shown[a.n] !== key) {
        shown[a.n] = key;
        frame.src = "/sites/" + site + "/concepts/" + a.concept_id + "/preview?t=" + Date.now();
      } else if (busy && a.chars > 400) {
        var now = Date.now();
        if (!lastDraft[a.n] || now - lastDraft[a.n] > 4000) {
          lastDraft[a.n] = now;
          frame.src = "/sites/" + site + "/concepts/" + a.concept_id + "/draft?t=" + now;
        }
      }
    });
    if (s.state === "done" || s.state === "failed") {
      es.close();
      clearInterval(evTimer);
      refreshEvidence();
      setTimeout(function () { location.reload(); }, 1500);
    }
  };
})();
