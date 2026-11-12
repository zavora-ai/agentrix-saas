"""Generates docs/diagrams/control-plane.svg in the style of generate.py.

Usage: python3 docs/diagrams/generate_control_plane.py docs/diagrams [more output dirs...]
It reuses the Svg helper and palette from generate.py without running that file's diagrams.
Extra output directories receive a copy (the concepts serve one from concept/control-plane/).
Render-check: headless Chrome --screenshot on the SVG.
"""
import shutil
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
_base = (HERE / "generate.py").read_text(encoding="utf-8").split("\ndef platform():", 1)[0]
_ns = {"__name__": "generate_base"}
exec(compile(_base, str(HERE / "generate.py"), "exec"), _ns)  # defines OUT from sys.argv[1], C, Svg
Svg, C = _ns["Svg"], _ns["C"]


def control_plane():
    s = Svg(1280, 1012, "Agentrix WebAgent Platform control plane",
            "Tenancy from operator to site; the people and surfaces; the change path that keeps coding agents "
            "governed; the global control layer; the operator tenant whose WebAgents run Zavora; and the cells, "
            "sandbox runner pools and Vultr account underneath.")
    s.text(24, 38, "Agentrix WebAgent Platform · the control plane", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "Zavora runs on Zavora: operator work is done by WebAgents under human authority. "
           "Blue = Vultr product · coral = holds secrets · amber dashed = sealed, untrusted work.", size=12, fill=C["muted"])

    # Row 1 · tenancy
    s.panel(24, 84, 380, 356, "TENANCY · DOWN ONLY BY SIGNED GRANT")
    tiers = [("Operator", ["Zavora Cloud, or a self-hoster", "fleet · platform levers · books"]),
             ("Partner", ["agency · reseller · referral", "white-label · roles by owner grant"]),
             ("Organisation", ["the business: owns its sites,", "data, receipts and billing"]),
             ("Site", ["TenantKernel · charter · WebAgents", "the site wall: SiteId everywhere"])]
    for i, (t, ls) in enumerate(tiers):
        s.box(44 + i * 14, 112 + i * 80, 340 - i * 14, 72, t, ls)
    for i in range(3):
        x = 52 + i * 14
        s.add(f'<path d="M{x},{184 + i * 80} V{198 + i * 80} H{x + 10}" fill="none" stroke="{C["grey"]}" stroke-width="1.4"/>')

    # Row 1 · people and surfaces
    s.panel(424, 84, 520, 356, "PEOPLE & SURFACES · ONE SHELL, THREE SCOPES")
    surf = [("Operator console", ["Zavora · Operator scope", "Fleet · Rollouts · Incidents", "Money · Partners · Trust"]),
            ("Portfolio console", ["agency or reseller scope", "white-label · client cards", "acts only by owner grant"]),
            ("Owner console", ["Site scope (doc 07)", "Today · Decisions · Trust", "operator actions shown here"]),
            ("agentrix CLI · API · MCP", ["one API, one authority model", "mutations become cards", "mcp serve: read + propose"]),
            ("On-call channels", ["WhatsApp: ack, evidence, extend", "push: a decision is waiting", "email: briefs, never approvals"]),
            ("Partners' systems", ["signed webhooks", "scoped, expiring tokens", "never database access"])]
    for i, (t, ls) in enumerate(surf):
        s.box(440 + (i % 2) * 250, 112 + (i // 2) * 106, 238, 96, t, ls, kind="actor" if i < 3 else "svc")

    # Row 1-2 · change path
    s.panel(964, 84, 292, 528, "CHANGE PATH · AGENTS PROPOSE")
    steps = [("Zavora CLI", ["worker + bounded planner · guard"], "svc"),
             ("Pull request", ["pack · theme · fix · fleet change"], "svc"),
             ("CI in sealed rooms", ["S1–S19 · gauntlet · evals"], "sealed"),
             ("Human review + merge", ["a merge is not an approval"], "actor"),
             ("Reconciler plans", ["agentrix-fleet · plan digest"], "svc"),
             ("Approval", ["passkey, or MFA step-up · 1 or 2 people"], "actor"),
             ("Effects Gateway applies", ["exactly once → receipt"], "key")]
    for i, (t, ls, k) in enumerate(steps):
        y = 112 + i * 70
        s.box(980, y, 260, 58, t, ls, kind=k)
        if i < len(steps) - 1:
            s.arrow(f"M1110,{y + 58} V{y + 69}")

    # Row 2 · global control layer
    s.panel(24, 460, 440, 260, "CONTROL PLANE · GLOBAL LAYER (ams-1, standby fra-1)")
    core = [("agentrix-control", ["accounts · orgs · partners", "grants · decisions · sign-in", "console + partner API"]),
            ("agentrix-fleet", ["registry · reconciler", "trains → rings → waves", "gates · rollback by pin"]),
            ("agentrix-billing", ["rating · invoices · dunning", "partner statements, payouts", "posts to zavora-era (eTIMS)"]),
            ("Operator ledger", ["hash-chained receipts", "every operator action", "matched to Vultr Audit Logs"])]
    for i, (t, ls) in enumerate(core):
        s.box(40 + (i % 2) * 212, 490 + (i // 2) * 110, 200, 98, t, ls)

    # Row 2 · operator tenant
    s.panel(484, 460, 460, 260, "OPERATOR TENANT · ZAVORA RUNS ON ZAVORA")
    s.chip(500, 490, "Operator Chief WebAgent")
    x = 500
    for n in ["SRE", "Release", "Capacity", "Billing", "Support"]:
        x += s.chip(x, 520, n) + 8
    x = 500
    for n in ["Growth", "Security & Abuse", "Partner Success", "Compliance"]:
        x += s.chip(x, 550, n) + 6
    s.text(500, 594, "Same governed path, plus an operator token (target × verb).", size=11.5, fill=C["muted"])
    s.text(500, 611, "Never read site content · never approve an owner's card.", size=11.5, weight=600, fill=C["coral"])
    s.box(500, 626, 206, 78, "Operator Model GW", ["own Inference subscription", "Zavora's cost, never a site's"], kind="key", title_size=13)
    s.box(718, 626, 210, 78, "Operator Effects GW", ["Vultr service users · payouts", "Zavora's email and WhatsApp"], kind="key", title_size=13)

    # Row 3 · cells
    s.panel(24, 744, 920, 254, "CELLS ON VULTR · ONE REGION'S STACK, ONE BLAST RADIUS")
    cells = [("ams-1 · Amsterdam", ["VKE: edge · control · kernels", "Postgres · Valkey · Kafka · CDN", "a TenantKernel per site"]),
             ("fra-1 · Frankfurt", ["standby: compute · DB replica", "no Object Storage in Frankfurt", "copies kept in Milan"]),
             ("jnb-1 · Johannesburg", ["edge cell for African sites", "kernels stay in ams-1"]),
             ("atl-0 · Atlanta", ["the first release · single node", "control VM + sandbox VM"])]
    for i, (t, ls) in enumerate(cells):
        s.box(40 + i * 224, 774, 212, 96, t, ls, kind="vultr", title_size=13)
    s.panel(40, 884, 888, 100, "SANDBOX RUNNER POOLS · NO SECRETS · OUTSIDE KUBERNETES", kind="sealed")
    x = 56
    for n in ["gVisor · any VM", "microVM · VX1", "throwaway VM · Vultr API"]:
        x += s.chip(x, 916, n, "sealed") + 8
    s.text(56, 966, "Migration Factory: one fair-share queue for owners and agencies · per-job egress allowlists · polite crawler fleet-wide", size=11.5, fill=C["muted"])

    # Row 2-3 · Vultr account
    s.panel(964, 632, 292, 366, "VULTR ACCOUNT")
    acct = [("Serverless Inference", ["a subscription per site,", "partner or operator tenant"]),
            ("IAM · sub-accounts", ["service users · assume-role", "zc-prod · zc-sandbox · dedicated"]),
            ("Audit Logs", ["matched to the operator ledger;", "an unmatched event pages Security"]),
            ("Marketplace · partners", ["single-node one-click listing", "Vultr partner programme"])]
    for i, (t, ls) in enumerate(acct):
        s.box(980, 662 + i * 82, 260, 72, t, ls, kind="vultr", title_size=13)

    # Arrows
    s.arrow("M600,440 V450 H190 V488", "one API · console, CLI, partners", 400, 450)
    s.arrow("M84,440 V488", "grants", 84, 452)
    s.arrow("M498,561 H456", color="indigo")
    s.arrow("M454,540 H474 V742", "trains → rings → waves", 474, 731)
    s.arrow("M1180,604 V660", "Vultr API · Terraform", 1180, 622, color="coral")
    s.arrow("M928,690 H952 V780 H978", color="coral")
    s.save("control-plane.svg")


control_plane()
for extra in sys.argv[2:]:
    shutil.copyfile(Path(sys.argv[1]) / "control-plane.svg", Path(extra) / "control-plane.svg")
print("ok")
