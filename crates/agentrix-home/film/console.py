"""Captures the owner console for the home page's screen 5, from a local node.

  CONSOLE_OWNER_FILE=/path/to/test-owner.txt python3 console.py look <out dir> [path ...]
  CONSOLE_OWNER_FILE=/path/to/test-owner.txt python3 console.py shoot <out dir> [stage ...]
  python3 console.py crop <out dir> <static dir>

`brief` and `talk` are the Classy Perfumes sample on a local node, signed in; `shell-decision`
and `shell-live` are W5's shell-node (`cargo run -p agentrix-shell --example shell-node`, here
on :19021), whose seeded Kilele Bakery sample has a held order and a Live conversation.

Signs in as a local test owner inside headless Chrome (POST /auth/password, then /auth/totp),
so the session lives only in the capture profile. The owner file holds `email:`, `password:`
and `totp:` lines; it is read at run time and never printed, and only a local test account
belongs in it. CONSOLE_ORIGIN picks the node (default http://localhost:18050) and must be a
local address.
"""

import asyncio
import base64
import hashlib
import hmac
import json
import os
import re
import struct
import sys
import time
from urllib.parse import urlparse

from cdp import Browser, Page

ORIGIN = os.environ.get("CONSOLE_ORIGIN", "http://localhost:18050")
PROFILE = os.environ.get("CONSOLE_PROFILE", "/tmp/agentrix-console-capture")
WIDTH, HEIGHT = 1440, 900


def owner():
    """The test owner's email, password and authenticator secret, from CONSOLE_OWNER_FILE."""
    fields = {}
    with open(os.environ["CONSOLE_OWNER_FILE"]) as f:
        for line in f:
            m = re.match(r"\s*(email|password|totp)\s*[:=]\s*(.*?)\s*$", line)
            if m:
                fields[m.group(1)] = m.group(2)
    missing = {"email", "password", "totp"} - fields.keys()
    if missing:
        raise SystemExit(f"the owner file has no {', '.join(sorted(missing))} line")
    return fields


def totp(secret, at=None):
    """RFC 6238: six digits, SHA-1, 30-second steps."""
    key = base64.b32decode(secret.replace(" ", "").upper() + "=" * (-len(secret) % 8))
    counter = int((at or time.time()) // 30)
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    o = digest[-1] & 0x0F
    return f"{(struct.unpack('>I', digest[o:o + 4])[0] & 0x7FFFFFFF) % 1_000_000:06d}"


async def signed_in(page):
    """Signs in unless the profile's session still holds; says how, never with what."""
    await page.goto(f"{ORIGIN}/auth/session", settle=0.3)
    status = await page.eval("fetch('/auth/session', {credentials: 'same-origin'}).then(r => r.status)")
    if status == 200:
        return "session kept"
    who = owner()
    step = await page.eval(
        f"""fetch('/auth/password', {{method: 'POST', credentials: 'same-origin',
  headers: {{'content-type': 'application/json'}},
  body: JSON.stringify({{action: 'login', email: {json.dumps(who['email'])}, password: {json.dumps(who['password'])}}})}})
  .then(async r => ({{status: r.status, body: await r.json().catch(() => ({{}}))}}))"""
    )
    body = step.get("body") or {}
    if step.get("status") != 200 or body.get("next") != "totp":
        raise SystemExit(f"password step: {step.get('status')} {body.get('next') or body.get('error', '')}")
    # A code works once per 30-second window: wait for a window no earlier run used, and one
    # with a few seconds left. The profile keeps the last window's number (not the code).
    used = os.path.join(PROFILE, "last-totp-window")
    try:
        last = int(open(used).read())
    except (OSError, ValueError):
        last = -1
    while int(time.time() // 30) <= last or 30 - time.time() % 30 < 4:
        await asyncio.sleep(30 - time.time() % 30 + 0.5)
    window = int(time.time() // 30)
    done = await page.eval(
        f"""fetch('/auth/totp', {{method: 'POST', credentials: 'same-origin',
  headers: {{'content-type': 'application/json'}},
  body: JSON.stringify({{pending: {json.dumps(body['pending'])}, code: {json.dumps(totp(who['totp'], window * 30 + 1))}}})}})
  .then(async r => ({{status: r.status, redirect: (await r.json().catch(() => ({{}}))).redirect || ''}}))"""
    )
    with open(used, "w") as f:
        f.write(str(window))
    if done.get("status") != 200:
        raise SystemExit(f"authenticator step: {done.get('status')}")
    return f"signed in, sent to {done.get('redirect')}"


OUTLINE = """(() => {
  const out = [];
  for (const el of document.querySelectorAll('h1, h2, h3, a[href], button, [role=dialog], form')) {
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) continue;
    const t = (el.getAttribute('aria-label') || el.textContent || '').trim().replace(/\\s+/g, ' ').slice(0, 90);
    out.push(`${el.tagName.toLowerCase()} @${Math.round(r.left)},${Math.round(r.top + scrollY)} ${Math.round(r.width)}x${Math.round(r.height)}` + (el.getAttribute('href') ? ` ${el.getAttribute('href')}` : '') + ` | ${t}`);
  }
  return out.join('\\n');
})()"""


async def look(out, paths):
    """Full-page screenshots and an outline of each page, for planning the crops."""
    os.makedirs(out, exist_ok=True)
    with Browser(PROFILE, port=9341, width=WIDTH, height=HEIGHT) as b:
        page = await Page.open(b)
        await page.viewport(WIDTH, HEIGHT)
        print(await signed_in(page))
        for p in paths:
            await page.goto(p if p.startswith("http") else f"{ORIGIN}{p}", settle=1.5)
            name = re.sub(r"[^a-z0-9]+", "-", p.lower()).strip("-") or "root"
            await page.shot(f"{out}/{name}.png")
            with open(f"{out}/{name}.txt", "w") as f:
                f.write(f"{await page.eval('location.href')}\n{await page.eval(OUTLINE)}\n")
            print(f"{p} -> {name}.png")
        await page.close()


SITE = os.environ.get("CONSOLE_SITE", "classy-perfumes-sample")


async def until(page, expr, timeout=60, every=0.5):
    """Waits for `expr` to be truthy in the page; returns its last value."""
    deadline = time.time() + timeout
    value = None
    while time.time() < deadline:
        value = await page.eval(expr)
        if value:
            return value
        await asyncio.sleep(every)
    return value


async def stage(page, out, name, note=""):
    await page.shot(f"{out}/{name}.png")
    with open(f"{out}/{name}.txt", "w") as f:
        f.write(f"{await page.eval('location.href')}\n{note}\n{await page.eval(OUTLINE)}\n")
    print(f"{name}: {note}")


async def brief(page, out):
    await page.goto(f"{ORIGIN}/console/{SITE}", settle=2)
    await stage(page, out, "brief")


async def talk(page, out, said="How was this week?"):
    await page.goto(f"{ORIGIN}/console/{SITE}/apps/talk", settle=2)
    await page.eval(
        f"""(() => {{ const f = document.querySelector('[data-vtype]'); f.querySelector('input').value = {json.dumps(said)};
  f.requestSubmit(); return true; }})()"""
    )
    reply = await until(
        page,
        "[...document.querySelectorAll('[data-voice] *')].some(e => e.children.length === 0 && /Chief WebAgent/.test(e.textContent)) && document.querySelector('[data-voice]').innerText.length > 400",
        90,
        1,
    )
    await asyncio.sleep(3)
    await stage(page, out, "talk", f"reply={bool(reply)}")


# W5's shell-node (always signed in through test fakes, on its seeded Kilele Bakery sample):
# node's samples don't record their Ask turns in Live yet, so a node's Live has no conversation.
SHELL = os.environ.get("CONSOLE_SHELL", "http://localhost:19021")


async def shell_live(page, out):
    # Narrower, so the conversation's column fits the same 16:10 crop as the other frames.
    await page.viewport(1180, HEIGHT, scale=2)
    await page.goto(f"{SHELL}/console/site_kilele_sample/live", settle=2)
    await stage(page, out, "shell-live-list")
    thread = await page.eval("(document.querySelector('a[href*=\"/live/lv_\"]') || {}).href || ''")
    if thread:
        await page.goto(thread, settle=3)
        await stage(page, out, "shell-live-thread", thread)
    await page.viewport(WIDTH, HEIGHT, scale=2)


async def shell_decision(page, out):
    """The sample's held cake order, with its approval sheet open."""
    await page.goto(f"{SHELL}/console/site_kilele_sample/decisions", settle=2)
    hit = await page.eval(
        """(() => {
  for (const b of document.querySelectorAll('main button, main a')) {
    if (b.textContent.trim() !== 'Approve' || !b.getBoundingClientRect().width) continue;
    const card = b.closest('article, li, section, .card, [data-card]');
    if (card && card.textContent.includes('cake order')) { b.click(); return true; }
  }
  return false;
})()"""
    )
    sheet = await until(page, "(() => { const o = document.getElementById('passkey'); return !!o && !o.hidden && o.getBoundingClientRect().height > 0; })()", 15)
    await asyncio.sleep(1.2)
    await stage(page, out, "shell-decision", f"clicked={hit} sheet={sheet}")


STAGES = {
    "brief": brief,
    "talk": talk,
    "shell-decision": shell_decision,
    "shell-live": shell_live,
}
SIGNED = {"brief", "talk"}


async def shoot(out, names):
    os.makedirs(out, exist_ok=True)
    with Browser(PROFILE, port=9341, width=WIDTH, height=HEIGHT) as b:
        page = await Page.open(b)
        await page.viewport(WIDTH, HEIGHT, scale=2)
        if SIGNED & set(names):
            print(await signed_in(page))
        for n in names:
            await STAGES[n](page, out)
        await page.close()


# Screen 5's four frames: (capture, left, top) in CSS pixels at 2x; each is 760x475 CSS (16:10),
# written as a 1280x800 WebP under static/.
CROPS = {
    "owner-brief": ("brief", 0, 66),
    "owner-decide": ("shell-decision", 340, 212),
    "owner-live": ("shell-live-thread", 80, 66),
    "owner-ask": ("talk", 96, 190),
}


def crop(shots, static, quality=80):
    from PIL import Image

    for name, (shot, x, y) in CROPS.items():
        with Image.open(f"{shots}/{shot}.png") as im:
            box = (x * 2, y * 2, (x + 760) * 2, (y + 475) * 2)
            frame = im.convert("RGB").crop(box).resize((1280, 800), Image.LANCZOS)
            path = f"{static}/{name}.webp"
            frame.save(path, "WEBP", quality=quality, method=6)
            print(f"{name}.webp: {os.path.getsize(path) // 1024} KB from {shot}.png at {x},{y}")


if __name__ == "__main__":
    if urlparse(ORIGIN).hostname not in ("localhost", "127.0.0.1") and not urlparse(ORIGIN).hostname.endswith(".localhost"):
        raise SystemExit("CONSOLE_ORIGIN must be a local node")
    mode, out = sys.argv[1], sys.argv[2]
    if mode == "look":
        asyncio.run(look(out, sys.argv[3:] or ["/os"]))
    elif mode == "crop":
        crop(out, sys.argv[3])
    elif mode == "shoot":
        asyncio.run(shoot(out, sys.argv[3:] or list(STAGES)))
    else:
        raise SystemExit(f"unknown mode {mode}")
