"""Generates docs/diagrams/data-architecture.svg in the style of generate.py.

Usage: python3 docs/diagrams/generate_data.py docs/diagrams [more output dirs...]
It reuses the Svg helper and palette from generate.py without running that file's diagrams.
Extra output directories receive a copy (concept/data/ serves one).
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


def data_architecture():
    s = Svg(1280, 1090, "Agentrix WebAgent Platform data architecture",
            "Where each class of data lives in both profiles, what may cross the content-free boundary, "
            "the outbox to Kafka to lakehouse path, the observability pipeline, the visualisation surfaces, "
            "and the lifecycle controls (erasure, retention, backup, residency) that reach every store.")
    s.text(24, 38, "Agentrix WebAgent Platform · data architecture", size=21, weight=800, fill=C["indigo"])
    s.text(24, 60, "Personal data stays inside its site's stores; only content-free rows cross the boundary. "
           "Blue = Vultr product · coral = holds keys · amber dashed = sealed, untrusted work.", size=12, fill=C["muted"])

    # Column 1 · producers
    s.panel(24, 84, 262, 648, "PRODUCERS")
    prods = [("Visitors & AI agents", ["pages from the CDN (no data)", "Ask · artifacts · requests"], "actor"),
             ("Owner & team", ["console · WhatsApp · email", "decisions · facts · rules"], "actor"),
             ("WebAgents", ["TenantKernel per site", "runs · drafts · agenda · ledger"], "svc"),
             ("Model Gateway", ["reserve and settle per call", "usage ledger · no prompts kept"], "key"),
             ("Effects Gateway", ["receipts for every effect", "orders · payments · sends"], "key"),
             ("Sealed rooms", ["crawl · replay · render", "evidence by digest only"], "sealed"),
             ("Owner's tools via MCP", ["CRM · calendar · books · mail", "read per run · never mirrored"], "ghost")]
    for i, (t, ls, k) in enumerate(prods):
        s.box(40, 112 + i * 87, 230, 78, t, ls, kind=k, title_size=13)

    # Column 2 · inside the site wall
    s.panel(306, 84, 500, 648, "INSIDE THE SITE WALL · ONE SITE · C0–C3")
    stores = [("Records", ["conversations · leads · orders", "facts · drafts · kernel runs", "1-node: SQLite file per site", "fleet: Postgres schema + role"], "svc"),
              ("Vectors", ["site corpus · owner facts", "1-node: LanceDB in site dir", "fleet: pgvector, same schema", "the first release: Vultr vector store"], "svc"),
              ("Objects (CAS)", ["releases · evidence · artifacts", "legacy archive (Archive tier)", "1-node: local dir, mirrored", "fleet: Object Storage prefix"], "vultr"),
              ("Site keys", ["a data key per site, wrapped", "personal blobs · exports", "backups · archive", "destroyed = crypto-shredded"], "key"),
              ("Fast state", ["rate limits · wake queues", "presence · cache pub/sub", "1-node: in process", "fleet: Valkey · TTL · no records"], "vultr"),
              ("Site log + outbox", ["hash-chained receipts", "content-free events, written", "in the same transaction", "as the row they describe"], "svc")]
    for i, (t, ls, k) in enumerate(stores):
        s.box(322 + (i % 2) * 238, 112 + (i // 2) * 128, 226, 116, t, ls, kind=k, title_size=13.5)
    s.text(322, 516, "The owner reads all of it. The back office reads by partition.", size=11.5, fill=C["muted"])
    s.text(322, 533, "Visitor memory: public desks only, with a consent receipt.", size=11.5, fill=C["muted"])
    s.box(322, 550, 468, 166, "Data classes", [], kind="actor", title_size=13)
    classes = [("C0 public", "signed releases, published facts"),
               ("C1 content-free", "ids · kinds · durations · counts · money"),
               ("C2 business", "owner facts · drafts · playbooks · receipts"),
               ("C3 personal", "conversations · leads · orders · memory"),
               ("C4 secret", "in key holders only; sign-in secrets sealed")]
    for i, (k, v) in enumerate(classes):
        y = 590 + i * 24
        col = C["coral"] if k.startswith("C4") else (C["vultr"] if k.startswith(("C0", "C1")) else C["indigo"])
        s.text(338, y, k, size=11.5, weight=700, fill=col)
        s.text(458, y, v, size=11.5, fill=C["muted"])

    # The boundary
    s.add(f'<path d="M815,92 V722" stroke="{C["coral"]}" stroke-width="2" stroke-dasharray="7 5" fill="none"/>')
    s.add(f'<text x="809" y="410" font-size="10.5" font-weight="700" fill="{C["coral"]}" letter-spacing="1.2" '
          f'text-anchor="middle" transform="rotate(-90 809 410)">CONTENT-FREE BOUNDARY · ONLY C0 AND C1 CROSS</text>')

    # Column 3 · across sites, content-free
    s.panel(826, 84, 430, 648, "ACROSS SITES · CONTENT-FREE · C0–C1")
    flow = [("Kafka topics", ["ledger · usage · receipt index · edge", "keyed by site_id · typed, versioned", "1-node: an in-process channel"], "vultr"),
            ("agentrix-lake ingester", ["Rust: arrow-rs → Parquet files", "Iceberg commits · erasure deletes"], "svc"),
            ("Lakehouse on Object Storage", ["bronze → silver → gold, per cell", "Iceberg tables over Parquet", "catalog in the cell's Postgres"], "vultr"),
            ("Query engines", ["DataFusion inside services", "DuckDB for analysts and exports"], "svc")]
    ys = [112, 222, 314, 424]
    hs = [96, 78, 96, 78]
    for (t, ls, k), y, h in zip(flow, ys, hs):
        s.box(842, y, 250, h, t, ls, kind=k, title_size=13)
    for y0, y1 in [(208, 220), (300, 312), (410, 422)]:
        s.arrow(f"M967,{y0} V{y1}")
    s.box(1104, 112, 138, 168, "Telemetry", ["OTel collector", "per cell", "metrics, traces,", "logs: content-free"], kind="svc", title_size=13)
    s.box(1104, 292, 138, 130, "Observability", ["Object Storage", "short windows", "SLO burn alerts"], kind="vultr", title_size=13)
    s.arrow("M1173,280 V290")
    s.box(842, 516, 400, 96, "Privacy gate (fleet learning)", ["min cohort · k-anonymity · DP noise", "aggregates only → signed playbook packs"], kind="svc", title_size=13)
    s.box(842, 624, 400, 92, "Global layer (ams-1, standby fra-1)", ["gold aggregates only · fleet registry", "billing rating → zavora-era books"], kind="actor", title_size=13)
    s.arrow("M967,502 V514")

    # Producers write through one bus: every write carries SiteId
    for i in range(6):
        y = 151 + i * 87
        s.add(f'<path d="M270,{y} H296" stroke="{C["indigo"]}" stroke-width="1.6" fill="none"/>')
    s.add(f'<path d="M296,151 V586" stroke="{C["indigo"]}" stroke-width="1.6" fill="none"/>')
    for y in [170, 298, 426]:
        s.arrow(f"M296,{y} H320")
    # outbox -> kafka across the boundary
    s.arrow("M674,484 V500 H830 V160 H840", "outbox · content-free only", 712, 500, color="coral")

    # Row · visualisation and use
    s.panel(24, 752, 1232, 176, "VISUALISATION AND USE")
    uses = [("Owner analytics", ["console: plain words + a number", "typed queries, no free SQL", "own records + own lake slice"], "svc"),
            ("WebAgent documents", ["zavora-xlsx · slide · docx", "printpdf · charts with sources", "saved as signed artifacts"], "svc"),
            ("Owner export", ["one click · open formats", "Parquet · JSONL · CSV", "static site · receipts"], "svc"),
            ("Operator BI", ["operator console: fleet, money", "Grafana for SLOs and burn", "Superset on DuckDB, gold only"], "svc"),
            ("Partner views", ["their sites only, by", "tenant path · content-free", "more only if the owner grants"], "svc")]
    for i, (t, ls, k) in enumerate(uses):
        s.box(40 + i * 242, 790, 230, 122, t, ls, kind=k, title_size=13)
    s.arrow("M397,732 V781", "own records · C0–C3", 470, 744)
    s.add(f'<path d="M48,785 H746" stroke="{C["indigo"]}" stroke-width="1.6" fill="none"/>')
    s.arrow("M1002,732 V781", "gold tables · C0–C1", 1090, 744)
    s.add(f'<path d="M774,785 H1230" stroke="{C["indigo"]}" stroke-width="1.6" fill="none"/>')

    # Lifecycle strip
    s.panel(24, 948, 1232, 126, "LIFECYCLE · REACHES EVERY STORE")
    x = 40
    for n in ["Erasure orchestrator: each store returns a receipt", "Erasures replayed after any restore", "Retention jobs leave receipts"]:
        x += s.chip(x, 980, n) + 10
    x = 40
    for n in ["PITR + nightly per-site export to the copy region", "Residency: home cell; inference location disclosed", "Schema: expand, migrate, contract"]:
        x += s.chip(x, 1012, n) + 10
    s.text(40, 1058, "Single node: the same stores as files under data/sites/<site>/, WAL shipped to Object Storage. "
           "Fleet: Managed PostgreSQL + pgvector, Valkey, Kafka, Object Storage per cell.", size=11.5, fill=C["muted"])
    s.save("data-architecture.svg")


if __name__ == "__main__":
    data_architecture()
    for extra in sys.argv[2:]:
        Path(extra).mkdir(parents=True, exist_ok=True)
        shutil.copy(Path(sys.argv[1]) / "data-architecture.svg", Path(extra) / "data-architecture.svg")
