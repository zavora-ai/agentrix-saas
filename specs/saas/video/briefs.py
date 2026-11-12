"""Writes briefs.json and briefs.js: the latest brief in the day's shape that each company's
Chief sent its owner, word for word, read from the company's own store on the server (the `brief_owner` tool's call).

    python3 briefs.py

It reads over the deploy's SSH (deploy/saas/provision.sh wrote the settings), read-only.
"""
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SSH = ["ssh", "-F", os.path.expanduser("~/.config/agentrix-saas/deploy/ssh_config"), "-o", "BatchMode=yes", "saas-node"]


def latest(company):
    cmd = f"sudo -n sh -c \"strings -n 8 /var/lib/agentrix-saas/{company}/{company}.db /var/lib/agentrix-saas/{company}/{company}.db-wal 2>/dev/null | grep -a 'brief_owner' | tail -400\""
    out = subprocess.run(SSH + [cmd], capture_output=True, text=True, timeout=90).stdout
    found = re.findall(r'"name":"brief_owner","args":(\{"text":"(?:[^"\\]|\\.)*"\})', out)
    texts = [json.loads(f)["text"] for f in found]
    # The store isn't read in time order, so prefer a brief in the day's shape (People, Agents,
    # Today) over an earlier free-form one.
    shaped = [t for t in texts if "\nPeople" in t and "\nAgents" in t and "\nToday" in t]
    return shaped[-1] if shaped else None


def main():
    # Only briefs in the day's shape are kept: an earlier free-form brief can name a payee's
    # address, which doesn't belong in a public repository.
    briefs = {c: t for c in ("agentrix", "kilele") if (t := latest(c))}
    if not briefs:
        sys.exit("briefs: no brief in the day's shape yet")
    json.dump(briefs, open(os.path.join(HERE, "briefs.json"), "w"), indent=1, ensure_ascii=False)
    with open(os.path.join(HERE, "briefs.js"), "w") as f:
        f.write("window.BRIEFS = " + json.dumps(briefs, ensure_ascii=False) + ";\n")
    for c, t in briefs.items():
        print(f"--- {c}\n{t}\n")


if __name__ == "__main__":
    main()
