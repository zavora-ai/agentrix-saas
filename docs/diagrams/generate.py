"""Generates the platform's diagrams in docs/diagrams/ as standalone SVG files.

Usage: python3 docs/diagrams/generate.py docs/diagrams
Render-check: headless Chrome --screenshot on each SVG.
"""
import html
import sys
from pathlib import Path

OUT = Path(sys.argv[1])
FONT = "Inter, 'DM Sans', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif"
C = dict(
    ink="#1d1b33", muted="#5d5a78", indigo="#38356d", coral="#de4060", vultr="#0b6cff",
    vfill="#eef4ff", panel="#f6f5fb", line="#d8d5ea", amber="#c2620a", afill="#fff7ed",
    cfill="#fff1f3", white="#ffffff", grey="#9a97b0",
)


class Svg:
    def __init__(self, w, h, title, desc):
        self.w, self.h = w, h
        self.parts = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img" aria-labelledby="t d">',
            f'<title id="t">{html.escape(title)}</title><desc id="d">{html.escape(desc)}</desc>',
            "<defs>"
            f'<marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{C["indigo"]}"/></marker>'
            f'<marker id="ac" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{C["coral"]}"/></marker>'
            f'<marker id="aa" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{C["amber"]}"/></marker>'
            "</defs>",
            f'<rect width="{w}" height="{h}" fill="{C["white"]}"/>',
            f'<g font-family="{FONT}" fill="{C["ink"]}">',
        ]

    def add(self, s):
        self.parts.append(s)

    def text(self, x, y, s, size=12, weight=400, fill=None, anchor="start", spacing=0):
        fill = fill or C["ink"]
        ls = f' letter-spacing="{spacing}"' if spacing else ""
        self.add(f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}"{ls}>{html.escape(s)}</text>')

    def box(self, x, y, w, h, title, lines=(), kind="svc", r=12, tag=None, title_size=13.5):
        stroke, fill, dash = {
            "svc": (C["indigo"], C["white"], ""),
            "key": (C["coral"], C["cfill"], ""),
            "vultr": (C["vultr"], C["vfill"], ""),
            "sealed": (C["amber"], C["afill"], ' stroke-dasharray="6 4"'),
            "actor": (C["line"], C["white"], ""),
            "ghost": (C["line"], C["panel"], ' stroke-dasharray="4 4"'),
        }[kind]
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}" stroke="{stroke}" stroke-width="1.6"{dash}/>')
        self.text(x + 14, y + 24, title, size=title_size, weight=700,
                  fill={"key": C["coral"], "vultr": "#0752c4", "sealed": C["amber"]}.get(kind, C["indigo"] if kind == "svc" else C["ink"]))
        for i, line in enumerate(lines):
            self.text(x + 14, y + 44 + i * 17, line, size=11.5, fill=C["muted"])
        if tag:
            tw = 7.0 * len(tag) + 16
            self.add(f'<rect x="{x + w - tw - 10}" y="{y + 10}" width="{tw}" height="20" rx="10" fill="{C["vfill"]}" stroke="{C["vultr"]}" stroke-width="1"/>')
            self.text(x + w - tw / 2 - 10, y + 24, tag, size=10.5, weight=600, fill="#0752c4", anchor="middle")

    def panel(self, x, y, w, h, label, kind="panel"):
        if kind == "sealed":
            self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="#fffbf5" stroke="{C["amber"]}" stroke-width="1.4" stroke-dasharray="8 5"/>')
            self.text(x + 16, y + 22, label, size=11, weight=700, fill=C["amber"], spacing=1.2)
        else:
            self.add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" fill="{C["panel"]}" stroke="{C["line"]}" stroke-width="1.2"/>')
            self.text(x + 16, y + 22, label, size=11, weight=700, fill=C["muted"], spacing=1.2)

    def chip(self, x, y, s, kind="svc"):
        w = 7.1 * len(s) + 18
        stroke, fill, col = {"svc": (C["indigo"], "#f1f0fa", C["indigo"]), "sealed": (C["amber"], C["white"], C["amber"]),
                             "key": (C["coral"], C["white"], C["coral"]), "vultr": (C["vultr"], C["vfill"], "#0752c4")}[kind]
        self.add(f'<rect x="{x}" y="{y}" width="{w}" height="22" rx="11" fill="{fill}" stroke="{stroke}" stroke-width="1"/>')
        self.text(x + w / 2, y + 15, s, size=11, weight=600, fill=col, anchor="middle")
        return w

    def arrow(self, d, label=None, lx=0, ly=0, color="indigo", dashed=False):
        col = C[color]
        marker = {"indigo": "a", "coral": "ac", "amber": "aa"}[color]
        dash = ' stroke-dasharray="5 4"' if dashed else ""
        self.add(f'<path d="{d}" fill="none" stroke="{col}" stroke-width="1.6"{dash} marker-end="url(#{marker})"/>')
        if label:
            w = 6.4 * len(label) + 12
            self.add(f'<rect x="{lx - w / 2}" y="{ly - 12}" width="{w}" height="18" rx="9" fill="{C["white"]}" stroke="{C["line"]}" stroke-width="0.8"/>')
            self.text(lx, ly + 1, label, size=10.5, weight=600, fill=col, anchor="middle")

    def save(self, name):
        self.add("</g></svg>")
        (OUT / name).write_text("\n".join(self.parts) + "\n", encoding="utf-8")


def platform():
    s = Svg(1280, 900, "Agentrix WebAgent Platform architecture",
            "Edge, control and agent planes, two key holders, data stores and a separate sandbox plane, all on Vultr.")
    s.text(24, 38, "Agentrix WebAgent Platform · architecture", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "One Rust workspace on adk-rust v3, running on Vultr. Blue = Vultr product · coral = holds secrets · amber dashed = sealed, untrusted work.", size=12, fill=C["muted"])
    # actors
    s.box(24, 90, 192, 76, "Owner & team", ["Console PWA · WhatsApp", "passkey-signed approvals"], kind="actor")
    s.box(24, 190, 192, 60, "Visitors", ["pages · Ask this site"], kind="actor")
    s.box(24, 272, 192, 60, "AI agents", ["AWP · MCP · A2A door"], kind="actor")
    # edge
    s.panel(240, 82, 760, 112, "EDGE")
    s.box(260, 114, 200, 66, "CDN · Load Balancer", ["34 edges · TLS"], kind="vultr")
    s.box(500, 114, 480, 66, "agentrix-edge", ["signed pages · AWP twins · Ask (SSE) · agent door", "bot identity tiers · rate limits · release cache"])
    # control + agents
    s.panel(240, 212, 760, 202, "CONTROL & WEBAGENTS")
    s.box(260, 242, 300, 156, "agentrix-control", ["accounts · sites · plans · billing", "console API (REST + SSE)", "decisions inbox · passkeys", "LaunchGate · plain-words activity", "adk-surface · adk-harness"])
    s.box(580, 242, 400, 156, "agentrix-kernel", ["one TenantKernel per site · its WebAgents · adk-kernel"])
    x = 594
    for n in ["Chief WebAgent", "Concierge", "Sales", "Editor"]:
        x += s.chip(x, 300, n) + 8
    x = 594
    for n in ["Reputation", "Cashier", "Guardian", "Caretaker"]:
        x += s.chip(x, 330, n) + 8
    s.chip(594, 360, "Mover (migration)")
    # key holders
    s.box(1024, 84, 232, 92, "Vultr Serverless Inference", ["qwen3.8-27b · glm-5.3", "nemotron safety · systemone", "embeddings · vision · voice"], kind="vultr", title_size=13)
    s.box(1024, 200, 232, 92, "agentrix-models", ["Model Gateway", "only holder of inference keys", "routing · budgets · screening"], kind="key")
    s.box(1024, 314, 232, 92, "agentrix-effects", ["Effects Gateway", "only holder of provider keys", "verifies signed approvals"], kind="key")
    s.box(1024, 430, 232, 76, "Providers", ["M-Pesa · PayPal · Stripe", "WhatsApp · email · DNS"], kind="actor")
    # data
    s.panel(240, 432, 760, 112, "DATA")
    s.box(260, 462, 210, 66, "Managed PostgreSQL", ["+ pgvector · RLS on site_id"], kind="vultr", title_size=13)
    s.box(490, 462, 210, 66, "Managed Valkey", ["queues · limits · sessions"], kind="vultr", title_size=13)
    s.box(720, 462, 260, 66, "Object Storage (CAS)", ["signed releases · evidence · archives"], kind="vultr", title_size=13)
    # sandbox plane
    s.panel(240, 572, 1016, 196, "SANDBOX PLANE · NO SECRETS", kind="sealed")
    s.text(1240, 594, "separate VX1 hosts · own VPC · reachable only by the broker", size=11, weight=600, fill=C["amber"], anchor="end")
    s.box(260, 604, 240, 140, "agentrix-sandbox", ["broker · typed jobs:", "fetch · render · replay", "exec · build", "warm pools · per-site caps"])
    s.box(520, 604, 340, 140, "agentrix-runner · sealed rooms", [], kind="sealed")
    s.chip(534, 640, "gVisor · any VM", "sealed")
    s.chip(664, 640, "microVM · VX1", "sealed")
    s.chip(534, 670, "throwaway VM · Vultr API", "sealed")
    s.text(534, 718, "fresh per job · caps · destroyed ✓", size=11.5, fill=C["muted"])
    s.text(534, 735, "signed receipt per job", size=11.5, fill=C["muted"])
    s.box(880, 604, 356, 140, "agentrix-egress · per-host proxy", ["per-job origin allowlist", "DNS pinning · refuses private ranges", "and 169.254.169.254 (instance metadata)", "every request logged into the receipt"], kind="sealed")
    s.box(880, 800, 356, 74, "Legacy site · open web", ["WordPress · Wix · Shopify", "read-only · never admin credentials"], kind="actor")
    # legend
    s.text(24, 380, "LEGEND", size=11, weight=700, fill=C["muted"], spacing=1.2)
    s.box(24, 392, 192, 40, "Platform service", kind="svc", title_size=12)
    s.box(24, 442, 192, 40, "Holds secrets", kind="key", title_size=12)
    s.box(24, 492, 192, 40, "Vultr product", kind="vultr", title_size=12)
    s.box(24, 542, 192, 40, "Sealed / untrusted", kind="sealed", title_size=12)
    # arrows
    s.arrow("M216,214 C236,214 236,150 258,150")
    s.arrow("M216,300 C240,300 240,166 258,166")
    s.arrow("M216,128 H230 V318 H258")
    s.arrow("M460,147 H498")
    s.arrow("M780,180 V240", "Ask", 806, 212)
    s.arrow("M560,320 H578")
    s.arrow("M980,160 H994 V495 H982", "releases", 1000, 420)
    s.arrow("M980,266 H1022", "purpose", 1003, 252, color="coral")
    s.arrow("M980,370 H1022", "effects", 1003, 392, color="coral")
    s.arrow("M1140,200 V178", color="coral")
    s.arrow("M1140,406 V428", color="coral")
    s.arrow("M365,398 V460")
    s.arrow("M480,398 V602", "jobs", 480, 560)
    s.arrow("M830,604 V530", "artifacts by digest", 760, 562, color="amber")
    s.arrow("M860,674 H878", color="amber")
    s.arrow("M1058,744 V798", "allowlisted only", 1058, 774, color="amber")
    s.save("platform-architecture.svg")


def walls():
    s = Svg(1280, 640, "The four walls",
            "How untrusted input, agents, releases and effects are separated: action wall, site wall, release wall and owner wall.")
    s.text(24, 38, "Four walls · what a tricked WebAgent cannot do", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "A compromised WebAgent can waste a bounded budget or propose something a gate refuses. It cannot steal a key, publish, or touch another site.", size=12, fill=C["muted"])
    s.text(24, 100, "UNTRUSTED WORLD", size=11, weight=700, fill=C["muted"], spacing=1.2)
    for i, (t, l) in enumerate([("Legacy site", "possibly compromised"), ("Open web pages", "hidden instructions, redirects"), ("Visitor messages", "and uploads"), ("Model-written code", "and theme builds")]):
        s.box(24, 112 + i * 98, 190, 78, t, [l], kind="actor")
    # action wall
    s.panel(240, 84, 210, 500, "1 · ACTION WALL", kind="sealed")
    s.box(256, 132, 178, 332, "Sealed room", ["no keys", "egress: allowlist only", "no private ranges", "no instance metadata", "caps: cpu · mem · pids", "time limit", "destroyed after", "signed receipt"], kind="sealed")
    s.text(256, 500, "gVisor · microVM (VX1)", size=11.5, fill=C["muted"])
    s.text(256, 518, "· throwaway VM", size=11.5, fill=C["muted"])
    for i in range(4):
        s.arrow(f"M214,{151 + i * 98} H254", color="amber")
    # site wall
    s.panel(476, 84, 340, 500, "2 · SITE WALL")
    s.box(492, 120, 308, 200, "Kilele Bakery · TenantKernel", ["own budget · own inference subscription", "own keys namespace · RLS on site_id"])
    x = 506
    for n in ["Chief WebAgent", "Concierge", "Sales"]:
        x += s.chip(x, 186, n) + 6
    x = 506
    for n in ["Editor", "Cashier", "Guardian"]:
        x += s.chip(x, 216, n) + 6
    s.text(506, 270, "reads sanitised, screened data only", size=11.5, fill=C["muted"])
    s.text(506, 288, "proposes; never publishes or pays", size=11.5, fill=C["muted"])
    s.add(f'<line x1="492" y1="346" x2="800" y2="346" stroke="{C["coral"]}" stroke-width="1.6" stroke-dasharray="6 5"/>')
    s.text(646, 340, "no reads, spends or approvals across", size=11, weight=600, fill=C["coral"], anchor="middle")
    s.box(492, 360, 308, 110, "Another business · TenantKernel", ["isolated by construction", "cross-site request → denied + audited"], kind="ghost")
    s.arrow("M434,282 H490", "data only", 462, 268, color="amber")
    # release wall
    s.panel(842, 84, 180, 236, "3 · RELEASE WALL")
    s.box(856, 116, 152, 188, "LaunchGate", ["approvals", "parity ≥ 0.95", "policy · scans", "Ed25519 signs", "atomic pointer", "1-tap rollback"])
    s.box(1048, 116, 208, 92, "Edge", ["serves signed releases only", "tamper is detectable by anyone"], kind="svc")
    s.box(1048, 226, 208, 70, "Visitors & AI agents", ["same truth for both"], kind="actor")
    s.arrow("M800,200 H854", "candidate", 827, 186)
    s.arrow("M1008,160 H1046")
    s.arrow("M1152,208 V224")
    # owner wall
    s.panel(842, 348, 180, 236, "4 · OWNER WALL")
    s.box(856, 380, 152, 188, "Signed approval", ["passkey over the", "effect fingerprint", "standing rules", "budget caps", "freeze switch", "exactly once"], kind="key")
    s.box(1048, 380, 208, 104, "Key holders", ["Model Gateway · Effects Gateway", "the only places with secrets", "agents hold capabilities"], kind="key")
    s.box(1048, 504, 208, 70, "Providers", ["payments · messages · DNS"], kind="actor")
    s.arrow("M800,296 H828 V452 H854", "effects", 829, 380, color="coral")
    s.arrow("M1008,440 H1046", color="coral")
    s.arrow("M1152,484 V502", color="coral")
    s.text(24, 616, "Every crossing leaves a receipt in the site log: hash-chained, signed, and readable in plain words in the owner's Trust tab.", size=12, fill=C["muted"])
    s.save("trust-walls.svg")


def deployment():
    s = Svg(1280, 720, "Deployment on Vultr",
            "Single-node profile for the first release and self-hosting, and the fleet profile for the managed service.")
    s.text(24, 38, "Deployment on Vultr · one codebase, two profiles", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "Single-node runs the hackathon entry and Marketplace self-host. Fleet runs Zavora Cloud. Same binaries, different stores.", size=12, fill=C["muted"])
    # single node
    s.panel(24, 80, 600, 620, "SINGLE-NODE · the first release · self-host · Marketplace · Vultr Atlanta")
    s.box(44, 112, 270, 330, "control VM", ["VX1 or High Frequency", "", "agentrix-edge", "agentrix-control", "agentrix-kernel", "agentrix-models (Model Gateway)", "agentrix-effects (Effects Gateway)", "sandbox broker", "Caddy (TLS) · SQLite · local CAS", "", "firewall group: in 80/443 only", "to sandbox VM: private VPC, mTLS"])
    s.box(336, 112, 268, 330, "sandbox VM", ["VX1 · nested virtualisation", "", "agentrix-runner", "agentrix-egress", "gVisor (runsc)", "Microsandbox microVMs", "no secrets · no public inbound", "in: broker port from control only", "out: proxy only (nftables)", "", "destroyed rooms, signed receipts"], kind="sealed")
    s.arrow("M314,300 H334", color="amber")
    s.box(44, 486, 180, 70, "Serverless Inference", ["six model roles"], kind="vultr", title_size=12.5)
    s.box(236, 486, 180, 70, "Object Storage", ["releases · evidence"], kind="vultr", title_size=12.5)
    s.box(428, 486, 176, 70, "Firewall groups", ["inbound only"], kind="vultr", title_size=12.5)
    s.box(44, 576, 560, 104, "Why two VMs, even for a hackathon", ["The host that renders hostile pages and replays compromised WordPress sites", "holds no key and cannot reach the control plane. A sandbox escape finds", "nothing to steal. Egress is enforced on the host because Vultr firewall", "groups filter inbound traffic only."], kind="actor", title_size=12.5)
    s.arrow("M180,442 V484", color="coral")
    s.arrow("M220,442 C220,470 300,460 320,484")
    # fleet
    s.panel(656, 80, 600, 620, "FLEET · Zavora Cloud · Amsterdam/Frankfurt + Johannesburg edge")
    s.box(676, 112, 270, 66, "CDN pull zone", ["custom domains · purge on rollback"], kind="vultr", title_size=12.5)
    s.box(966, 112, 270, 66, "Load Balancer · DNS API", ["ACME for customer domains"], kind="vultr", title_size=12.5)
    s.box(676, 198, 560, 150, "VKE · free control plane · autoscaled node pools", [], kind="vultr", title_size=13)
    x = 692
    for n in ["edge pool", "control pool", "kernel pool", "gateways pool"]:
        x += s.chip(x, 240, n) + 10
    s.text(692, 296, "TenantKernels sleep for free and wake on signals; gateways are HA.", size=11.5, fill=C["muted"])
    s.text(692, 314, "Signed images from Container Registry · Terraform (Vultr provider).", size=11.5, fill=C["muted"])
    s.box(676, 366, 560, 70, "VX1 runner pool · outside Kubernetes", ["scaled by the broker through the Vultr API · throwaway VMs for risky replays"], kind="sealed", title_size=13)
    s.box(676, 454, 178, 74, "Managed PostgreSQL", ["pgvector · replicas · PITR"], kind="vultr", title_size=12)
    s.box(866, 454, 178, 74, "Managed Valkey", ["queues · limits"], kind="vultr", title_size=12)
    s.box(1056, 454, 180, 74, "Object Storage", ["CAS · Archive tier"], kind="vultr", title_size=12)
    s.box(676, 546, 178, 70, "Serverless Inference", ["a subscription per site"], kind="vultr", title_size=12)
    s.box(866, 546, 178, 70, "IAM · Audit Logs", ["assume-role · no host keys"], kind="vultr", title_size=12)
    s.box(1056, 546, 180, 70, "Managed Kafka", ["events at scale"], kind="vultr", title_size=12)
    s.box(676, 630, 560, 54, "Later: Cloud GPU for in-region models · Marketplace one-click listing · partner programme", [], kind="ghost", title_size=12)
    s.save("vultr-deployment.svg")




def anatomy():
    s = Svg(1280, 812, "Anatomy of a WebAgent",
            "How one WebAgent wakes, reasons, proposes a tool call, passes the governed path, acts through a gateway, remembers and sleeps again.")
    s.text(24, 38, "Anatomy of a WebAgent · one governed loop", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "A WebAgent is a durable kernel process running a HarnessAgent. It holds capabilities, never credentials, and sleeps for free between wakes.", size=12, fill=C["muted"])
    # wake sources
    s.panel(24, 84, 200, 508, "WAKE SOURCES")
    wakes = [("Owner message", "paired phone · WhatsApp"), ("Visitor or AI agent", "public desks only"),
             ("Typed signal", "from the public desks"), ("Schedule", "brief · probes · reviews"),
             ("Webhook or queue", "orders · payments · email"), ("Approval decided", "the owner signed a card"),
             ("Run finished", "another job completed"), ("Budget replenished", "a new day's allowance")]
    for i, (t, l) in enumerate(wakes):
        y = 112 + i * 58
        s.box(38, y, 172, 52, t, [l], kind="actor", title_size=12.5)
        s.add(f'<line x1="210" y1="{y + 26}" x2="236" y2="{y + 26}" stroke="{C["indigo"]}" stroke-width="1.4"/>')
    s.add(f'<line x1="236" y1="138" x2="236" y2="544" stroke="{C["indigo"]}" stroke-width="1.4"/>')
    # kernel
    s.panel(248, 84, 220, 508, "TENANTKERNEL · ADK-KERNEL")
    s.box(262, 112, 192, 74, "Wake router", ["WakeSet · Trigger", "one run per event key"])
    s.box(262, 204, 192, 92, "Admission", ["Quotas · QuietHours", "daily budget · freeze", "human turns go first"])
    s.box(262, 314, 192, 92, "Durable run", ["process = run + lease", "checkpoint per step", "action ledger"])
    s.box(262, 424, 192, 74, "Asleep", ["Waiting · no lease", "no model call · no cost"], kind="ghost")
    s.text(262, 524, "freeze parks every process", size=11.5, fill=C["muted"])
    s.text(262, 541, "at its next safe point", size=11.5, fill=C["muted"])
    s.arrow("M236,149 H260")
    s.arrow("M358,186 V202")
    s.arrow("M358,296 V312")
    # loop
    s.panel(492, 84, 320, 508, "THE LOOP · HARNESSAGENT")
    s.box(516, 112, 280, 74, "1 · Assemble context", ["role prompt · playbook skill · charter terms", "harness manifest · memory facts · task"])
    s.box(516, 202, 280, 58, "2 · Reason", ["Model Gateway · named purpose · metered"])
    s.box(516, 274, 280, 58, "3 · Propose a tool call", ["typed arguments · declared ToolEffect"])
    s.box(516, 346, 280, 58, "4 · Governed path decides", ["allow · hold for the owner · deny"])
    s.box(516, 418, 280, 58, "5 · Observe and checkpoint", ["result to the model · loop or finish"])
    s.box(516, 490, 280, 74, "Finish", ["a draft · a decision card · a signal", "a reply · a receipt · then sleep"])
    for y in (186, 260, 332, 404, 476):
        s.arrow(f"M656,{y} V{y + 14}")
    s.arrow("M516,447 H502 V231 H514", "loop", 502, 340)
    s.arrow("M454,360 H480 V149 H514", "claim", 480, 196)
    s.arrow("M516,527 H480 V461 H456", "sleep", 480, 494)
    # effects
    s.panel(836, 84, 420, 188, "WHERE ACTIONS GO")
    s.box(852, 112, 192, 62, "Model Gateway", ["only inference keys"], kind="key")
    s.box(1054, 112, 186, 62, "Effects Gateway", ["fingerprint = idempotency"], kind="key")
    s.box(852, 186, 192, 72, "Sandbox plane", ["fetch · render · replay", "build · no secrets"], kind="sealed")
    s.box(1054, 186, 186, 72, "Release builder", ["candidate release", "the LaunchGate signs"])
    s.arrow("M796,231 H822 V143 H850", "purpose", 822, 186, color="coral")
    # governed path
    s.panel(836, 290, 420, 302, "GOVERNED PATH · STRICTEST WINS")
    layers = ["HarnessPolicy · mode table", "LadderPolicy · job level", "JobPauses · guardrails",
              "PartitionPolicy · crossings", "DriverPolicy · pinned tools", "Platform · capability token"]
    for i, t in enumerate(layers):
        s.box(852, 318 + i * 42, 204, 34, t, kind="svc", title_size=11.5, r=9)
    s.box(1070, 318, 170, 74, "Allow", ["action ledger first", "then it runs once"])
    s.box(1070, 402, 170, 74, "Hold", ["decision card", "owner signs with passkey"], kind="key")
    s.box(1070, 486, 170, 74, "Deny", ["the reason goes back", "to the model"], kind="actor")
    s.arrow("M796,375 H850")
    s.arrow("M1056,355 H1068")
    s.arrow("M1056,439 H1068", color="coral")
    s.arrow("M1056,523 H1068")
    s.arrow("M1155,318 V260", "runs", 1182, 289)
    # memory
    s.panel(24, 616, 1232, 152, "MEMORY · SCOPED TO THE WEBAGENT'S PARTITION AND SITE")
    mem = [("Session", ["adk-session transcript", "checkpoints · resumable"]),
           ("Long-term facts", ["Assumed until the owner", "confirms them as Known"]),
           ("Site corpus", ["the signed release's twins", "all the public desks read"]),
           ("Playbooks", ["a versioned skill per job", "the owner approves changes"]),
           ("/proc ledger", ["content-free activity rows", "hashed subjects only"]),
           ("Receipts", ["hash-chained · signed", "plain words in Trust"])]
    for i, (t, ls) in enumerate(mem):
        s.box(42 + i * 201, 646, 192, 96, t, ls)
    s.arrow("M656,564 V644", "recall · remember", 708, 604)
    s.text(24, 796, "A call outside the WebAgent's jobs is refused, a sleeping WebAgent holds no lease, and every effect runs once, through a gateway, with a receipt.", size=12, fill=C["muted"])
    s.save("webagent-anatomy.svg")


def team():
    s = Svg(1280, 936, "A site's WebAgents",
            "Two partitions joined only by typed signals; a Chief WebAgent leads the back office; owners decide on any surface; only signed packs and aggregates cross between sites.")
    s.text(24, 38, "A site's WebAgents · two partitions, one Chief WebAgent", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "Kilele Bakery's TenantKernel. Untrusted text stays in the public partition; the back office sees typed signals only. The owner decides on any surface.", size=12, fill=C["muted"])
    # actors
    s.box(24, 108, 180, 66, "Visitors", ["Ask this site · forms"], kind="actor")
    s.box(24, 192, 180, 66, "AI agents", ["AWP · MCP · A2A"], kind="actor")
    s.box(24, 276, 180, 66, "Email and social", ["messages · reviews"], kind="actor")
    # public partition
    s.panel(228, 84, 300, 436, "PUBLIC PARTITION · UNTRUSTED INPUT")
    s.box(244, 114, 268, 196, "Concierge · its desks", [])
    x = 258
    for n in ["Ask desk", "Agent door", "Inbox"]:
        x += s.chip(x, 150, n) + 8
    for i, l in enumerate(["+ one section desk per menu item", "search_site over the signed release", "declared capabilities only", "reply_in_thread, to the sender only", "emit_signal: typed and bounded", "no publish · no spend · no web fetch"]):
        s.text(258, 196 + i * 18, l, size=11.5, fill=C["muted"] if i else C["indigo"])
    s.box(244, 326, 268, 92, "Agent edge · AwpEdge", ["trust tiers · schemas · idempotency", "consent receipts · held calls", "AI disclosure on every reply"])
    s.box(244, 432, 268, 74, "Mode: the manifest's policy", ["no standing rules", "no owner memory (A-I2)"], kind="ghost", title_size=12.5)
    s.arrow("M204,141 H242")
    s.arrow("M204,225 H242")
    s.arrow("M204,300 H242")
    # signals
    s.text(548, 104, "TYPED SIGNALS", size=11, weight=700, fill=C["muted"], spacing=1.2)
    for i, n in enumerate(["lead_captured", "content_gap", "review_seen", "support_escalation", "visitor_intent", "agent_interaction", "abuse_observed", "inbound_message"]):
        s.chip(548, 116 + i * 30, n)
    s.text(548, 376, "bounded · screened", size=11.5, fill=C["muted"])
    s.text(548, 393, "no raw text (A-I3)", size=11.5, fill=C["muted"])
    s.arrow("M528,230 H546")
    s.arrow("M700,151 H734")
    # back office
    s.panel(720, 84, 536, 436, "BACK OFFICE · CHARTER AUTHORITY PER JOB")
    s.box(736, 114, 200, 74, "Agenda · SignalSink", ["counted, rate-limited", "wakes the owning WebAgent"])
    s.box(956, 114, 284, 74, "Chief WebAgent", ["leads the team · morning brief", "weekly review · answers the owner"])
    s.arrow("M936,151 H954")
    specialists = [("Sales", ["nurture · handoff · quotes", "bookings with consent"]),
                   ("Editor", ["freshness · SEO · AEO", "languages · experiments"]),
                   ("Reputation", ["replies to reviews", "and mentions, your voice"]),
                   ("Cashier", ["orders · payments", "refunds over threshold"]),
                   ("Guardian", ["probes · drift · alarms", "rollback, reversible only"]),
                   ("Caretaker", ["domains · certificates", "backups · running costs"])]
    for i, (t, ls) in enumerate(specialists):
        col, row = i % 3, i // 3
        s.box(736 + col * 170, 240 + row * 92, 160, 78, t, ls)
    s.add(f'<path d="M1098,188 V214 M816,214 H1156" fill="none" stroke="{C["indigo"]}" stroke-width="1.4"/>')
    for cx in (816, 986, 1156):
        s.arrow(f"M{cx},214 V238")
    s.text(1112, 208, "delegate · depth 1", size=10.5, weight=600, fill=C["indigo"])
    s.box(736, 432, 500, 74, "Mover · during the Move only", ["claim → replay → clean → rebuild → prove → go live", "then hands each job to its WebAgent and retires"], kind="ghost", title_size=12.5)
    # surfaces
    s.panel(24, 544, 1232, 120, "OWNER SURFACES · ADK-SURFACE · SURFACES HOLD NO STATE")
    surf = [("Console PWA", ["Today · Decisions · WebAgents", "passkey for consequential cards"]),
            ("WhatsApp", ["brief · Allow once at low risk", "never money, never high risk"]),
            ("Email", ["morning brief · weekly review", "a link, never an approval"]),
            ("Voice", ["ask · deny · stop all", "never approves money"])]
    for i, (t, ls) in enumerate(surf):
        s.box(42 + i * 302, 572, 290, 78, t, ls)
    s.arrow("M988,520 V570", "cards · briefs · alarms", 1062, 532)
    # gateways
    s.panel(24, 688, 1232, 108, "WHAT THE BACK OFFICE ACTS THROUGH")
    gw = [("Model Gateway", ["purpose routing · budgets"], "key"), ("Effects Gateway", ["signed approval · exactly once"], "key"),
          ("LaunchGate", ["parity · scans · Ed25519 sign"], "svc"), ("Sandbox plane", ["sealed rooms · no secrets"], "sealed")]
    for i, (t, ls, k) in enumerate(gw):
        s.box(42 + i * 302, 716, 290, 62, t, ls, kind=k)
    # fleet
    s.panel(24, 818, 1232, 104, "FLEET · AGENCY OR ZAVORA CLOUD")
    for i in range(3):
        s.box(42 + i * 170, 846, 158, 58, "Another site", ["its own TenantKernel"], kind="ghost", title_size=12.5)
    s.add(f'<line x1="552" y1="846" x2="552" y2="904" stroke="{C["coral"]}" stroke-width="1.6" stroke-dasharray="6 5"/>')
    s.text(572, 866, "Between sites only signed packs, playbook revisions, detector updates", size=11.5, weight=600, fill=C["coral"])
    s.text(572, 884, "and k-anonymous aggregates move. No conversation, lead, fact or prompt does (A-I22).", size=11.5, weight=600, fill=C["coral"])
    s.save("webagent-team.svg")


platform()
walls()
deployment()
anatomy()
team()
print("ok")
