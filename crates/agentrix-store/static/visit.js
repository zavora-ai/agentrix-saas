// Agentrix visit counter (docs/06-website-experience.md §4.9), served from /_agx/visit.js on the
// site's own host. It sends the page's route (never the full URL), the referring page and the
// release seq once the page has loaded. No cookie, no identifier, nothing stored on the device
// except the visitor's own "don't count my visits" choice. Nothing is sent when the browser
// sends Global Privacy Control or the visitor said no.
//
// Include with <script src="/_agx/visit.js" data-seq="14" defer></script>.
// - Any element with data-agx-count="off" (or "on") is the visitor's switch.
// - Elements marked data-visits="on" show while visits are counted, data-visits="off" while
//   they aren't, and data-visits="gpc" when the browser sends Global Privacy Control (06 §4.9's
//   own line). Under GPC the switch and the "off" line hide when the page has a "gpc" line,
//   since the visitor's switch can't override their browser.
// - The page hears "agx-count-changed" with { counting, gpc } after every switch.
(function () {
  "use strict";
  var KEY = "agx-no-count";
  function gpc() {
    return navigator.globalPrivacyControl === true;
  }
  function counting() {
    if (gpc()) return false;
    try {
      return localStorage.getItem(KEY) !== "1";
    } catch (e) {
      return true;
    }
  }
  function paint() {
    var on = counting();
    var underGpc = gpc();
    var hasGpcLine = !!document.querySelector('[data-visits="gpc"]');
    var els = document.querySelectorAll("[data-visits]");
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      var v = el.getAttribute("data-visits");
      if (v === "on") el.hidden = !on;
      else if (v === "off") el.hidden = on || (underGpc && (hasGpcLine || el.hasAttribute("data-agx-count")));
      else if (v === "gpc") el.hidden = !underGpc;
    }
  }
  document.addEventListener("click", function (event) {
    var el = event.target && event.target.closest ? event.target.closest("[data-agx-count]") : null;
    if (!el) return;
    event.preventDefault();
    try {
      if (el.getAttribute("data-agx-count") === "off") localStorage.setItem(KEY, "1");
      else localStorage.removeItem(KEY);
    } catch (e) {}
    paint();
    document.dispatchEvent(
      new CustomEvent("agx-count-changed", { detail: { counting: counting(), gpc: gpc() } })
    );
  });
  // The script is deferred, so the page's elements are there: show the true state at once.
  paint();
  var me = document.currentScript;
  var seq = me ? parseInt(me.getAttribute("data-seq") || "0", 10) : 0;
  function send() {
    if (!counting() || !navigator.sendBeacon) return;
    var body = { r: location.pathname, ref: document.referrer };
    if (seq > 0) body.seq = seq;
    navigator.sendBeacon("/_agx/visit", JSON.stringify(body));
  }
  if (document.readyState === "complete") send();
  else window.addEventListener("load", send);
})();
