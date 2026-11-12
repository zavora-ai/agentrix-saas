// The hero film plays muted on its own, unless the reader has asked for less motion or to save
// data. The button pauses it and plays it again (WCAG 2.2.2). Without this script the poster stays.
(() => {
  const figure = document.querySelector(".hero-film");
  const video = figure && figure.querySelector("video");
  const button = figure && figure.querySelector(".film-toggle");
  if (!video || !button) return;
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches || (navigator.connection && navigator.connection.saveData);
  const show = () => {
    const playing = !video.paused;
    button.querySelector("span").textContent = playing ? "Pause" : "Play";
    button.setAttribute("aria-label", playing ? "Pause the film" : "Play the film");
    button.classList.toggle("is-playing", playing);
  };
  video.addEventListener("play", show);
  video.addEventListener("pause", show);
  button.addEventListener("click", () => (video.paused ? video.play().catch(show) : video.pause()));
  button.hidden = false;
  show();
  if (!still) video.play().catch(show);
})();
