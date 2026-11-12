"""A small Chrome DevTools driver for the home page film: capture product pages and render frames.

Starts headless Chrome on its own profile and port, so it never touches anyone's browser.
Used by capture.py (the right-hand panels, clicked through on the real product) and
render.py (the film, one frame at a time).
"""

import asyncio
import base64
import itertools
import json
import os
import subprocess
import time
import urllib.request

import websockets

CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"


class Browser:
    def __init__(self, profile, port=9333, width=1920, height=1080):
        self.profile, self.port, self.width, self.height = profile, port, width, height
        self.proc = None

    def __enter__(self):
        os.makedirs(self.profile, exist_ok=True)
        self.proc = subprocess.Popen(
            [
                CHROME,
                "--headless=new",
                f"--remote-debugging-port={self.port}",
                f"--user-data-dir={self.profile}",
                "--no-first-run",
                "--no-default-browser-check",
                "--hide-scrollbars",
                "--disable-gpu",
                "--force-color-profile=srgb",
                f"--window-size={self.width},{self.height}",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.time() + 20
        while time.time() < deadline:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{self.port}/json/version", timeout=1)
                return self
            except Exception:
                time.sleep(0.2)
        raise RuntimeError("Chrome did not start")

    def __exit__(self, *exc):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def ws_url(self):
        targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{self.port}/json"))
        page = next(t for t in targets if t["type"] == "page")
        return page["webSocketDebuggerUrl"]


class Page:
    def __init__(self, ws):
        self.ws = ws
        self.ids = itertools.count(1)
        self.events = []

    @classmethod
    async def open(cls, browser):
        ws = await websockets.connect(browser.ws_url(), max_size=None)
        page = cls(ws)
        await page.send("Page.enable")
        await page.send("Runtime.enable")
        return page

    async def send(self, method, params=None):
        msg_id = next(self.ids)
        await self.ws.send(json.dumps({"id": msg_id, "method": method, "params": params or {}}))
        while True:
            msg = json.loads(await self.ws.recv())
            if msg.get("id") == msg_id:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return msg.get("result", {})
            self.events.append(msg)

    async def wait_event(self, name, timeout=20):
        deadline = time.time() + timeout
        while time.time() < deadline:
            for i, ev in enumerate(self.events):
                if ev.get("method") == name:
                    del self.events[i]
                    return ev
            try:
                msg = json.loads(await asyncio.wait_for(self.ws.recv(), timeout=0.5))
                self.events.append(msg)
            except asyncio.TimeoutError:
                pass
        raise TimeoutError(name)

    async def viewport(self, width, height, scale=1, mobile=False):
        await self.send(
            "Emulation.setDeviceMetricsOverride",
            {"width": width, "height": height, "deviceScaleFactor": scale, "mobile": mobile},
        )

    async def goto(self, url, settle=0.8):
        self.events.clear()
        await self.send("Page.navigate", {"url": url})
        try:
            await self.wait_event("Page.loadEventFired", timeout=15)
        except TimeoutError:
            pass
        await self.eval("document.fonts ? document.fonts.ready.then(() => true) : true")
        await asyncio.sleep(settle)

    async def eval(self, expr):
        res = await self.send(
            "Runtime.evaluate",
            {"expression": expr, "awaitPromise": True, "returnByValue": True},
        )
        if "exceptionDetails" in res:
            raise RuntimeError(f"eval failed: {res['exceptionDetails'].get('text')} in {expr[:80]}")
        return res.get("result", {}).get("value")

    async def click_text(self, text, selector="button, a, [role=button], label"):
        """Clicks the first visible element whose text contains `text`."""
        found = await self.eval(
            f"""(() => {{
  const want = {json.dumps(text)};
  for (const el of document.querySelectorAll({json.dumps(selector)})) {{
    const r = el.getBoundingClientRect();
    if (r.width && r.height && el.textContent.includes(want)) {{ el.click(); return true; }}
  }}
  return false;
}})()"""
        )
        return bool(found)

    async def shot(self, path, fmt="png", quality=92):
        params = {"format": fmt, "captureBeyondViewport": False}
        if fmt == "jpeg":
            params["quality"] = quality
        data = (await self.send("Page.captureScreenshot", params))["data"]
        with open(path, "wb") as f:
            f.write(base64.b64decode(data))

    async def close(self):
        await self.ws.close()
