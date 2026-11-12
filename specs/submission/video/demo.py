"""The screen-share demo: scenes driven like a presenter on a call, with a visible mouse cursor.

    python3 demo.py SCENE OUT.mp4 [--seconds 120]

Scenes: problem, solution, architecture, run, themes, terminal, close. Each is a real browser
session (headless Chrome, recorded by rec.py's screencast) against the live site, the public
GitHub repo, the deck or a terminal page that replays commands run for real just before.
"""

import argparse
import asyncio
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import rec  # noqa: E402

REPO = "https://github.com/zavora-ai/agentrix-webagent-platform"
SITE = "https://agentrix.website"
FIXTURE = "https://fixture.agentrix.website/"
POS = {"x": 960, "y": 540}

CURSOR_JS = """(() => {
  let c = document.getElementById('__cur');
  if (!c) {
    c = document.createElement('div'); c.id = '__cur';
    c.innerHTML = '<svg width="30" height="30" viewBox="0 0 24 24"><path d="M4 2l6.5 18.5 2.6-7.4 7.4-2.6z" fill="#16152b" stroke="#fff" stroke-width="1.7" stroke-linejoin="round"/></svg>';
    Object.assign(c.style, {position: 'fixed', zIndex: 2147483647, pointerEvents: 'none', left: '%(x)spx', top: '%(y)spx',
      transition: 'left .85s cubic-bezier(.45,0,.2,1), top .85s cubic-bezier(.45,0,.2,1)', filter: 'drop-shadow(0 2px 3px rgba(0,0,0,.35))'});
    document.documentElement.appendChild(c);
    const st = document.createElement('style');
    st.textContent = '@keyframes __rip{from{transform:translate(-50%%,-50%%) scale(.2);opacity:.6}to{transform:translate(-50%%,-50%%) scale(1.6);opacity:0}}';
    document.documentElement.appendChild(st);
  }
  return true;
})()"""


async def cursor(tab):
    await tab.js(CURSOR_JS % POS)


async def point(tab, selector=None, text=None, pause=0.6, dx=0, dy=0):
    """Glides the cursor to an element (scrolling it into view first) and returns its centre."""
    find = (f"document.querySelector({json.dumps(selector)})" if selector else
            f"[...document.querySelectorAll('a, button, h1, h2, h3, h4, p, li, td, th, span, label, summary, strong, b, img, svg, pre, code')].find(e => e.getBoundingClientRect().width && (e.innerText || '').trim().startsWith({json.dumps(text)}))")
    box = await tab.js(f"""(() => {{ const e = {find}; if (!e) return null;
      const r0 = e.getBoundingClientRect(); if (r0.top < 90 || r0.bottom > innerHeight - 60) e.scrollIntoView({{block: 'center', behavior: 'smooth'}});
      return true; }})()""")
    if not box:
        return None
    await asyncio.sleep(0.9)
    box = await tab.js(f"(() => {{ const r = ({find}).getBoundingClientRect(); return [Math.round(r.left + Math.min(r.width / 2, 160)), Math.round(r.top + r.height / 2)]; }})()")
    await cursor(tab)
    POS["x"], POS["y"] = box[0] + dx, box[1] + dy
    await tab.js(f"(() => {{ const c = document.getElementById('__cur'); c.style.left = '{POS['x']}px'; c.style.top = '{POS['y']}px'; return true; }})()")
    await asyncio.sleep(0.95 + pause)
    return POS["x"], POS["y"]


async def click(tab, selector=None, text=None, pause=0.4):
    at = await point(tab, selector, text, pause=0.2)
    if not at:
        raise RuntimeError(f"nothing to click: {selector or text}")
    x, y = at
    await tab.js(f"""(() => {{ const r = document.createElement('div');
      Object.assign(r.style, {{position: 'fixed', left: '{x}px', top: '{y}px', width: '46px', height: '46px', borderRadius: '50%', background: 'rgba(255,143,166,.55)',
        zIndex: 2147483646, pointerEvents: 'none', animation: '__rip .55s ease-out forwards'}});
      document.documentElement.appendChild(r); setTimeout(() => r.remove(), 700); return true; }})()""")
    for kind in ("mouseMoved", "mousePressed", "mouseReleased"):
        await tab.send("Input.dispatchMouseEvent", {"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1})
        await asyncio.sleep(0.05)
    await asyncio.sleep(pause)


async def go(tab, url, settle=1.6):
    await tab.goto(url, settle=settle)
    await cursor(tab)


# ---------------------------------------------------------------------------------------------

async def scene_problem(tab, base):
    await go(tab, REPO + "#1-why-agentrix-webagent-platform", settle=2.5)
    await point(tab, text="The problem", pause=1.2)
    await point(tab, text="1. Websites are passive", pause=2.5)
    await point(tab, text="Netcraft counted", pause=3.0)
    await point(tab, text="Search sends fewer people", pause=3.0)
    await point(tab, text="2. Customers now send AI agents", pause=2.0)
    await point(tab, text="More than half of web traffic", pause=3.0)
    await point(tab, text="3. The software underneath is insecure", pause=2.0)
    await point(tab, text="WordPress runs 39.4%", pause=2.2)
    await point(tab, text="11,334 new vulnerabilities", pause=3.5)


async def scene_solution(tab, base):
    await go(tab, SITE, settle=2.5)
    await point(tab, selector="h1", pause=2.0)
    await point(tab, selector="#film video, #film", pause=3.0)
    await click(tab, text="Samples", pause=2.0)
    await asyncio.sleep(1.5)
    await go(tab, "https://kilele-bakery-sample.agentrix.website/", settle=2.5)
    await point(tab, selector="h1", pause=2.0)
    await point(tab, text="Today's bakes", pause=1.5)
    sel = "input[placeholder^='Ask '][placeholder$='anything…'], input[placeholder*='anything']"
    await click(tab, selector=sel, pause=0.4)
    await tab.type_into(sel, "Are you open on Sunday?", cps=14)
    await asyncio.sleep(0.4)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
    await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
    await asyncio.sleep(9)


async def scene_architecture(tab, base):
    await go(tab, REPO + "#2-architecture", settle=3)
    await point(tab, text="2.1 The picture", pause=1.0)
    await tab.scroll_to(await tab.js("scrollY") + 380, 2.5)
    await asyncio.sleep(4)
    await point(tab, text="2.2 Design rules", pause=1.0)
    await point(tab, text="Static first", pause=1.5)
    await point(tab, text="Untrusted input only in sealed rooms", pause=2.5)
    svg = open(os.path.join(HERE, "../../../docs/diagrams/webagent-anatomy.svg")).read()
    wrap = os.path.join(HERE, "clips", "anatomy.html")
    open(wrap, "w").write('<!doctype html><meta charset="utf-8"><title>webagent-anatomy.svg</title><body style="margin:0;background:#fff;display:flex;justify-content:center">'
                          '<div style="width:1280px;margin-top:10px">' + svg + '</div></body>')
    await go(tab, "file://" + wrap, settle=1.2)
    for label in ("Wake router", "1 · Assemble context", "4 · Governed path decides", "Hold", "Model Gateway", "Sandbox plane", "Receipts"):
        await tab.js(f"""(() => {{ const t = [...document.querySelectorAll('text')].find(e => e.textContent.trim().startsWith({json.dumps(label)}));
          if (!t) return false; const r = t.getBoundingClientRect(); window.__pt = [Math.round(r.left + 20), Math.round(r.top + r.height / 2)]; return true; }})()""")
        pt = await tab.js("window.__pt || null")
        if pt:
            await cursor(tab)
            POS["x"], POS["y"] = pt
            await tab.js(f"(() => {{ const c = document.getElementById('__cur'); c.style.left = '{pt[0]}px'; c.style.top = '{pt[1]}px'; return true; }})()")
            await asyncio.sleep(2.2)


async def scene_run(tab, base):
    await go(tab, SITE, settle=2)
    sel = "form.paste input[name=url], input[name=url]"
    await click(tab, selector=sel, pause=0.3)
    await tab.type_into(sel, FIXTURE, cps=16)
    await asyncio.sleep(0.5)
    await click(tab, text="Check my site", pause=1.0)
    await tab.wait_for("document.body.innerText.includes('Checking ')", 30)
    await point(tab, text="Checking", pause=1.0)
    await tab.wait_for("document.body.innerText.includes('Your report is ready') || document.body.innerText.includes('In short')", 240)
    await asyncio.sleep(1.5)
    await point(tab, text="Hidden spam links", pause=1.5)
    await point(tab, text="Hidden instructions aimed at AI", pause=1.5)
    await point(tab, text="2 pages sent our sealed room", pause=2.5)
    await click(tab, text="See the full report", pause=2.5)
    await asyncio.sleep(2)
    href = await tab.js("(() => { const a = [...document.querySelectorAll('a')].find(a => /\\/receipts\\/sha256:/.test(a.getAttribute('href') || '')); return a ? a.getAttribute('href') : null; })()")
    if href:
        open(os.path.join(HERE, "clips", ".receipt"), "w").write(href)
        await tab.scroll_to(await tab.js("scrollY") + 500, 2)
        await asyncio.sleep(1.5)
        await click(tab, selector=f"a[href='{href}']", pause=2.0)
        await asyncio.sleep(1.5)
        await cursor(tab)
        await point(tab, text="Runtime", pause=1.5)
        await point(tab, text="Room destroyed", pause=1.5)
        if await tab.js("!![...document.querySelectorAll('button')].find(b => b.innerText.includes('Check the signature'))"):
            await click(tab, text="Check the signature", pause=3.5)


async def scene_themes(tab, base):
    deck = "file://" + os.path.join(HERE, "../deck/deck.html")
    await go(tab, deck, settle=2)
    for sid, spots in [("the-move", ["Replay", "Clean", "Go live"]), ("real-work", ["Plan", "Dispatch", "Verify"]),
                       ("architecture", []), ("models", ["Build", "Screening"]), ("containment", ["Process isolation", "Lifecycle"]),
                       ("gauntlet", ["Redirect to Vultr", "The replayed PHP"])]:
        await tab.js(f"document.getElementById({json.dumps(sid)}).scrollIntoView({{behavior: 'smooth', block: 'start'}}); true")
        await asyncio.sleep(1.6)
        await cursor(tab)
        if not spots:
            await asyncio.sleep(4)
        for sp in spots:
            await tab.js(f"""(() => {{ const s = document.getElementById({json.dumps(sid)});
              const e = [...s.querySelectorAll('b, h3, td, span, .tag, p')].find(e => e.innerText && e.innerText.trim().startsWith({json.dumps(sp)}));
              if (!e) return false; const r = e.getBoundingClientRect(); window.__pt = [Math.round(r.left + Math.min(r.width / 2, 120)), Math.round(r.top + r.height / 2)]; return true; }})()""")
            pt = await tab.js("window.__pt || null")
            if pt:
                POS["x"], POS["y"] = pt
                await tab.js(f"(() => {{ const c = document.getElementById('__cur'); c.style.left = '{pt[0]}px'; c.style.top = '{pt[1]}px'; return true; }})()")
                await asyncio.sleep(1.9)


def run(cmd):
    r = subprocess.run(["bash", "-c", cmd], cwd=HERE, capture_output=True, text=True, timeout=90)
    return (r.stdout or r.stderr).rstrip()


async def scene_terminal(tab, base):
    receipt = open(os.path.join(HERE, "clips", ".receipt")).read().strip() if os.path.exists(os.path.join(HERE, "clips", ".receipt")) else None
    steps = [
        {"note": "a customer's AI agent asks the site what it may do", "cmd": "./door.sh manifest"},
        {"note": "then searches the catalogue over MCP", "cmd": "./door.sh search sourdough"},
        {"cmd": './door.sh ask "Are you open on Sunday?"'},
        {"note": "every action leaves a signed receipt; here is the key they are signed with", "cmd": "curl -s https://agentrix.website/.well-known/agentrix/keys | jq -c '.keys[]? // .'"},
    ]
    if receipt:
        steps.append({"note": "and one sealed room's receipt from the check we just ran", "cmd": f"curl -s -H 'accept: application/json' https://agentrix.website{receipt} | jq '{{kind, destroyed, runtime: .policy.runtime, allowed: .policy.allowlist, refused: (.observed.refused // .observed.refusals)}}'", "hold": 5000})
    for s in steps:
        s["out"] = run(s["cmd"])
    open(os.path.join(HERE, "clips", ".terminal.json"), "w").write(json.dumps(steps, indent=1))
    await go(tab, "file://" + os.path.join(HERE, "terminal.html"), settle=1)
    await tab.js("window.ready")
    await tab.js(f"window.play({json.dumps(steps)})")


async def scene_close(tab, base):
    await go(tab, SITE, settle=2.5)
    await point(tab, selector="h1", pause=3.0)
    await point(tab, text="Check my site", pause=3.0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scene", choices=sorted(n for n in rec.SCENES))
    ap.add_argument("out")
    ap.add_argument("--seconds", type=float, default=150)
    ap.add_argument("--scale", type=float, default=1.25)
    a = ap.parse_args()
    asyncio.run(rec.record(a.scene, a.out, SITE, a.seconds, a.scale))



async def ask_bar(tab, question, wait=8):
    sel = "input[placeholder^='Ask '][placeholder$='anything…'], input[placeholder*='anything']"
    await click(tab, selector=sel, pause=0.3)
    await tab.type_into(sel, question, cps=15)
    await asyncio.sleep(0.4)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
    await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
    await asyncio.sleep(wait)


async def scene_kilele(tab, base):
    await go(tab, SITE, settle=2.2)
    await point(tab, selector="h1", pause=1.2)
    await point(tab, text="Check my site", pause=1.0)
    await point(tab, selector="#film video, .hero-film video, .hero-film", pause=1.5)
    await go(tab, "https://kilele-bakery-sample.agentrix.website/", settle=2.5)
    await point(tab, selector="h1", pause=1.2)
    await point(tab, text="Each menu item", pause=1.5)
    await ask_bar(tab, "Are you open on Sunday?", wait=7)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Escape", "code": "Escape", "windowsVirtualKeyCode": 27})
    await go(tab, "https://kilele-bakery-sample.agentrix.website/cakes/studio", settle=2.5)
    await point(tab, text="What's the occasion", pause=1.0)
    await click(tab, selector="#q", pause=0.3)
    await tab.type_into("#q", "Create for me a 3 kg cake for my 4 year old son who loves Roblox", cps=16)
    await asyncio.sleep(0.4)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
    await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
    await asyncio.sleep(10)
    await click(tab, selector="#q", pause=0.3)
    await tab.type_into("#q", "About 20 people. Chocolate sponge, sky blue icing, smooth finish, sprinkles, Happy birthday Liam, collect on Saturday 14", cps=18)
    await asyncio.sleep(0.4)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
    await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
    await asyncio.sleep(12)
    await point(tab, text="Your cake", pause=1.5)
    n0 = await tab.js("document.querySelectorAll('img').length")
    await click(tab, selector="#ideaBtn", pause=0.5)
    await tab.wait_for("[...document.querySelectorAll('img')].some(i => /\\/media\\/ideas\\//.test(i.currentSrc || i.src) && i.complete && i.naturalWidth > 0)", 75)
    await tab.js("(() => { const i = [...document.querySelectorAll('img')].find(i => /\\/media\\/ideas\\//.test(i.currentSrc || i.src)); if (i) i.scrollIntoView({block: 'center', behavior: 'smooth'}); return true; })()")
    await asyncio.sleep(1.5)
    await point(tab, selector="img[src*='/media/ideas/']", pause=4.0)
    await point(tab, text="Birthday cake, roblox theme", pause=3.0)


async def scene_zavora(tab, base):
    await go(tab, SITE, settle=2)
    sel = "form.paste input[name=url], input[name=url]"
    await click(tab, selector=sel, pause=0.3)
    await tab.type_into(sel, "https://zavora.ai/", cps=14)
    await asyncio.sleep(0.4)
    await click(tab, text="Check my site", pause=1.0)
    await tab.wait_for("document.body.innerText.includes('Checking ')", 30)
    await point(tab, text="Checking", pause=1.0)
    await tab.wait_for("document.body.innerText.includes('Your report is ready') || document.body.innerText.includes('In short')", 240)
    await asyncio.sleep(1.2)
    await point(tab, text="Hidden instructions aimed at AI", pause=1.5)
    await point(tab, text="A hidden frame", pause=1.5)
    href = await tab.js("(() => { const a = document.querySelector('[data-report]'); return a ? a.getAttribute('href') : null; })()")
    await point(tab, text="See the full report", pause=0.8)
    if href and href != "/":
        open(os.path.join(HERE, "clips", ".zavora-report"), "w").write(href)
        await go(tab, SITE + href, settle=2.5)
        for word in ("Live", "Sealed screenshot", "The sealed room", "In short", "Security", "AI assistants", "Speed", "Receipt", "receipt"):
            got = await point(tab, text=word, pause=1.6)
        await tab.scroll_to(await tab.js("document.documentElement.scrollHeight"), 4)
        await asyncio.sleep(2)


for _name, _fn in list(globals().items()):
    if _name.startswith("scene_"):
        rec.SCENES[_name[6:]] = _fn

if __name__ == "__main__":
    main()
