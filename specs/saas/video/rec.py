"""Records the live product for the PayPal entry's film, one scene per clip.

    python3 rec.py SCENE OUT.mp4 [--seconds 60] [--scale 1.5]

It reuses the recorder in ../../submission/video/rec.py (headless Chrome on its own profile, frames
piped to ffmpeg in real time). Every scene drives the deployed build the way a person would. The
owner scenes use the sample's public owner sign-in, which the page itself fills in. A ring is
drawn around the thing being talked about; nothing on the page is changed.

AGX_PLATFORM names the judged build's host (default paypal.agentrix.website).
"""

import argparse
import asyncio
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../../submission/video"))
import rec as base  # noqa: E402

P = os.environ.get("AGX_PLATFORM", "paypal.agentrix.website")
HOME = f"https://{P}"
KILELE = f"https://kilele-bakery-sample.{P}"
MONEY = f"https://money.{P}"
KMONEY = f"https://kilele.money.{P}"
OWNER = f"{HOME}/samples/kilele-bakery-sample"

RING = """(() => { const e = %s; document.querySelectorAll('.agx-ring').forEach(x => x.remove()); if (!e) return false;
  const r = e.getBoundingClientRect(), d = document.createElement('div'); d.className = 'agx-ring';
  d.style.cssText = `position:fixed;left:${r.left - 8}px;top:${r.top - 6}px;width:${r.width + 16}px;height:${r.height + 12}px;`
    + 'border:3px solid #ff4f6d;border-radius:12px;box-shadow:0 0 0 6px rgba(255,79,109,.22);pointer-events:none;z-index:99999;box-sizing:border-box';
  document.body.appendChild(d); return true; })()"""


RING_ALL = """(() => { const es = %s; document.querySelectorAll('.agx-ring').forEach(x => x.remove()); if (!es.length) return false;
  const rs = es.map(e => e.getBoundingClientRect()), l = Math.min(...rs.map(r => r.left)), t = Math.min(...rs.map(r => r.top)),
    r = Math.max(...rs.map(r => r.right)), b = Math.max(...rs.map(r => r.bottom)), d = document.createElement('div'); d.className = 'agx-ring';
  d.style.cssText = `position:fixed;left:${l - 8}px;top:${t - 6}px;width:${r - l + 16}px;height:${b - t + 12}px;`
    + 'border:3px solid #ff4f6d;border-radius:12px;box-shadow:0 0 0 6px rgba(255,79,109,.22);pointer-events:none;z-index:99999;box-sizing:border-box';
  document.body.appendChild(d); return true; })()"""


def by_text(tags, text):
    return f"[...document.querySelectorAll({json.dumps(tags)})].find(e => e.getBoundingClientRect().width && e.innerText.includes({json.dumps(text)}))"


def deep(text, nth=0):
    """The innermost element whose text holds `text` (the nth such, in page order)."""
    t = json.dumps(text)
    return (f"(() => {{ const has = e => e.getBoundingClientRect().width && (e.innerText || '').includes({t});"
            f" const tops = [...document.querySelectorAll('body *')].filter(e => has(e) && ![...e.children].some(has));"
            f" return tops[{nth}] || null; }})()")


async def show(tab, find, seconds=1.4, ring=True, offset=0, block="center"):
    """Brings the element into view (inside whatever scrolls) and rings it. The ring stays where
    it was drawn, so call unring before the page moves again."""
    await unring(tab)
    ok = await tab.js(f"(() => {{ const e = {find}; if (!e) return false; e.scrollIntoView({{block: {json.dumps(block)}, behavior: 'smooth'}}); return true; }})()")
    if not ok:
        print(f"rec: nothing found for {find[:80]}", file=sys.stderr)
        return False
    await asyncio.sleep(seconds)
    if ring:
        await tab.js(RING % find)
    return True


async def unring(tab):
    await tab.js("document.querySelectorAll('.agx-ring').forEach(x => x.remove()), true")


async def owner(tab, path=""):
    """Opens a page of Kilele's sample owner view, through the sign-in the sample fills in."""
    await tab.goto(f"{OWNER}/{path}", settle=1.0)
    if await tab.js("location.pathname.endsWith('/sign-in')"):
        await asyncio.sleep(1.0)
        await tab.click(text="Sign in as Wanjiru")
        await tab.wait_for("!location.pathname.endsWith('/sign-in')", 20)
        await tab.js("document.fonts.ready.then(() => true)")
        await asyncio.sleep(1.5)


async def scene_shop(tab, _):
    """The shop as a customer sees it, then its page for agents."""
    await tab.goto(KILELE + "/", settle=2)
    await asyncio.sleep(3)
    await tab.scroll_to(700, 2.5)
    await asyncio.sleep(2)
    await tab.goto(KILELE + "/agents", settle=1.5)
    await asyncio.sleep(4)


async def scene_door(tab, _):
    """Orders the shopper agents placed overnight, then one that came through the agent door."""
    await owner(tab, "orders")
    await asyncio.sleep(3.5)
    await show(tab, by_text("tr", "Agent door"), 1.5, offset=330)
    await asyncio.sleep(3)
    await unring(tab)
    href = await tab.js(f"(() => {{ const r = {by_text('tr', 'Agent door')}; return r ? r.querySelector('a').href : null; }})()")
    await tab.goto(href, settle=1.5)
    await asyncio.sleep(2)
    await show(tab, deep("verified by web-bot-auth"), 2.0, offset=380)
    await asyncio.sleep(6)


async def scene_today(tab, _):
    """The owner's morning: the brief counted from the records, and the decisions held for her."""
    await owner(tab, "")
    await asyncio.sleep(4)
    await show(tab, by_text("li", "custom orders wait for your OK"), 1.0, offset=300)
    await asyncio.sleep(4)
    await unring(tab)
    await show(tab, deep("Approve custom order"), 1.0, offset=300)
    await asyncio.sleep(4)


async def scene_settle(tab, _):
    """Kilele's console: the mandate, then the invoices its agents settled with Agentrix's agents."""
    await tab.goto(KMONEY + "/console", settle=2)
    await asyncio.sleep(2.5)
    await show(tab, deep("mandate for Payables") + "?.closest('article, section')", 1.5, offset=200)
    await asyncio.sleep(4)
    await unring(tab)
    await show(tab, deep("INV-2026-0002") + "?.closest('li')", 1.6, ring=False)
    await tab.js(RING_ALL % "[...document.querySelectorAll('li')].filter(e => /INV-\\d{4}-\\d+ · \\$/.test(e.innerText))")
    await asyncio.sleep(6)


async def scene_bills(tab, _):
    """Agentrix's own console: the bills its agents raised for themselves, and its staff of agents."""
    await tab.goto(MONEY + "/console", settle=2)
    await asyncio.sleep(2.5)
    await show(tab, deep("Bills to pay") + "?.closest('article, section')", 2.0, offset=200)
    await asyncio.sleep(5)
    await unring(tab)
    await show(tab, deep("Each agent signs its own requests") + "?.closest('article, section')", 2.0, offset=160, ring=False)
    await asyncio.sleep(4)


async def scene_customer(tab, _):
    """A person shops: the home page, the bread, two things into the bag, the bag with its total."""
    await tab.goto(KILELE + "/", settle=2)
    await asyncio.sleep(3)
    await tab.goto(KILELE + "/bread", settle=1.5)
    await asyncio.sleep(2)
    for n in (0, 1):
        await tab.js(RING % f"[...document.querySelectorAll('button')].filter(b => b.innerText.trim() === 'Add to bag')[{n}]")
        await asyncio.sleep(0.8)
        await tab.js(f"[...document.querySelectorAll('button')].filter(b => b.innerText.trim() === 'Add to bag')[{n}].click(), true")
        await asyncio.sleep(1.4)
    await unring(tab)
    await tab.goto(KILELE + "/bag", settle=1.5)
    await asyncio.sleep(5)
    await tab.js("scrollBy({top: 380, behavior: 'smooth'}), true")
    await asyncio.sleep(4)


async def settle_text(tab, limit=40):
    """Waits until the page's text has stopped growing for a second and a half."""
    last, still = -1, 0
    for _ in range(limit * 2):
        n = await tab.js("document.body.innerText.length")
        still = still + 1 if n == last else 0
        last = n
        if still >= 3:
            return
        await asyncio.sleep(0.5)


async def say_to(tab, selector, text):
    await tab.click(selector=selector)
    await asyncio.sleep(0.4)
    await tab.type_into(selector, text, cps=18)
    await asyncio.sleep(0.5)
    await tab.send("Input.dispatchKeyEvent", {"type": "keyDown", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"})
    await tab.send("Input.dispatchKeyEvent", {"type": "keyUp", "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13})
    await asyncio.sleep(1.5)
    await settle_text(tab)


async def scene_chat(tab, _):
    """A customer asks about a product, says what she wants, and the desk puts the order and a
    pay card in the conversation; she presses it."""
    await tab.goto(KILELE + "/", settle=2)
    await asyncio.sleep(2)
    await say_to(tab, "input[placeholder*='anything']", os.environ.get("AGX_CHAT_1", "Is the country sourdough in stock?"))
    await asyncio.sleep(3)
    await say_to(tab, "input[placeholder*='follow-up'], textarea[placeholder*='follow-up']", os.environ.get("AGX_CHAT_2", "I'd like two loaves, please."))
    await asyncio.sleep(1.5)
    if await tab.js("!!document.querySelector('.pay-card')"):
        await tab.js("document.querySelector('.pay-card').scrollIntoView({block: 'center', behavior: 'smooth'}), true")
        await asyncio.sleep(1.2)
        await tab.js(RING % "document.querySelector('.pay-card')")
        await asyncio.sleep(5)
        await unring(tab)
        await tab.js(RING % "document.querySelector('.pay-card button')")
        await asyncio.sleep(1.2)
        await unring(tab)
        await tab.click(selector=".pay-card button")
        await asyncio.sleep(6)
    else:
        print("rec: no pay card in the conversation", file=sys.stderr)
        await asyncio.sleep(3)


async def scene_track(tab, _):
    """A customer tracks an order by its number; then the paid order as the shop holds it."""
    order = os.environ.get("AGX_ORDER", "KB-1052")
    await tab.goto(KILELE + "/orders", settle=2)
    await asyncio.sleep(1.5)
    sel = "form.k-track input, input[name='n'], #ordno"
    await tab.click(selector=sel)
    await tab.type_into(sel, order, cps=12)
    await asyncio.sleep(0.6)
    await tab.js(f"document.querySelector({json.dumps(sel)}).form.requestSubmit(), true")
    await asyncio.sleep(1.5)
    await settle_text(tab, 10)
    await show(tab, deep(order) + "?.closest('article, section, li, div')", 1.2)
    await asyncio.sleep(4)
    await unring(tab)
    await owner(tab, "orders/" + order)
    await asyncio.sleep(1.5)
    await show(tab, deep("with PayPal at Kilele's rate") + "?.parentElement", 1.5)
    await asyncio.sleep(4.5)
    await show(tab, deep("captured") + "?.closest('tr')", 1.2)
    await asyncio.sleep(4)


async def scene_paid(tab, _):
    """A paid order in the owner's view: what was ordered, and the PayPal payment the Cashier took."""
    await owner(tab, "orders/" + os.environ.get("AGX_ORDER", "KB-1052"))
    await asyncio.sleep(3)
    await show(tab, deep("with PayPal at Kilele's rate") + "?.parentElement", 1.5)
    await asyncio.sleep(4.5)
    await show(tab, deep("captured") + "?.closest('tr')", 1.2)
    await asyncio.sleep(4)


async def scene_control(tab, _):
    """The owner's control: the mandate she signed, the bills inside it, and the off switch for each agent."""
    await tab.goto(KMONEY + "/console", settle=2)
    await asyncio.sleep(2)
    await show(tab, deep("mandate for Payables") + "?.closest('article, section')", 1.5, block="start")
    await asyncio.sleep(5)
    await show(tab, deep("Bills to pay") + "?.closest('article, section')", 1.6, block="start")
    await asyncio.sleep(4)
    await show(tab, "[...document.querySelectorAll('button')].find(b => b.innerText.trim() === 'Revoke') || " + deep("Each agent signs its own requests"), 1.6)
    await asyncio.sleep(5)


async def scene_agentrix(tab, _):
    """Agentrix itself: the plans a site subscribes to with PayPal, then what a site's agents paid it."""
    await tab.goto(MONEY + "/", settle=2)
    await asyncio.sleep(2.5)
    await show(tab, deep("Subscribe with PayPal") + "?.closest('section') || " + deep("Subscribe with PayPal"), 1.6, ring=False, block="center")
    await asyncio.sleep(4.5)
    await tab.goto(MONEY + "/console", settle=1.5)
    await asyncio.sleep(1.5)
    await show(tab, deep("Settled agent to agent") + "?.closest('article, section')", 1.6)
    await asyncio.sleep(6)


async def scene_wallet(tab, _):
    """Kilele's wallet: every sale with PayPal's id, matched three ways, and the hourly checkpoints."""
    await tab.goto(KMONEY + "/console/kilele/wallet", settle=2)
    await asyncio.sleep(2.5)
    await show(tab, deep("Three-way matched") + "?.parentElement", 1.2)
    await asyncio.sleep(3.5)
    await show(tab, deep("Wallet ledger") + "?.closest('article, section') || " + deep("Wallet ledger"), 1.6, ring=False, block="start")
    await asyncio.sleep(4)
    await show(tab, deep("Wallet checkpoints") + "?.closest('article, section') || " + deep("Wallet checkpoints"), 2.0, ring=False, block="start")
    await asyncio.sleep(4.5)


async def scene_badge(tab, _):
    """An agent's badge, verified on the page."""
    await tab.goto(KILELE + "/agents", settle=2)
    await asyncio.sleep(2.5)
    await tab.click(selector="form[data-verify] button")
    await asyncio.sleep(3)
    await show(tab, "document.querySelector('form[data-verify] [data-verify-result]')", 1.4)
    await asyncio.sleep(5)
    await show(tab, deep("matches PayPal"), 1.6)
    await asyncio.sleep(5)


async def scene_proofs(tab, _):
    """Proofs: the signed log, then the attack suite."""
    await tab.goto(MONEY + "/proofs", settle=2)
    await asyncio.sleep(4)
    await show(tab, by_text("h2, h3", "The attack suite"), 2.5, offset=120, ring=False)
    await asyncio.sleep(5)
    await tab.js("scrollBy({top: 420, behavior: 'smooth'}), true")
    await asyncio.sleep(4)


async def scene_ask(tab, _):
    """A visitor asks the shop's front desk a question, and it answers from the shop's own pages."""
    os.environ["AGX_URL"] = KILELE + "/"
    os.environ.setdefault("AGX_QUESTIONS", "Are you open on Sunday?")
    await base.scene_ask(tab, HOME)


async def scene_runday(tab, _):
    """The judges' page: Run the day, pressed for real. The five scenes run against the sandbox;
    the page keeps the running scene in view. When each scene finished is written beside the
    clip (runday.json), so the film can cut on those moments."""
    t0 = asyncio.get_event_loop().time()
    marks = {}
    await tab.goto(MONEY + "/judges", settle=2)
    await asyncio.sleep(2.5)
    await show(tab, "document.querySelector('section.run-day')", 1.4, ring=False, block="start")
    await tab.js("scrollBy({top: -76, behavior: 'smooth'}), true")
    await asyncio.sleep(1.6)
    await tab.js(RING % "[...document.querySelectorAll('section.run-day button')].find(b => b.innerText.includes('Run the day'))")
    await asyncio.sleep(1.2)
    await unring(tab)
    marks["pressed"] = asyncio.get_event_loop().time() - t0
    await tab.click(text="Run the day")
    seen = 0
    for _ in range(600):  # at most five minutes
        await asyncio.sleep(0.5)
        state = await tab.js("[...document.querySelectorAll('section.run-day li.scene')].map(e => e.classList.contains('done') ? 'done' : e.classList.contains('running') ? 'running' : e.classList.contains('failed') ? 'failed' : 'pending')")
        done = sum(1 for x in state if x == "done")
        if done > seen:
            for n in range(seen + 1, done + 1):
                marks[f"scene{n}"] = asyncio.get_event_loop().time() - t0
            seen = done
        # Keep the end of the running scene (or the last finished one) on screen.
        await tab.js("""(() => { const all = [...document.querySelectorAll('section.run-day li.scene')];
          const e = all.find(x => x.classList.contains('running')) || all.filter(x => x.classList.contains('done')).pop();
          if (!e) return false; const r = e.getBoundingClientRect();
          if (r.bottom > innerHeight - 40) scrollBy({top: Math.min(r.bottom - innerHeight + 60, 260), behavior: 'smooth'});
          else if (r.top < 70) scrollBy({top: r.top - 90, behavior: 'smooth'}); return true; })()""")
        if state and all(x in ("done", "failed") for x in state):
            break
    await asyncio.sleep(3)
    # A slow pass over what the run left, so the narration can point at each result.
    now = lambda: asyncio.get_event_loop().time() - t0  # noqa: E731
    for key, text, hold in (
        ("tour_settled", "settled it on PayPal through Vault", 4.0),
        ("tour_short", "We'll propose instalments", 4.5),
        ("tour_plan", "agreed a plan with Kilele's agent", 4.5),
        ("tour_collected", "collected through Vault", 4.0),
        ("tour_payout", "paid it by PayPal Payouts", 4.0),
        ("tour_held", "waits for their passkey", 5.0),
        ("tour_refused", "denied by policy", 5.0),
        ("tour_audit", "signed audit log, provable", 4.0),
    ):
        if await tab.js(f"!!{deep(text)}"):
            marks[key] = now()
            await show(tab, deep(text), 1.3)
            await asyncio.sleep(hold)
    await unring(tab)
    marks["end"] = now()
    marks["states"] = state
    out = os.environ.get("AGX_RUNDAY_MARKS", os.path.join(HERE, "runday.json"))
    json.dump(marks, open(out, "w"), indent=1)
    print("runday marks:", json.dumps(marks))


async def terminal(tab, session):
    await tab.goto("file://" + os.path.join(HERE, "terminal.html"), settle=0.5)
    await tab.js("window.ready")
    await asyncio.sleep(0.8)
    await tab.js(f"window.play({json.dumps(session)})")
    await asyncio.sleep(1)


def sh(script):
    return subprocess.run([os.path.join(HERE, script)], capture_output=True, text=True, timeout=60).stdout.strip()


def door(*args):
    return subprocess.run([os.path.join(HERE, "door.sh"), *args], capture_output=True, text=True, timeout=60).stdout.rstrip()


async def scene_agentcli(tab, _):
    """A terminal: what a customer's agent does at the shop's agent door, run for real a moment
    before recording. It reads what the shop lets agents do, then searches the catalogue."""
    await terminal(tab, [
        {"note": "A customer's AI agent arrives at Kilele Bakery's agent door (MCP).", "cmd": "./door.sh manifest", "out": door("manifest"), "hold": 3200},
        {"note": "It searches the catalogue, as an agent would.", "cmd": "./door.sh search sourdough", "out": door("search", "sourdough"), "hold": 4200},
    ])


async def scene_impostor(tab, _):
    """A terminal: the impostor request, run for real a moment before recording."""
    await terminal(tab, [{"note": "An impostor names a real agent and makes up the signature.", "cmd": "./impostor.sh", "out": sh("impostor.sh"), "hold": 7000}])


async def scene_verify(tab, _):
    """A terminal: an agent's badge checked from outside, run for real a moment before recording."""
    await terminal(tab, [{"note": "Check the Cashier's badge from outside. Anyone can.", "cmd": "./verify.sh", "out": sh("verify.sh"), "hold": 7000}])


async def scene_replay(tab, _):
    """The night, replayed from the record (replay.html reads record.json beside it)."""
    await tab.goto("file://" + os.path.join(HERE, "replay.html") + "?auto=1&seconds=" + os.environ.get("AGX_REPLAY_SECONDS", "12"), settle=0.5)
    await tab.wait_for("window.done === true", 90)
    await asyncio.sleep(1.5)


_ask = base.scene_ask
base.SCENES.clear()
base.scene_ask = _ask
base.SCENES.update({name[6:]: fn for name, fn in globals().items() if name.startswith("scene_")})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scene", choices=sorted(base.SCENES))
    ap.add_argument("out")
    ap.add_argument("--seconds", type=float, default=75)
    ap.add_argument("--scale", type=float, default=1.5, help="device pixel ratio; the page lays out at 1920/scale wide")
    a = ap.parse_args()
    asyncio.run(base.record(a.scene, a.out, HOME, a.seconds, a.scale))


if __name__ == "__main__":
    main()
