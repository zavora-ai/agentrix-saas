"""Records a scripted browser session of the live platform as an H.264 clip, with no frame dump.

    python3 rec.py SCENE OUT.mp4 [--base https://agentrix.website] [--seconds 60]

Headless Chrome on its own profile (it never touches anyone's browser). Chrome's screencast sends
JPEG frames; a writer feeds the latest one to ffmpeg 30 times a second, so the clip runs in real
time and nothing is cut. Scenes are the coroutines named scene_* below; each drives the page the
way a person would (type, click, scroll) and waits on what the page shows.
"""

import argparse
import asyncio
import base64
import itertools
import json
import os
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "../../../crates/agentrix-home/film"))
from cdp import Browser  # noqa: E402

import websockets  # noqa: E402

W, H, FPS = 1920, 1080, 30


class Tab:
    def __init__(self, ws):
        self.ws, self.ids, self.pending, self.frame, self.frames = ws, itertools.count(1), {}, None, 0
        self.waiters = []

    async def reader(self):
        async for raw in self.ws:
            msg = json.loads(raw)
            if "id" in msg and msg["id"] in self.pending:
                self.pending.pop(msg["id"]).set_result(msg)
            elif msg.get("method") == "Page.screencastFrame":
                p = msg["params"]
                self.frame = base64.b64decode(p["data"])
                self.frames += 1
                asyncio.ensure_future(self.send("Page.screencastFrameAck", {"sessionId": p["sessionId"]}))
            for w in list(self.waiters):
                if msg.get("method") == w[0] and not w[1].done():
                    w[1].set_result(msg)
                    self.waiters.remove(w)

    async def send(self, method, params=None):
        i = next(self.ids)
        fut = asyncio.get_event_loop().create_future()
        self.pending[i] = fut
        await self.ws.send(json.dumps({"id": i, "method": method, "params": params or {}}))
        msg = await asyncio.wait_for(fut, 60)
        if "error" in msg:
            raise RuntimeError(f"{method}: {msg['error']}")
        return msg.get("result", {})

    async def event(self, name, timeout=20):
        fut = asyncio.get_event_loop().create_future()
        self.waiters.append((name, fut))
        try:
            return await asyncio.wait_for(fut, timeout)
        except asyncio.TimeoutError:
            return None

    async def js(self, expr):
        r = await self.send("Runtime.evaluate", {"expression": expr, "awaitPromise": True, "returnByValue": True})
        if "exceptionDetails" in r:
            raise RuntimeError(f"js: {r['exceptionDetails'].get('text')} {expr[:90]}")
        return r.get("result", {}).get("value")

    async def goto(self, url, settle=1.0):
        load = asyncio.ensure_future(self.event("Page.loadEventFired", 25))
        await self.send("Page.navigate", {"url": url})
        await load
        await self.js("document.fonts ? document.fonts.ready.then(() => true) : true")
        await asyncio.sleep(settle)

    async def wait_for(self, cond_js, timeout=120, poll=0.25):
        end = time.time() + timeout
        while time.time() < end:
            if await self.js(f"!!({cond_js})"):
                return True
            await asyncio.sleep(poll)
        return False

    async def wait_text(self, text, timeout=120):
        return await self.wait_for(f"document.body && document.body.innerText.includes({json.dumps(text)})", timeout)

    async def scroll_to(self, y_or_selector, seconds=2.0):
        """Scrolls smoothly, the way a reader would."""
        if isinstance(y_or_selector, str):
            target = f"(document.querySelector({json.dumps(y_or_selector)})?.getBoundingClientRect().top ?? 0) + scrollY - 90"
        else:
            target = str(y_or_selector)
        await self.js(f"""new Promise(done => {{
  const from = scrollY, to = Math.max(0, {target}), t0 = performance.now(), d = {seconds * 1000};
  const step = t => {{ const k = Math.min(1, (t - t0) / d), e = k < .5 ? 2*k*k : 1 - Math.pow(-2*k + 2, 2) / 2;
    scrollTo(0, from + (to - from) * e); k < 1 ? requestAnimationFrame(step) : done(true); }};
  requestAnimationFrame(step); }})""")

    async def type_into(self, selector, text, cps=14):
        await self.js(f"(() => {{ const el = document.querySelector({json.dumps(selector)}); el.focus(); el.value = ''; return true; }})()")
        for ch in text:
            await self.send("Input.insertText", {"text": ch})
            await asyncio.sleep(1 / cps)

    async def click(self, selector=None, text=None):
        """Clicks an element by CSS selector or by its visible text, with a real mouse event."""
        find = (f"document.querySelector({json.dumps(selector)})" if selector else
                f"[...document.querySelectorAll('button, a, [role=button], summary, label')].find(e => e.getBoundingClientRect().width && e.innerText.trim().includes({json.dumps(text)}))")
        box = await self.js(f"(() => {{ const e = {find}; if (!e) return null; e.scrollIntoView({{block: 'center', behavior: 'instant'}}); const r = e.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()")
        if not box:
            raise RuntimeError(f"nothing to click: {selector or text}")
        x, y = box
        for kind in ("mouseMoved", "mousePressed", "mouseReleased"):
            await self.send("Input.dispatchMouseEvent", {"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1})
            await asyncio.sleep(0.05)


async def record(scene, out, base, seconds, scale):
    profile = tempfile.mkdtemp(prefix="agx-rec-")
    ff = subprocess.Popen(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "image2pipe", "-framerate", str(FPS),
                           "-c:v", "mjpeg", "-i", "-", "-vf", f"scale={W}:{H}:force_original_aspect_ratio=decrease,pad={W}:{H}:(ow-iw)/2:(oh-ih)/2:color=0x1f1d3d,format=yuv420p",
                           "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-r", str(FPS), out], stdin=subprocess.PIPE)
    with Browser(profile, port=9420, width=W, height=H) as b:
        ws = await websockets.connect(b.ws_url(), max_size=None)
        tab = Tab(ws)
        reader = asyncio.ensure_future(tab.reader())
        await tab.send("Page.enable")
        await tab.send("Runtime.enable")
        await tab.send("Emulation.setDeviceMetricsOverride", {"width": int(W / scale), "height": int(H / scale), "deviceScaleFactor": scale, "mobile": False})
        await tab.send("Page.startScreencast", {"format": "jpeg", "quality": 88, "maxWidth": W, "maxHeight": H, "everyNthFrame": 1})
        stop = asyncio.Event()

        async def writer():
            t0, n = time.monotonic(), 0
            while not stop.is_set():
                if tab.frame:
                    ff.stdin.write(tab.frame)
                    n += 1
                await asyncio.sleep(max(0, t0 + n / FPS - time.monotonic()) if tab.frame else 0.02)
                if not tab.frame:
                    t0 = time.monotonic()
            return n

        wtask = asyncio.ensure_future(writer())
        started = time.monotonic()
        try:
            await asyncio.wait_for(SCENES[scene](tab, base), timeout=seconds)
        except asyncio.TimeoutError:
            print(f"rec: scene {scene} hit its {seconds} s limit; the clip ends there", file=sys.stderr)
        finally:
            stop.set()
            n = await wtask
            ff.stdin.close()
            ff.wait()
            reader.cancel()
            await ws.close()
    print(f"{out}: {n / FPS:.1f} s of video from {tab.frames} screencast frames, {time.monotonic() - started:.1f} s wall")


# ---------------------------------------------------------------------------------------------
# Scenes. Each returns when its story is told; the caller caps its length.
# ---------------------------------------------------------------------------------------------

async def scene_test(tab, base):
    await tab.goto(base)
    await asyncio.sleep(1)
    await tab.scroll_to(1080 * 2, 3)
    await asyncio.sleep(1)


async def scene_home(tab, base):
    """The front door: the hero, how it works, the samples, prices."""
    await tab.goto(base, settle=2.5)
    await asyncio.sleep(3)
    for sel in ("#film", "#samples", "#owner", "#price"):
        if await tab.js(f"!!document.querySelector({json.dumps(sel)})"):
            await tab.scroll_to(sel, 2.2)
            await asyncio.sleep(2.2)
    await tab.scroll_to(0, 2)
    await asyncio.sleep(1)


async def scene_assess(tab, base):
    """Assess, no account: paste the fixture's address and watch the sealed browser read it."""
    fixture = os.environ.get("AGX_FIXTURE", "https://fixture.agentrix.website/")
    await tab.goto(base, settle=2)
    sel = "form.paste input[name=url], form[action*='assess'] input[name=url], input[name=url]"
    await tab.type_into(sel, fixture)
    await asyncio.sleep(0.6)
    await tab.js(f"document.querySelector({json.dumps(sel)}).form.requestSubmit()")
    await tab.wait_for("location.pathname.startsWith('/assess/') || document.body.innerText.includes('Checking ')", 30)
    await asyncio.sleep(2)
    await tab.wait_for("document.body.innerText.includes('In short') || document.body.innerText.includes('Your report is ready')", 240)
    await asyncio.sleep(3)
    await tab.scroll_to(900, 3)
    await asyncio.sleep(4)
    await tab.scroll_to(1800, 3)
    await asyncio.sleep(4)


async def scene_page(tab, base):
    """One page, read slowly: AGX_URL, scrolled to AGX_SCROLL (comma-separated y values)."""
    await tab.goto(os.environ.get("AGX_URL", base), settle=2)
    await asyncio.sleep(2.5)
    for y in [int(v) for v in os.environ.get("AGX_SCROLL", "").split(",") if v.strip()]:
        await tab.scroll_to(y, 2.5)
        await asyncio.sleep(2.5)


async def scene_receipts(tab, base):
    """Receipts, public at /receipts/{digest}: AGX_RECEIPTS is a comma-separated list of sha256 digests."""
    for d in [x.strip() for x in os.environ.get("AGX_RECEIPTS", "").split(",") if x.strip()]:
        await tab.goto(f"{base}/receipts/{d if d.startswith('sha256:') else 'sha256:' + d}", settle=1.5)
        await asyncio.sleep(2.5)
        for word in ("Runtime", "Room destroyed", "Attack", "contained", "held"):
            y = await tab.js(f"(() => {{ const el = [...document.querySelectorAll('dt, th, h2, h3, li, td, p')].find(e => e.innerText.trim().startsWith({json.dumps(word)})); return el ? el.getBoundingClientRect().top + scrollY - 260 : null; }})()")
            if y and y > 200:
                await tab.scroll_to(y, 2.2)
                await asyncio.sleep(3)
                break
        if await tab.js("!![...document.querySelectorAll('button')].find(b => b.innerText.includes('Check the signature'))"):
            await tab.click(text="Check the signature")
            await asyncio.sleep(3.5)
        await asyncio.sleep(1.5)


async def scene_ask(tab, base):
    """A site's Ask bar: AGX_URL, then each question in AGX_QUESTIONS (separated by ||), answered live."""
    await tab.goto(os.environ.get("AGX_URL", base), settle=2)
    await asyncio.sleep(1.5)
    sel = "input[placeholder^='Ask '][placeholder$='anything…'], input[placeholder*='anything']"
    for q in [x.strip() for x in os.environ.get("AGX_QUESTIONS", "").split("||") if x.strip()]:
        await tab.click(selector=sel)
        await asyncio.sleep(0.6)
        await tab.type_into(sel, q, cps=16)
        await asyncio.sleep(0.5)
        await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
        await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
        await asyncio.sleep(2)
        last, still = -1, 0
        for _ in range(120):  # until the page text stops growing for 3 s, at most 60 s
            n = await tab.js("document.body.innerText.length")
            still = still + 1 if n == last else 0
            last = n
            if still >= 6:
                break
            await asyncio.sleep(0.5)
        await asyncio.sleep(4)


SCENES = {name[6:]: fn for name, fn in globals().items() if name.startswith("scene_")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scene", choices=sorted(SCENES))
    ap.add_argument("out")
    ap.add_argument("--base", default="https://agentrix.website")
    ap.add_argument("--seconds", type=float, default=90)
    ap.add_argument("--scale", type=float, default=1.25, help="device pixel ratio; the page lays out at 1920/scale wide")
    a = ap.parse_args()
    asyncio.run(record(a.scene, a.out, a.base, a.seconds, a.scale))


if __name__ == "__main__":
    main()
