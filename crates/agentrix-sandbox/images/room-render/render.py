"""room-render (S1-02; design §3.1; R6.2): renders the candidate release's pages in a sealed room
with no network and saves one screenshot per page.

Contract (move lane, agentrix-move pipeline.rs, Render job):
  /in/job.json   {files: {"<path>": "sha256:<hex>"}, pages: [{path, file, shot}], width, height}
  /in/<hex>      each file's bytes, read-only
  /out/<shot>    one PNG per page, e.g. shots/index.html.png
  stdout         {"event":"note","text":"rendered <path>"|"<path>: <error>"} per page
The room runs with --network none. The candidate is served over loopback from inside the room
(a room without a network still has 127.0.0.1), so absolute paths such as /assets/site.css
resolve as they will on the edge; anything a page tries to fetch from the outside fails, which is
what a sealed render wants.
"""

import functools
import http.server
import json
import os
import shutil
import sys
import threading

from playwright.sync_api import sync_playwright


def serve(root):
    """Serves `root` on 127.0.0.1 on a free port; returns the base URL."""
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=root)
    handler.log_message = lambda *args: None
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{httpd.server_address[1]}"

IN = "/in"
OUT = "/out"
SITE = "/tmp/site"


def note(text):
    print(json.dumps({"event": "note", "text": text[:200]}), flush=True)


def safe_join(root, rel):
    path = os.path.normpath(os.path.join(root, rel.lstrip("/")))
    if not (path == root or path.startswith(root + os.sep)):
        raise ValueError(f"path escapes the site: {rel}")
    return path


def main():
    with open(os.path.join(IN, "job.json")) as f:
        job = json.load(f)
    width = int(job.get("width", 390))
    height = int(job.get("height", 844))

    os.makedirs(SITE, exist_ok=True)
    for rel, digest in job.get("files", {}).items():
        hexd = digest.split(":", 1)[-1]
        dst = safe_join(SITE, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(os.path.join(IN, hexd), dst)

    base = serve(SITE)
    failures = 0
    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--no-sandbox", "--disable-dev-shm-usage"])
        context = browser.new_context(viewport={"width": width, "height": height})
        page = context.new_page()
        page.set_default_timeout(20000)
        for entry in job.get("pages", []):
            try:
                src = safe_join(SITE, entry["file"])
                out = safe_join(OUT, entry["shot"])
                os.makedirs(os.path.dirname(out), exist_ok=True)
                rel = os.path.relpath(src, SITE).replace(os.sep, "/")
                page.goto(f"{base}/{rel}", wait_until="load")
                page.screenshot(path=out, full_page=False)
                note(f"rendered {entry['path']}")
            except Exception as e:  # one page failing must not stop the others
                failures += 1
                note(f"{entry.get('path', '?')}: {type(e).__name__}")
        browser.close()
    return 1 if failures and failures == len(job.get("pages", [])) else 0


if __name__ == "__main__":
    sys.exit(main())
