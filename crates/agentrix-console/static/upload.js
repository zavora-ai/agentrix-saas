/* Agentrix · the owner's WordPress export for the Move (R4.1).
   Each file goes straight to PUT /sites/{id}/uploads/{name}, which checks it is what its name
   says (XML export, gzip archives) before keeping it. Nothing here guesses: the status line
   says what the server said. */
(() => {
  const root = document.getElementById("uploads");
  if (!root) return;
  const site = root.dataset.site;
  const types = {
    "export.xml": "application/xml",
    "dump.sql.gz": "application/gzip",
    "wp-content.tar.gz": "application/gzip",
  };
  root.querySelectorAll("input[type=file][data-name]").forEach((input) => {
    input.addEventListener("change", async () => {
      const file = input.files && input.files[0];
      const name = input.dataset.name;
      const status = document.getElementById(`${input.id}-status`);
      const say = (text) => { if (status) status.textContent = text; };
      if (!file || !types[name]) return;
      say(`Uploading ${file.name} (${Math.max(1, Math.round(file.size / 1024))} KB)…`);
      input.disabled = true;
      try {
        const r = await fetch(`/sites/${encodeURIComponent(site)}/uploads/${name}`, {
          method: "PUT",
          headers: { "content-type": types[name], accept: "application/json" },
          body: file,
          credentials: "same-origin",
        });
        const d = await r.json().catch(() => ({}));
        if (!r.ok) {
          say(d.message || `That didn't upload (${r.status}).`);
          input.disabled = false;
          return;
        }
        say("Uploaded and checked.");
        setTimeout(() => location.reload(), 700);
      } catch (_) {
        say("The upload didn't finish. Check your connection and try again.");
        input.disabled = false;
      }
    });
  });
})();
