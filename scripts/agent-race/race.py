#!/usr/bin/env python3
"""The agent race (R9.3, S3-03): the same customer task, the old site through screenshots
versus the new site's agent door, side by side.

    python race.py --new https://kilele.agentrix.website            # the order race
    python race.py --new https://... --question "Do you bake gluten-free bread?"
    python race.py --only old                                        # just the old site
    python mock_door.py & python race.py --only new --new http://127.0.0.1:8765   # door test

Both agents are Claude Sonnet 5.5, called directly with ANTHROPIC_API_KEY from the repo's .env
(loaded into this process only, never printed). Results: a table on stdout, and
submission/race/race-<UTC time>.json plus the old site's final screenshot, for the video.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import shutil
import sys
import textwrap
import threading

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from common import MODEL, REPO, Run  # noqa: E402

TASK = "Order a 2 kg chocolate birthday cake for Saturday, with the message 'Happy 7th, Amani', for pickup."


def next_saturday(today: dt.date) -> dt.date:
    days = (5 - today.weekday()) % 7 or 7
    return today + dt.timedelta(days=days)


def prompt_for(task: str, question: str | None, name: str, email: str, phone: str, today: dt.date) -> str:
    if question:
        return (
            f"Your user's question: {question}\n"
            "Find the answer on this business's website. Answer only from what the site says, and say where "
            "you found it. If the site does not say, report that."
        )
    sat = next_saturday(today)
    return (
        f"Your user's task: {task}\n"
        f"Today is {today:%A %d %B %Y}; Saturday means {sat:%A %d %B %Y} ({sat.isoformat()}).\n"
        f"Your user's details, which you may give this business: name {name}, email {email}, phone {phone}.\n"
        "Place the order (or the closest thing the site offers, such as an order or contact form), and report "
        "exactly what the site confirmed."
    )


def table(old: Run | None, new: Run | None) -> str:
    def cell(run: Run | None, key: str) -> str:
        if run is None:
            return "-"
        u = run.usage
        return {
            "outcome": run.outcome,
            "wall": f"{run.wall_s:.1f} s",
            "steps": str(run.steps),
            "calls": str(u.calls),
            "tokens": f"{u.input + u.cache_write + u.cache_read:,} in / {u.output:,} out",
            "cost": f"${u.cost:.3f}",
            "final": run.final.get("screenshot")
            or json.dumps(run.final.get("held_call") or run.final.get("ask") or run.final.get("answer") or "")[:60],
        }[key]

    rows = [("Outcome", "outcome"), ("Wall time", "wall"), ("Steps", "steps"), ("Model calls", "calls"),
            ("Tokens", "tokens"), ("Model cost", "cost"), ("Final", "final")]
    w = 42
    out = [f"{'':14}{'OLD SITE (screenshots)':<{w}}{'NEW SITE (agent door)':<{w}}", "-" * (14 + 2 * w)]
    for label, key in rows:
        out.append(f"{label:14}{cell(old, key)[:w - 2]:<{w}}{cell(new, key)[:w - 2]:<{w}}")
    for side, run in (("OLD", old), ("NEW", new)):
        if run:
            out.append("")
            out.append(f"{side}: " + textwrap.shorten(run.summary or "(no summary)", 300))
            for warn in run.warnings:
                out.append(f"{side} warning: {warn}")
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--old", default=os.environ.get("RACE_OLD_URL", "https://fixture.agentrix.website/"),
                    help="the old site (default: Zavora's infected fixture)")
    ap.add_argument("--new", default=os.environ.get("RACE_NEW_URL"), help="the converted site with the agent door")
    ap.add_argument("--task", default=TASK)
    ap.add_argument("--question", help="the R9.3 fallback: race one cited question instead of the order")
    ap.add_argument("--name", default="Wanjiru Kamau", help="the customer's name the agents may give")
    ap.add_argument("--email", default="wanjiru.kamau@example.com")
    ap.add_argument("--phone", default="+254 700 000 007")
    ap.add_argument("--only", choices=["old", "new"])
    ap.add_argument("--max-steps", type=int, default=25)
    ap.add_argument("--effort", default="medium", choices=["low", "medium", "high"])
    ap.add_argument("--headed", action="store_true", help="show the old site's browser window")
    ap.add_argument("--out", default=str(REPO / "submission" / "race"))
    args = ap.parse_args()

    if args.only != "old" and not args.new:
        ap.error("--new <converted site URL> is required (or --only old)")
    out_dir = pathlib.Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    today = dt.date.today()
    mode = "question" if args.question else "order"
    prompt = prompt_for(args.task, args.question, args.name, args.email, args.phone, today)
    shown_task = args.question or args.task

    old = new = None
    threads = []
    if args.only != "new":
        from old_site import run_old

        old = Run("old", mode, args.old, shown_task)
        threads.append(threading.Thread(target=run_old, args=(old,), kwargs=dict(
            start_url=args.old, prompt=prompt, max_steps=args.max_steps, effort=args.effort,
            out_dir=out_dir, stamp=stamp, headed=args.headed)))
    if args.only != "old":
        from new_site import run_new

        new = Run("new", mode, args.new, shown_task)
        threads.append(threading.Thread(target=run_new, args=(new,), kwargs=dict(
            base_url=args.new, prompt=prompt, max_steps=args.max_steps, effort=args.effort)))
    print(f"Race at {stamp}: {shown_task}\nModel {MODEL}, effort {args.effort}, at most {args.max_steps} steps each.\n", flush=True)
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    print("\n" + table(old, new) + "\n")
    result = {
        "race": "R9.3 agent race",
        "at": stamp,
        "mode": mode,
        "task": shown_task,
        "customer": {"name": args.name, "email": args.email, "phone": args.phone},
        "model": MODEL,
        "effort": args.effort,
        "max_steps": args.max_steps,
        "old": old.as_dict() if old else None,
        "new": new.as_dict() if new else None,
        "notes": [
            "Steps: old = browser actions chosen by the model (plus finish); new = requests to the site "
            "(discovery, MCP initialize, tools/list, tool calls, status polls).",
            "Cost: Claude Sonnet 5.5 first-party rates ($2 in, $10 out, $2.50 cache write, $0.20 cache read per million tokens).",
            "The old site is Zavora's fixture: an infected WordPress with planted attacks, labelled as a fixture.",
        ],
    }
    path = out_dir / f"race-{stamp}.json"
    path.write_text(json.dumps(result, indent=2, default=str), encoding="utf-8")
    shutil.copyfile(path, out_dir / "latest.json")
    try:
        shown = path.relative_to(REPO)
    except ValueError:
        shown = path
    print(f"Wrote {shown} (and latest.json beside it)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
