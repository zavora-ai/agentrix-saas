// The board's own widgets, registered with AG Studio beside its built-in ones:
//
// - Mandate gauge: how much of an owner's daily mandate an agent has used (AP2-style intent
//   mandate, signed with a passkey). Data: a "used" and a "limit" field, summed.
// - Audit verified: whether the signed audit log's checkpoint verifies offline
//   (`adk-rust audit verify`). Data: the feed's audit block, through Studio's `context`.
// - Debt-plan timeline: each instalment of a plan on a line, paid, due or scheduled. Data: the
//   instalments table's label, due date, amount and status fields.
//
// Each reads through Studio's data engine (`widgetApi.getData`), so page filters, cross-filters
// and the Treasurer's `board.filter` apply to them as to every other widget.

import type { AgTypeScriptComponent, AgWidgetParams } from "ag-studio";
import type { Audit } from "./feed";

/** What the board passes as Studio's `context`. */
export interface BoardContext {
  audit: Audit;
  asOf: string;
  source: string;
}

type Params = AgWidgetParams<any, any, BoardContext>;

const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

const SVG = "http://www.w3.org/2000/svg";
function svg(tag: string, attrs: Record<string, string | number>): SVGElement {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** The fields a data-mapping key resolved to (Studio hands hydrated fields to custom widgets). */
function fields(params: Params, key: string): any[] {
  return ((params.dataMapping as Record<string, unknown[] | undefined>)[key] ?? []) as any[];
}

/** A row's value for a field: by its id, or its `table.field` reference. */
function valueOf(row: Record<string, any>, field: any): any {
  if (!row || !field) return undefined;
  for (const k of [field.key, field.id, field.fieldId, field.name]) {
    if (k !== undefined && k in row) return row[k];
  }
  const tail = String(field.id ?? "").split(".").pop();
  if (tail && tail in row) return row[tail];
  return undefined;
}

async function rowsFor(params: Params, keys: string[], queryId: string): Promise<{ rows: Record<string, any>[]; fields: Record<string, any> } | undefined> {
  const picked: Record<string, any> = {};
  for (const k of keys) {
    const f = fields(params, k)[0];
    if (f) picked[k] = f;
  }
  if (Object.keys(picked).length < keys.length) {
    params.widgetApi.setDisplayState("incompleteDataMapping");
    return undefined;
  }
  params.widgetApi.setDisplayState("loading");
  const response: any = await params.widgetApi.getData({ fields: Object.values(picked) } as any, { queryId } as any);
  if (response?.replaced) return undefined;
  const rows: Record<string, any>[] = response?.results?.rows ?? [];
  (globalThis as any).__axLast = { ...((globalThis as any).__axLast ?? {}), [queryId]: { fields: picked, rows } };
  params.widgetApi.setDisplayState(rows.length ? "displayed" : "noData");
  return { rows, fields: picked };
}

/** Mandate gauge: an arc from 0 to the mandate's limit, filled to what is used. */
export class MandateGauge implements AgTypeScriptComponent<Params> {
  private root = el("div", "ax-gauge");

  getGui() {
    return this.root;
  }

  async init(params: Params) {
    await this.refresh(params);
  }

  async refresh(params: Params) {
    const got = await rowsFor(params, ["used", "limit"], "mandate");
    if (!got) return;
    const sum = (k: string) => got.rows.reduce((acc, r) => acc + (Number(valueOf(r, got.fields[k])) || 0), 0);
    const used = sum("used");
    const limit = sum("limit");
    const share = limit > 0 ? Math.min(used / limit, 1) : 0;
    const tone = share >= 1 ? "risk" : share >= 0.75 ? "wait" : "ok";

    const r = 52;
    const len = Math.PI * r;
    const gauge = svg("svg", { viewBox: "0 0 128 76", class: "ax-gauge-arc", role: "img", "aria-label": `${usd.format(used)} of ${usd.format(limit)} used today` });
    gauge.append(
      svg("path", { d: `M 12 66 A ${r} ${r} 0 0 1 116 66`, class: "track", fill: "none", "stroke-width": 12, "stroke-linecap": "round" }),
      svg("path", {
        d: `M 12 66 A ${r} ${r} 0 0 1 116 66`,
        class: `fill ${tone}`,
        fill: "none",
        "stroke-width": 12,
        "stroke-linecap": "round",
        "stroke-dasharray": `${(len * share).toFixed(1)} ${len.toFixed(1)}`,
      }),
    );
    const figure = el("div", "ax-gauge-figure");
    figure.append(el("b", "", usd.format(used)), el("span", "", ` of ${usd.format(limit)}`));
    const caption = el("p", "ax-caption", share >= 1 ? "Used up: more waits for the owner's passkey." : `${Math.round(share * 100)}% used today. Over it, the owner's passkey.`);
    this.root.replaceChildren(gauge, figure, caption);
  }

  destroy() {
    this.root.replaceChildren();
  }
}

/** Audit verified: the signed audit log's last offline check. */
export class AuditBadge implements AgTypeScriptComponent<Params> {
  private root = el("div", "ax-audit");

  getGui() {
    return this.root;
  }

  init(params: Params) {
    this.refresh(params);
  }

  refresh(params: Params) {
    params.widgetApi.setDisplayState("displayed");
    const a = params.context?.audit;
    if (!a) {
      this.root.replaceChildren(el("p", "ax-caption", "No audit check yet."));
      return;
    }
    const badge = el("div", `ax-seal ${a.verified ? "ok" : "risk"}`);
    const icon = svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" });
    icon.append(
      svg("path", { d: "M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5l-8-3z", fill: "currentColor", opacity: 0.16 }),
      svg("path", { d: a.verified ? "m8 12 3 3 5-6" : "M9 9l6 6M15 9l-6 6", fill: "none", stroke: "currentColor", "stroke-width": 2.2, "stroke-linecap": "round", "stroke-linejoin": "round" }),
    );
    badge.append(icon, el("b", "", a.verified ? "Audit verified" : "Audit failed"));
    const facts = el("dl", "ax-facts");
    for (const [k, v] of [
      ["Entries", a.entries.toLocaleString("en-US")],
      ["Checkpoint", `${a.checkpoint.slice(0, 19)}…`],
      ["Checked", a.checked],
    ]) {
      facts.append(el("dt", "", k), el("dd", "", v));
    }
    const how = el("p", "ax-caption");
    how.append("Offline, with ", el("code", "", a.tool), ". Flip one byte and it fails.");
    this.root.replaceChildren(badge, facts, how);
  }

  destroy() {
    this.root.replaceChildren();
  }
}

/** Debt-plan timeline: a plan's instalments on one line. */
export class DebtTimeline implements AgTypeScriptComponent<Params> {
  private root = el("div", "ax-timeline");

  getGui() {
    return this.root;
  }

  async init(params: Params) {
    await this.refresh(params);
  }

  async refresh(params: Params) {
    const got = await rowsFor(params, ["label", "due", "amount", "status"], "instalments");
    if (!got) return;
    const rows = got.rows
      .map((r) => ({
        label: String(valueOf(r, got.fields.label) ?? ""),
        due: valueOf(r, got.fields.due),
        amount: Number(valueOf(r, got.fields.amount)) || 0,
        status: String(valueOf(r, got.fields.status) ?? ""),
      }))
      .sort((a, b) => String(dateText(a.due)).localeCompare(String(dateText(b.due))));
    const total = rows.reduce((s, r) => s + r.amount, 0);
    const paid = rows.filter((r) => r.status === "paid").reduce((s, r) => s + r.amount, 0);

    const head = el("div", "ax-tl-head");
    head.append(el("b", "", `${usd.format(paid)} of ${usd.format(total)} paid`), el("span", "", `${rows.filter((r) => r.status === "paid").length} of ${rows.length} instalments`));
    const bar = el("div", "ax-tl-bar");
    const fill = el("span", "");
    fill.style.width = `${total > 0 ? (paid / total) * 100 : 0}%`;
    bar.append(fill);

    const list = el("ol", "ax-tl");
    for (const r of rows) {
      const li = el("li", `st-${r.status}`);
      const pin = el("span", "pin");
      pin.setAttribute("aria-hidden", "true");
      const body = el("div", "");
      body.append(el("b", "", r.label), el("span", "amt", usd.format(r.amount)));
      li.append(el("time", "", dateText(r.due)), pin, body, el("span", `chip ${r.status}`, r.status));
      list.append(li);
    }
    const caption = el("p", "ax-caption", "Agreed agent to agent over A2A, inside both owners' mandates. Each instalment is collected on its due date; the kernel wakes on the date.");
    this.root.replaceChildren(head, bar, list, caption);
  }

  destroy() {
    this.root.replaceChildren();
  }
}

/** A calendar date as the books wrote it. Studio hands dates back as instants at local midnight. */
function dateText(v: unknown): string {
  const d = v instanceof Date ? v : typeof v === "number" || (typeof v === "string" && v.includes("T")) ? new Date(v as string | number) : undefined;
  if (d && !Number.isNaN(d.getTime())) {
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  }
  return String(v ?? "").slice(0, 10);
}

const icon = (path: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${path}</svg>`;

/** The three definitions, for `createWidgets({ additionalTypes })`. */
export const WIDGETS = [
  {
    id: "mandateGauge",
    label: "Mandate gauge",
    icon: icon('<path d="M4 18a8 8 0 1 1 16 0"/><path d="m12 18 4-6"/>'),
    dataMapping: {
      used: { type: "field", supportedRoles: ["numeric"], requires: { cardinality: "one" }, required: true },
      limit: { type: "field", supportedRoles: ["numeric"], requires: { cardinality: "one" }, required: true },
    },
    form: (params: any) => params.createDefaults({ dataMappingItems: [{ key: "used", label: "Used" }, { key: "limit", label: "Limit" }] }),
    comp: MandateGauge,
    defaultSize: { width: 320, height: 240 },
    minSize: { width: 200, height: 160 },
    ai: { description: "How much of an owner's signed mandate an agent has used today: a gauge from 0 to the limit." },
  },
  {
    id: "auditBadge",
    label: "Audit verified",
    icon: icon('<path d="M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5l-8-3z"/><path d="m9 12 2 2 4-4"/>'),
    dataMapping: {},
    form: (params: any) => params.createDefaults({ dataMappingItems: [] }),
    comp: AuditBadge,
    defaultSize: { width: 320, height: 240 },
    minSize: { width: 200, height: 140 },
    ai: { description: "Whether the signed audit log verifies offline, with its entry count and checkpoint." },
  },
  {
    id: "debtTimeline",
    label: "Debt-plan timeline",
    icon: icon('<path d="M3 12h18"/><circle cx="6" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="18" cy="12" r="2"/>'),
    dataMapping: {
      label: { type: "field", supportedRoles: ["category"], requires: { cardinality: "many" }, required: true, intent: "category" },
      due: { type: "field", supportedRoles: ["temporal", "category"], requires: { cardinality: "many" }, required: true, intent: "category" },
      amount: { type: "field", supportedRoles: ["numeric"], requires: { cardinality: "one", per: "dataMapping.label" }, required: true, intent: "value" },
      status: { type: "field", supportedRoles: ["category"], requires: { cardinality: "many" }, required: true, intent: "category" },
    },
    form: (params: any) =>
      params.createDefaults({
        dataMappingItems: [
          { key: "label", label: "Instalment" },
          { key: "due", label: "Due" },
          { key: "amount", label: "Amount" },
          { key: "status", label: "Status" },
        ],
      }),
    comp: DebtTimeline,
    defaultSize: { width: 480, height: 320 },
    minSize: { width: 280, height: 200 },
    ai: { description: "An instalment plan's instalments on a timeline: paid, due or scheduled, with amounts." },
  },
];
