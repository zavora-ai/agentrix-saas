"""Writes record.json: what the agents did during the unattended run, read back from the live build.

    python3 record.py [--since 2026-11-11T16:00:00Z] [--out record.json]

Nothing is typed in. Each event comes from a place a judge can open:
- orders and their payments: Kilele's sample owner view (the sign-in the sample fills in);
- settlements between the two companies and the bills: the two public consoles;
- each settlement's time, amount and state: read back from PayPal by its capture id (sandbox keys
  from the repository's .env, parsed, never printed);
- the wallet checkpoint: the Cashier's public badge.
Times are UTC in the file; replay.html shows them in East Africa Time.
"""

import argparse
import asyncio
import base64
import datetime as dt
import json
import os
import re
import sys
import tempfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "../../submission/video"))
sys.path.insert(0, os.path.join(HERE, "../../../crates/agentrix-home/film"))
import websockets  # noqa: E402
from cdp import Browser  # noqa: E402
from rec import Tab  # noqa: E402

P = os.environ.get("AGX_PLATFORM", "paypal.agentrix.website")
OWNER = f"https://{P}/samples/kilele-bakery-sample"
EAT = dt.timezone(dt.timedelta(hours=3))
MONTHS = {m: i + 1 for i, m in enumerate("Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split())}


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "agentrix-saas-record/1"})
    return urllib.request.urlopen(req, timeout=40).read().decode("utf-8", "replace")


def env():
    out = {}
    path = os.path.join(HERE, "../../../.env")
    if os.path.exists(path):
        for line in open(path):
            if "=" in line and not line.lstrip().startswith("#"):
                k, v = line.rstrip("\n").split("=", 1)
                out[k] = v.strip().strip('"').strip("'")
    return out


def paypal_capture_reader():
    e = env()
    tokens = []
    for cid, sec in (("PAYPAL_CLIENT_ID", "PAYPAL_CLIENT_SECRET"), ("KILELE_PAYPAL_CLIENT_ID", "KILELE_PAYPAL_CLIENT_SECRET")):
        if e.get(cid) and e.get(sec):
            req = urllib.request.Request("https://api-m.sandbox.paypal.com/v1/oauth2/token", data=b"grant_type=client_credentials",
                                         headers={"Authorization": "Basic " + base64.b64encode(f"{e[cid]}:{e[sec]}".encode()).decode(), "User-Agent": "agentrix-saas-record/1"})
            tokens.append(json.load(urllib.request.urlopen(req, timeout=40))["access_token"])

    def read(capture_id):
        for t in tokens:
            req = urllib.request.Request("https://api-m.sandbox.paypal.com/v2/payments/captures/" + capture_id,
                                         headers={"Authorization": "Bearer " + t, "User-Agent": "agentrix-saas-record/1"})
            try:
                return json.load(urllib.request.urlopen(req, timeout=40))
            except urllib.error.HTTPError:
                continue
        return None
    return read


def eat_stamp(text, now):
    """'Thu 12 Nov, 04:51' (East Africa Time, this year) as a UTC ISO string."""
    m = re.search(r"(\d{1,2}) ([A-Z][a-z]{2}), (\d{2}):(\d{2})", text)
    if not m:
        return None
    d = dt.datetime(now.year, MONTHS[m.group(2)], int(m.group(1)), int(m.group(3)), int(m.group(4)), tzinfo=EAT)
    return d.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


async def orders(since):
    out = []
    now = dt.datetime.now(EAT)
    with Browser(tempfile.mkdtemp(prefix="agx-record-"), port=9440, width=1400, height=900) as b:
        ws = await websockets.connect(b.ws_url(), max_size=None)
        tab = Tab(ws)
        reader = asyncio.ensure_future(tab.reader())
        await tab.send("Page.enable")
        await tab.send("Runtime.enable")
        await tab.goto(OWNER + "/orders")
        if await tab.js("location.pathname.endsWith('/sign-in')"):
            await tab.click(text="Sign in as Wanjiru")
            await tab.wait_for("!location.pathname.endsWith('/sign-in')", 20)
            await asyncio.sleep(1)
        rows = await tab.js("""[...document.querySelectorAll('tr')].map(r => [...r.querySelectorAll('td')].map(c => c.innerText.trim())).filter(c => c.length >= 6 && /Test customer agent/.test(c[2]))""")
        for r in rows:
            oid = r[0].split()[0]
            text = await tab.js(f"fetch({json.dumps(OWNER + '/orders/' + oid)}).then(x => x.text()).then(h => new DOMParser().parseFromString(h, 'text/html').querySelector('main').innerText)")
            flat = re.sub(r"\s+", " ", text)
            placed = re.search(r"Placed (\w{3} \d{1,2} \w{3}, \d{2}:\d{2})", flat) or re.search(r"Timeline (\w{3} \d{1,2} \w{3}, \d{2}:\d{2})", flat)
            at = eat_stamp(placed.group(1), now) if placed else None
            if not at or at < since:
                continue
            usd = re.search(r"USD (\d+\.\d{2}) at 1 USD", flat)
            paid = re.search(r"PayPal\s*USD (\d+\.\d{2})\s*captured\s*(\w{3} \d{1,2} \w{3}, \d{2}:\d{2})", flat)
            out.append({
                "kind": "order", "at": at, "order": oid, "agent": r[2].split(" (")[0], "items": r[3], "kes": r[5],
                "usd": float(usd.group(1)) if usd else None, "state": r[6],
                "door": "through the agent door" in flat, "verified": "verified by web-bot-auth" in flat,
                "paid_at": eat_stamp(paid.group(2), now) if paid else None, "paid_usd": float(paid.group(1)) if paid else None,
                "source": f"{OWNER}/orders/{oid}",
            })
        reader.cancel()
        await ws.close()
    return sorted(out, key=lambda e: e["at"])


def consoles(since):
    read = paypal_capture_reader()
    events = []
    kil = fetch(f"https://kilele.money.{P}/console")
    for inv, amount, cap in re.findall(r"<b>(INV-[\d-]+) · \$([\d.]+)</b> capture · PayPal (\w+)", kil):
        c = read(cap)
        if not c or c["create_time"] < since:
            continue
        events.append({"kind": "settlement", "at": c["create_time"], "invoice": inv, "usd": float(c["amount"]["value"]), "paypal": cap,
                       "state": c["status"], "payer": "Kilele Bakery", "payee": "Agentrix", "source": f"https://kilele.money.{P}/console"})
    refused = len(re.findall(r"A2A · refused: ", kil))
    for company, host, who in (("Kilele Bakery", f"kilele.money.{P}", "kilele"), ("Agentrix", f"money.{P}", "agentrix")):
        page = kil if who == "kilele" else fetch(f"https://{host}/console")
        for bill, to, due, status, amount in re.findall(r"<td[^>]*>([\w:.-]+:\d{8}T\d{6}Z-(?:\d{8}T\d{6}Z))</td>\s*<td[^>]*>([^<]+)</td>\s*<td[^>]*>([^<]+)</td>\s*<td[^>]*>[^<]*</td>\s*<td[^>]*>([^<]+)</td>\s*<td[^>]*>\$([\d.,]+)</td>", page):
            end = re.search(r"-(\d{8}T\d{6}Z)$", bill).group(1)
            at = dt.datetime.strptime(end, "%Y%m%dT%H%M%SZ").strftime("%Y-%m-%dT%H:%M:%SZ")
            if at < since:
                continue
            events.append({"kind": "bill", "at": at, "company": company, "bill": bill, "to": to.strip(), "due": due.strip(),
                           "status": status.strip(), "usd": float(amount.replace(",", "")), "source": f"https://{host}/console"})
    return events, refused


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--since", default="2026-11-11T16:00:00Z", help="the run began Wed 11 Nov 2026, 19:13 EAT")
    ap.add_argument("--out", default=os.path.join(HERE, "record.json"))
    a = ap.parse_args()
    ev = asyncio.run(orders(a.since))
    more, refused = consoles(a.since)
    ev = sorted(ev + more, key=lambda e: e["at"])
    badge = json.loads(fetch(f"https://kilele-bakery-sample.{P}/agents/cashier/badge.json"))
    rec = {
        "read_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "since": a.since,
        "sandbox": True,
        "wallet": {"entries": badge["wallet"]["sequence"], "reconciled": badge["wallet"]["reconciled"], "at": badge["wallet"]["reconciled_at"], "root": badge["wallet"]["root"]},
        "refusals_on_kilele_console": refused,
        "events": ev,
    }
    json.dump(rec, open(a.out, "w"), indent=1)
    with open(os.path.splitext(a.out)[0] + ".js", "w") as f:  # the same record, for replay.html opened from disk
        f.write("window.RECORD = " + json.dumps(rec) + ";\n")
    kinds = {}
    for e in ev:
        kinds[e["kind"]] = kinds.get(e["kind"], 0) + 1
    paid = [e for e in ev if e["kind"] == "order" and e.get("paid_usd")]
    print(f"{a.out}: {len(ev)} events {kinds}; {len(paid)} orders paid, USD {sum(e['paid_usd'] for e in paid):.2f}; "
          f"settled USD {sum(e['usd'] for e in ev if e['kind'] == 'settlement'):.2f}; wallet {rec['wallet']['entries']} entries, {rec['wallet']['reconciled']}")


if __name__ == "__main__":
    main()
