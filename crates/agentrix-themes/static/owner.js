// The owner apps' one island: the site switcher and the correction drawers close on an outside
// click or Escape. Everything else is plain links and forms, so the pages work without it.
(function () {
  "use strict";
  function closeAll(except) {
    document.querySelectorAll("details.switch[open], details.more[open]").forEach(function (d) {
      if (d !== except && !d.contains(except)) d.removeAttribute("open");
    });
  }
  document.addEventListener("click", function (e) {
    var inside = e.target.closest && e.target.closest("details.switch, details.more");
    closeAll(inside);
  });
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape") closeAll(null);
  });
})();
