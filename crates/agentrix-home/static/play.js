// The 2:24 film on screen 3 plays only when someone taps Play: the poster becomes the player,
// with controls. The chapter times then jump to their moment. Without this script, Play opens
// /film, which has the film and everything that happens in it written out.
(() => {
  const box = document.querySelector(".film-box[data-film]");
  if (!box) return;
  let video = null;
  const open = () => {
    if (video) return video;
    video = document.createElement("video");
    video.controls = true;
    video.playsInline = true;
    video.preload = "auto";
    video.src = box.dataset.film;
    video.poster = box.querySelector("img").src;
    video.setAttribute("aria-label", "The same Saturday, twice: a sample story made with AI");
    box.querySelector("img").replaceWith(video);
    box.querySelector(".film-play").remove();
    return video;
  };
  box.querySelector(".film-play").addEventListener("click", e => {
    e.preventDefault();
    open().play().catch(() => {});
  });
  document.querySelectorAll(".chapter-at[data-at]").forEach(span => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "chapter-jump";
    b.textContent = span.textContent;
    b.setAttribute("aria-label", `Play from ${span.textContent}`);
    b.addEventListener("click", () => {
      const v = open();
      v.currentTime = Number(span.dataset.at);
      v.play().catch(() => {});
      box.scrollIntoView({ block: "center" });
    });
    span.replaceWith(b);
  });
})();
