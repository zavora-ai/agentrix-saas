"""room-fetch-browser (S1-02, S1-10; design §3.1; R1.2, R1.4): the room-fetch contract with a real
browser (Playwright's Firefox), so each page also yields a sealed screenshot and its DOM after
scripts ran.

Contract (Assess lane, assess-room-fetch-contract.md); the same as room-fetch/fetch.sh plus two
outputs:
  /in/job.json       {task:"fetch", urls:[...], user_agent, delay_ms, timeout_ms, max_bytes,
                      max_redirects, screenshots}
  /out/fetch.json    {results:[{i, url, final_url, status|null, content_type, bytes, truncated,
                      ms, body:"bodies/<i>"|null, error:null|"words", rendered?}]}, one per URL
  /out/bodies/<i>    the raw body as served, cut at max_bytes
  /out/rendered/<i>.html  the DOM after scripts ran (HTML pages, when screenshots is true)
  stdout             {"event":"note","text":"fetched <i> <status>|error"} after each URL, then
                     {"event":"frame","b64":<JPEG>} for HTML pages when screenshots is true

Control has applied robots.txt and the page budget; the room fetches exactly the URLs it is
given, in order, delay_ms apart. The only way out is the job's egress proxy (HTTPS_PROXY), which
enforces the allowlist and records every request; loopback goes through it too, so a page that
reaches for 127.0.0.1 is refused and recorded.

Why Firefox: agentrix-egress refuses ClientHellos carrying ECH (S2-09). Chromium 130 (Playwright
1.48) still sends its GREASE ECH extension with --disable-features=EncryptedClientHello and with
the EncryptedClientHelloEnabled policy (tested on the sandbox, 7 Nov), so every HTTPS page was
refused. Firefox turns ECH off by preference.

/out/fetch.json always exists (R1.6): it is rewritten after every URL, so a room killed at its
deadline still reports what it fetched, and any URL not reached gets an error entry.
"""

import base64
import json
import os
import sys
import time
from urllib.parse import urlparse

IN = os.environ.get("ROOM_IN", "/in")
OUT = os.environ.get("ROOM_OUT", "/out")
DEFAULT_UA = "Mozilla/5.0 (compatible; AgentrixBot/1.0; +https://agentrix.website/bot)"
NOT_PAGES = (".txt", ".xml", ".json", ".pdf", ".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg")

results = []


def emit(event, **fields):
    sys.stdout.write(json.dumps({"event": event, **fields}) + "\n")
    sys.stdout.flush()


def manifest(urls):
    """Writes /out/fetch.json atomically: what was fetched, then an entry for every URL not."""
    rows = list(results)
    for i in range(len(rows), len(urls)):
        rows.append({"i": i, "url": urls[i], "status": None, "bytes": 0, "truncated": False,
                     "body": None, "error": "the room stopped before fetching this"})
    tmp = f"{OUT}/.fetch.json.tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"results": rows}, f)
    os.replace(tmp, f"{OUT}/fetch.json")


# Firefox preferences: no ECH (the proxy refuses it), no DNS over HTTPS (the proxy resolves),
# and loopback through the proxy like everything else.
FIREFOX_PREFS = {
    "security.tls.ech.grease_probability": 0,
    "network.dns.echconfig.enabled": False,
    "network.dns.http3_echconfig.enabled": False,
    "network.trr.mode": 5,
    "network.proxy.allow_hijacking_localhost": True,
    "network.proxy.no_proxies_on": "",
    "browser.safebrowsing.malware.enabled": False,
    "browser.safebrowsing.phishing.enabled": False,
    "network.captive-portal-service.enabled": False,
    "network.connectivity-service.enabled": False,
}


def words(error, url):
    """An error in the words the report looks for: refused, dns, tls, timed out."""
    e = str(error).lower()
    host = urlparse(url).hostname or url
    if "proxy_connection_refused" in e or "econnrefused" in e:
        return "fetch failed: the room's proxy did not answer"
    if "proxy_forbidden" in e or "tunnel_connection_failed" in e or "403" in e \
            or "refused" in e or "blocked" in e:
        return f"refused by the egress proxy ({host})"
    if "unknown_host" in e or "name_not_resolved" in e or "resolve" in e or "dns" in e:
        return "dns: could not resolve the host"
    if "cert" in e or "ssl" in e or "tls" in e or "sec_error" in e:
        return "tls: certificate or handshake failed"
    if "timeout" in e or "timed out" in e:
        return "timed out"
    return "fetch failed: " + str(error).splitlines()[0][:160]


def is_page(url):
    return not urlparse(url).path.lower().endswith(NOT_PAGES)


def record(i, url, *, final=None, status=None, ctype=None, raw=b"", max_bytes, ms, error=None,
           rendered=None):
    entry = {"i": i, "url": url, "final_url": final or url, "status": status,
             "content_type": ctype or "", "bytes": 0, "truncated": False, "ms": ms,
             "body": None, "error": error}
    if status is not None:
        kept = raw[:max_bytes]
        with open(f"{OUT}/bodies/{i}", "wb") as f:
            f.write(kept)
        entry.update(bytes=len(kept), truncated=len(raw) > max_bytes, body=f"bodies/{i}")
        if rendered is not None:
            with open(f"{OUT}/rendered/{i}.html", "w", encoding="utf-8") as f:
                f.write(rendered[: max_bytes * 2])
            entry["rendered"] = f"rendered/{i}.html"
    results.append(entry)


def main():
    args = json.load(open(f"{IN}/job.json", encoding="utf-8"))
    urls = [str(u) for u in args.get("urls", [])]
    os.makedirs(f"{OUT}/bodies", exist_ok=True)
    os.makedirs(f"{OUT}/rendered", exist_ok=True)
    manifest(urls)
    if not urls:
        return 0
    ua = args.get("user_agent") or DEFAULT_UA
    delay = max(0, int(args.get("delay_ms", 500))) / 1000.0
    timeout = max(1000, int(args.get("timeout_ms", 15000)))
    max_bytes = max(1, int(args.get("max_bytes", 2 * 1024 * 1024)))
    max_redirects = max(0, int(args.get("max_redirects", 5)))
    screenshots = bool(args.get("screenshots", False))
    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("https_proxy")
    if not proxy:
        for i, url in enumerate(urls):
            record(i, url, max_bytes=max_bytes, ms=0,
                   error="refused: the room has no egress proxy, so it fetched nothing")
            emit("note", text=f"fetched {i} error")
        return 0

    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        api = p.request.new_context(
            proxy={"server": proxy}, user_agent=ua, ignore_https_errors=False)
        browser = context = None
        if screenshots:
            browser = p.firefox.launch(
                headless=True, proxy={"server": proxy}, firefox_user_prefs=FIREFOX_PREFS)
            context = browser.new_context(
                user_agent=ua, viewport={"width": 1280, "height": 800},
                service_workers="block", accept_downloads=False)

        for i, url in enumerate(urls):
            start = time.monotonic()
            try:
                if context is not None and is_page(url):
                    page = context.new_page()
                    try:
                        resp = page.goto(url, wait_until="load", timeout=timeout)
                        try:
                            page.wait_for_load_state("networkidle", timeout=1500)
                        except Exception:  # noqa: BLE001 - a busy page is still rendered
                            pass
                        ms = int((time.monotonic() - start) * 1000)
                        if resp is None:
                            raise RuntimeError("no response")
                        ctype = resp.headers.get("content-type", "")
                        raw = resp.body()
                        html = "html" in ctype.lower()
                        rendered = page.content() if html else None
                        record(i, url, final=page.url, status=resp.status, ctype=ctype, raw=raw,
                               max_bytes=max_bytes, ms=ms, rendered=rendered)
                        emit("note", text=f"fetched {i} {resp.status}")
                        if html:
                            try:
                                jpeg = page.screenshot(type="jpeg", quality=60, full_page=False)
                                emit("frame", b64=base64.b64encode(jpeg).decode("ascii"))
                            except Exception:  # noqa: BLE001 - no frame, still a result
                                pass
                    finally:
                        page.close()
                else:
                    r = api.get(url, timeout=timeout, max_redirects=max_redirects)
                    ms = int((time.monotonic() - start) * 1000)
                    raw = r.body()
                    record(i, url, final=r.url, status=r.status,
                           ctype=r.headers.get("content-type", ""), raw=raw,
                           max_bytes=max_bytes, ms=ms)
                    emit("note", text=f"fetched {i} {r.status}")
            except Exception as e:  # noqa: BLE001 - a URL that fails is recorded, not fatal
                ms = int((time.monotonic() - start) * 1000)
                record(i, url, max_bytes=max_bytes, ms=ms, error=words(e, url))
                emit("note", text=f"fetched {i} error")
            manifest(urls)
            if i + 1 < len(urls):
                time.sleep(delay)

        if browser is not None:
            browser.close()
        api.dispose()
    return 0


if __name__ == "__main__":
    code = 0
    try:
        code = main()
    except Exception as e:  # noqa: BLE001 - the manifest must exist whatever happened
        emit("note", text=f"room error: {str(e)[:150]}")
    finally:
        try:
            urls = json.load(open(f"{IN}/job.json", encoding="utf-8")).get("urls", [])
            manifest([str(u) for u in urls])
        except Exception:  # noqa: BLE001
            with open(f"{OUT}/fetch.json", "w", encoding="utf-8") as f:
                f.write('{"results":[]}')
    sys.exit(0 if code in (0, None) else code)
