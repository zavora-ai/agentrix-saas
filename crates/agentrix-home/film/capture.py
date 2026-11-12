"""Captures the film's right-hand panels from the real product, running locally.

  python3 capture.py <out dir> [name ...]

Needs W2's samples preview on :19020 (kilele-bakery-sample.localhost), W5's shell-node on
:19021 and W4's onboarding preview behind ONBOARDING's proxy. Each capture is 960x1080 CSS
pixels at 2x; the film uses them as WebP (quality 86) in right/.
"""

import asyncio
import json
import os
import sys

from cdp import Browser, Page

KILELE = "http://kilele-bakery-sample.localhost:19020"
CONSOLE = "http://localhost:19021/console/site_kilele_sample"
# W4's onboarding preview (cargo run -p agentrix-onboarding --example preview), behind a proxy that
# adds W7's /static/ui/* from shell-node, since the preview doesn't mount them.
ONBOARDING = "http://127.0.0.1:19041"


async def studio(page, out):
    await page.goto(f"{KILELE}/cakes/studio")
    await page.shot(f"{out}/studio-0.png")
    steps = []
    for label in ["2 tiers", "Gluten-free", "Vanilla", "Collection"]:
        chips = await page.eval(
            "[...document.querySelectorAll('button')].filter(b => b.getBoundingClientRect().width).map(b => b.textContent.trim().replace(/\\s+/g, ' ')).slice(0, 40)"
        )
        hit = next((c for c in chips if label.lower() in c.lower()), None)
        if not hit:
            steps.append(f"no chip for {label!r}; chips: {chips}")
            continue
        await page.click_text(hit.split(" KSh")[0][:24], "button")
        await asyncio.sleep(1.2)
        n = len([s for s in steps if s.startswith("clicked")]) + 1
        await page.shot(f"{out}/studio-{n}.png")
        steps.append(f"clicked {hit!r}")
    return steps


async def simple(page, out, name, url, settle=1.5, prep=None, scroll=0):
    await page.goto(url, settle=settle)
    if prep:
        await page.eval(prep)
    if scroll:
        await page.eval(f"window.scrollTo(0, {scroll})")
        await asyncio.sleep(0.4)
    await page.shot(f"{out}/{name}.png")
    return [f"{name}: {url}"]


# W4's report and Move fixtures are about fixture.example and kilele-bakery.example; in each film
# they are that business's own old site.
def as_host(host):
    slug = host.replace(".", "-")
    return f"""(() => {{
  document.querySelectorAll('.chip').forEach(c => {{ if (c.textContent.includes("fixture")) c.remove(); }});
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n; (n = w.nextNode());) n.nodeValue = n.nodeValue
    .replaceAll("kilele-bakery-example.agentrix.website", "{slug}.agentrix.website")
    .replaceAll("fixture.example", "{host}").replaceAll("kilele-bakery.example", "{host}");
  document.querySelectorAll('a[href]').forEach(a => a.textContent = a.textContent.replaceAll("kilele-bakery-example", "{slug}"));
  return true;
}})()"""


def typed(host):
    return f"""(() => {{ const i = document.querySelector('#url'); i.value = '{host}'; i.blur(); return true; }})()"""


AS_KILELE = as_host("kilele-bakery.example")
TYPED = typed("kilele-bakery.example")
CLASSY = "http://classy-perfumes-sample.localhost:19020"
TANA = "http://tana-valley-times-sample.localhost:19020"


async def phone(page, out, name, url, prep=None, wait=1.5):
    """A phone-sized capture (390x844 at 3x), for scenes where someone reads on their phone."""
    await page.viewport(390, 844, 3, mobile=True)
    try:
        await page.goto(url, settle=1.2)
        if prep:
            await page.eval(prep)
            await asyncio.sleep(wait)
        await page.shot(f"{out}/{name}.png")
    finally:
        await page.viewport(960, 1080, 2)
    return [f"{name}: {url} (phone)"]


def onboarding(film, host):
    """The check-up and the Move for one film's business, as jobs named film/assess-start and so on."""
    return {
        f"{film}/assess-start": lambda p, o: simple(p, o, f"{film}/assess-start", f"{ONBOARDING}/assess", prep=typed(host)),
        f"{film}/assess": lambda p, o: simple(p, o, f"{film}/assess", f"{ONBOARDING}/assess/done", prep=as_host(host)),
        f"{film}/move": lambda p, o: simple(p, o, f"{film}/move", f"{ONBOARDING}/move/live", prep=as_host(host)),
        f"{film}/move-left": lambda p, o: simple(p, o, f"{film}/move-left", f"{ONBOARDING}/move/live", prep=as_host(host), scroll=1030),
        f"{film}/move-rooms": lambda p, o: simple(p, o, f"{film}/move-rooms", f"{ONBOARDING}/move/live", prep=as_host(host), scroll=440),
    }


ASK = """(async () => {{
  const i = document.querySelector('#dockQ'); i.focus(); i.value = {q!r}; i.form.requestSubmit(); return true;
}})()"""


async def classy_plan(page, out):
    await page.goto(f"{CLASSY}/gifts/plan", settle=1)
    await page.click_text("Try Achieng", "button, a")
    await asyncio.sleep(0.8)
    await page.click_text("Draft the plan", "button")
    await asyncio.sleep(1.5)
    await page.eval("(() => { const r = [...document.querySelectorAll('h2, h3, .plan, [data-role=plan]')].find(e => /plan/i.test(e.textContent) && e.getBoundingClientRect().top > 400); if (r) r.scrollIntoView({block: 'start'}); return true; })()")
    await asyncio.sleep(0.5)
    await page.shot(f"{out}/classy/plan.png")
    return ["classy/plan: Achieng's plan, drafted"]


async def main(out, names):
    for d in ("", "classy", "tana", "salon"):
        os.makedirs(os.path.join(out, d), exist_ok=True)
    jobs = {
        "studio": studio,
        "site": lambda p, o: simple(p, o, "site", f"{KILELE}/"),
        "cakes": lambda p, o: simple(p, o, "cakes", f"{KILELE}/cakes"),
        "console": lambda p, o: simple(p, o, "console", CONSOLE, 2.5),
        "decisions": lambda p, o: simple(p, o, "decisions", f"{CONSOLE}/decisions", 2.5),
        "assess-start": lambda p, o: simple(p, o, "assess-start", f"{ONBOARDING}/assess", prep=TYPED),
        "assess": lambda p, o: simple(p, o, "assess", f"{ONBOARDING}/assess/done", prep=AS_KILELE),
        "move": lambda p, o: simple(p, o, "move", f"{ONBOARDING}/move/live"),
        "move-left": lambda p, o: simple(p, o, "move-left", f"{ONBOARDING}/move/live", scroll=1030),
        # Classy Perfumes
        "classy/site": lambda p, o: simple(p, o, "classy/site", f"{CLASSY}/"),
        "classy/search": lambda p, o: simple(p, o, "classy/search", f"{CLASSY}/search?q=a+gift+under+KSh+5000+for+someone+who+loves+vanilla"),
        "classy/plan": classy_plan,
        "classy/agents": lambda p, o: simple(p, o, "classy/agents", f"{CLASSY}/agents"),
        **onboarding("classy", "classy-perfumes.example"),
        # Tana Valley Times
        "tana/site": lambda p, o: simple(p, o, "tana/site", f"{TANA}/"),
        "tana/story": lambda p, o: simple(p, o, "tana/story", f"{TANA}/2026/11/council-approves-first-phase-of-the-northern-rail-extension#correction"),
        "tana/corrections": lambda p, o: simple(p, o, "tana/corrections", f"{TANA}/corrections"),
        "tana/digest": lambda p, o: simple(p, o, "tana/digest", f"{TANA}/digest"),
        "tana/agents": lambda p, o: simple(p, o, "tana/agents", f"{TANA}/agents"),
        "tana/catch-phone": lambda p, o: phone(p, o, "tana/catch-phone", f"{TANA}/", "document.querySelector('#cmBtn').click(), true", 2.5),
        "tana/ask-phone": lambda p, o: phone(p, o, "tana/ask-phone", f"{TANA}/", ASK.format(q="How much will phase one of the northern rail extension cost?"), 5),
        **onboarding("tana", "tana-valley-times.example"),
        # Nyota Salon (no sample site: its old site is Zavora's infected fixture)
        **onboarding("salon", "nyota-salon.example"),
    }
    with Browser(os.path.join(out, ".chrome"), port=9334, width=960, height=1080) as browser:
        page = await Page.open(browser)
        await page.viewport(960, 1080, 2)
        for name in names or list(jobs):
            try:
                print(json.dumps({name: await jobs[name](page, out)}))
            except Exception as e:  # keep going: one page failing shouldn't stop the rest
                print(json.dumps({name: f"failed: {e}"}))
        await page.close()


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], sys.argv[2:]))
