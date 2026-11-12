"""Counts each form field in specs/submission/form.md against lablab's limits.

    python3 specs/submission/count.py [specs/submission/form.md]

A field sits between a line "[field: NAME]" and a line "[end of field]".
"""
import re
import sys

LIMITS = {"title": (1, 50), "short": (50, 255), "long": (600, 2000)}

path = sys.argv[1] if len(sys.argv) > 1 else "specs/submission/form.md"
text = open(path, encoding="utf-8").read()
bad = 0
for name, body in re.findall(r"^\[field: (\S+)\]\n(.*?)\n\[end of field\]$", text, re.S | re.M):
    n = len(body.strip())
    lo, hi = LIMITS.get(name, (0, 10**9))
    ok = lo <= n <= hi
    bad += not ok
    print(f"{name}: {n} characters ({lo} to {hi}) {'ok' if ok else 'OUT OF RANGE'}")
sys.exit(1 if bad else 0)
