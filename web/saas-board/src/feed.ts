// The board's data: saas-web's /feed.json (crates/saas-web/src/feed.rs, version 1), turned into
// AG Studio data sources. The feed keeps every amount in minor units; the board shows major
// units, so each `…Minor` becomes a number of dollars here and nowhere else.

export interface Agent { id: string; name: string; job: string; state: string; doing: string; keyId: string | null }
export interface Company { id: string; name: string; host: string; owner: string; agents: Agent[]; frozen: boolean }
export interface Receivable {
  company: string; invoiceId: string; number: string; customer: string; description: string;
  issued: string; due: string; currency: string; amountMinor: number; openMinor: number;
  daysOverdue: number; bucket: string; status: string; paypalId: string | null;
}
export interface AgeingRow {
  company: string; customer: string; currency: string; currentMinor: number; d130Minor: number;
  d3160Minor: number; d6190Minor: number; over90Minor: number; totalMinor: number; creditLimitMinor: number | null;
}
export interface Payable {
  company: string; billId: string; number: string; vendor: string; description: string; due: string;
  currency: string; amountMinor: number; openMinor: number; daysToDue: number; rail: string; mandate: string; status: string;
}
export interface Instalment { n: number; due: string; amountMinor: number; status: string; paypalId: string | null }
export interface DebtPlan {
  id: string; creditor: string; debtor: string; invoiceNumber: string; currency: string; principalMinor: number;
  paidMinor: number; status: string; agreed: string; mandateIds: string[]; instalments: Instalment[];
}
export interface CashBalance { company: string; rail: string; account: string; currency: string; balanceMinor: number; mode: string }
export interface MrrPoint { month: string; currency: string; mrrMinor: number; newMinor: number; churnMinor: number; customers: number }
export interface AgentSpend { company: string; agent: string; day: string; currency: string; modelMinor: number; paidOutMinor: number; toolCalls: number }
export interface Mandate {
  id: string; company: string; owner: string; agent: string; scope: string; currency: string; perPaymentMinor: number;
  dailyMinor: number; usedTodayMinor: number; payees: string[]; signedWith: string; signed: string; expires: string; status: string;
}
export interface Decision {
  id: string; company: string; agent: string; title: string; body: string; effects: string[]; why: string;
  currency: string; amountMinor: number | null; state: string; fingerprint: string;
}
export interface Settlement {
  at: string; payer: string; payee: string; invoiceNumber: string; currency: string; amountMinor: number;
  paypalId: string; status: string; route: string;
}
export interface Audit { verified: boolean; entries: number; checkpoint: string; checked: string; tool: string }

export interface Feed {
  version: number; asOf: string; source: string; companies: Company[]; receivables: Receivable[]; ageing: AgeingRow[];
  payables: Payable[]; debtPlans: DebtPlan[]; cash: CashBalance[]; mrr: MrrPoint[]; agentSpend: AgentSpend[];
  mandates: Mandate[]; decisions: Decision[]; settlements: Settlement[]; audit: Audit;
}

/** Minor units to a number of major units (two decimals for every currency the feed carries). */
export const major = (minor: number | null | undefined): number | null =>
  minor === null || minor === undefined ? null : Math.round(minor) / 100;

const companyName = (feed: Feed, id: string) => feed.companies.find((c) => c.id === id)?.name ?? id;
const agentName = (feed: Feed, company: string, id: string) =>
  feed.companies.find((c) => c.id === company)?.agents.find((a) => a.id === id)?.name ?? id;

const BUCKET_ORDER: Record<string, number> = { current: 0, "1-30": 1, "31-60": 2, "61-90": 3, "90+": 4, paid: 5 };

/** One plain row per table, keyed by table id. Field names are the ids widgets refer to. */
export function tables(feed: Feed): Record<string, Record<string, unknown>[]> {
  return {
    receivables: feed.receivables.map((r) => ({
      company: companyName(feed, r.company),
      invoice: r.number,
      customer: r.customer,
      description: r.description,
      issued: r.issued,
      due: r.due,
      currency: r.currency,
      amount: major(r.amountMinor),
      open: major(r.openMinor),
      daysOverdue: r.daysOverdue,
      bucket: r.bucket,
      bucketOrder: BUCKET_ORDER[r.bucket] ?? 9,
      pastDue: r.daysOverdue > 0 ? major(r.openMinor) : 0,
      status: r.status.replace("_", " "),
      paypalId: r.paypalId ?? "",
    })),
    ageing: feed.ageing.map((a) => ({
      company: companyName(feed, a.company),
      customer: a.customer,
      current: major(a.currentMinor),
      d1to30: major(a.d130Minor),
      d31to60: major(a.d3160Minor),
      d61to90: major(a.d6190Minor),
      over90: major(a.over90Minor),
      total: major(a.totalMinor),
      creditLimit: major(a.creditLimitMinor),
    })),
    payables: feed.payables.map((p) => ({
      company: companyName(feed, p.company),
      bill: p.number,
      vendor: p.vendor,
      description: p.description,
      due: p.due,
      currency: p.currency,
      amount: major(p.amountMinor),
      open: major(p.openMinor),
      daysToDue: p.daysToDue,
      rail: p.rail.replace("_", " "),
      mandate: p.mandate === "over" ? "needs owner's passkey" : "inside the mandate",
      status: p.status,
    })),
    instalments: feed.debtPlans.flatMap((p) =>
      p.instalments.map((i) => ({
        plan: p.id,
        invoice: p.invoiceNumber,
        creditor: companyName(feed, p.creditor),
        debtor: companyName(feed, p.debtor),
        n: i.n,
        label: `Instalment ${i.n} of ${p.instalments.length}`,
        due: i.due,
        amount: major(i.amountMinor),
        status: i.status,
        paypalId: i.paypalId ?? "",
        principal: major(p.principalMinor),
        agreed: p.agreed,
      })),
    ),
    cash: feed.cash.map((c) => ({
      company: companyName(feed, c.company),
      rail: c.rail,
      account: c.account,
      currency: c.currency,
      balance: major(c.balanceMinor),
      balanceUsd: c.currency === "USD" ? major(c.balanceMinor) : 0,
      mode: c.mode,
    })),
    mrr: feed.mrr.map((m) => ({
      month: `${m.month}-01`,
      mrr: major(m.mrrMinor),
      newMrr: major(m.newMinor),
      churn: major(m.churnMinor),
      customers: m.customers,
      latest: m.month === feed.mrr[feed.mrr.length - 1]?.month ? "yes" : "no",
    })),
    agentSpend: feed.agentSpend.map((s) => ({
      company: companyName(feed, s.company),
      agent: `${companyName(feed, s.company)} · ${agentName(feed, s.company, s.agent)}`,
      day: s.day,
      model: major(s.modelMinor),
      paidOut: major(s.paidOutMinor),
      toolCalls: s.toolCalls,
    })),
    mandates: feed.mandates.map((m) => ({
      id: m.id,
      company: companyName(feed, m.company),
      agent: agentName(feed, m.company, m.agent),
      owner: m.owner,
      scope: m.scope,
      perPayment: major(m.perPaymentMinor),
      daily: major(m.dailyMinor),
      usedToday: major(m.usedTodayMinor),
      signedWith: m.signedWith,
      expires: m.expires,
      status: m.status,
    })),
    settlements: feed.settlements.map((s) => ({
      at: s.at,
      payer: companyName(feed, s.payer),
      payee: companyName(feed, s.payee),
      invoice: s.invoiceNumber,
      amount: major(s.amountMinor),
      paypalId: s.paypalId,
      status: s.status,
      route: s.route,
    })),
  };
}

type Format = "textFormat" | "integerFormat" | "decimalFormat" | "currencyFormat" | "dateFormat";
interface FieldSpec { id: string; name: string; format: Format; description?: string }

const USD = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

const money = (id: string, name: string, description?: string): FieldSpec => ({ id, name, format: "currencyFormat", description });
const decimal = (id: string, name: string, description?: string): FieldSpec => ({ id, name, format: "decimalFormat", description });
const text = (id: string, name: string, description?: string): FieldSpec => ({ id, name, format: "textFormat", description });
const date = (id: string, name: string): FieldSpec => ({ id, name, format: "dateFormat" });
const int = (id: string, name: string): FieldSpec => ({ id, name, format: "integerFormat" });

/** Field names and formats per table: what Studio's field panel and the AI see. */
export const FIELDS: Record<string, { name: string; description: string; fields: FieldSpec[] }> = {
  receivables: {
    name: "Receivables",
    description: "Customer invoices in Zavora ERA. Amounts in US dollars.",
    fields: [
      text("company", "Company"), text("invoice", "Invoice"), text("customer", "Customer"), text("description", "For"),
      date("issued", "Issued"), date("due", "Due"), text("currency", "Currency"), money("amount", "Amount"),
      money("open", "Open", "Still owed"), int("daysOverdue", "Days overdue"), text("bucket", "Age bucket"),
      int("bucketOrder", "Bucket order"), money("pastDue", "Past due"), text("status", "Status"), text("paypalId", "PayPal id"),
    ],
  },
  ageing: {
    name: "Receivables ageing",
    description: "Open receivables by customer and age bucket, as ERA ages them.",
    fields: [
      text("company", "Company"), text("customer", "Customer"), money("current", "Not yet due"), money("d1to30", "1–30 days"),
      money("d31to60", "31–60 days"), money("d61to90", "61–90 days"), money("over90", "Over 90 days"), money("total", "Total open"),
      money("creditLimit", "Credit limit"),
    ],
  },
  payables: {
    name: "Payables",
    description: "Bills to pay through PayPal, inside the owner's mandate unless marked.",
    fields: [
      text("company", "Company"), text("bill", "Bill"), text("vendor", "Pay to"), text("description", "For"), date("due", "Due"),
      text("currency", "Currency"), money("amount", "Amount"), money("open", "To pay"), int("daysToDue", "Days to due"),
      text("rail", "Rail"), text("mandate", "Mandate"), text("status", "Status"),
    ],
  },
  instalments: {
    name: "Debt plan instalments",
    description: "Instalment plans the two companies' agents agreed inside both owners' mandates.",
    fields: [
      text("plan", "Plan"), text("invoice", "Invoice"), text("creditor", "Owed to"), text("debtor", "Owed by"), int("n", "Instalment"),
      text("label", "Label"), date("due", "Due"), money("amount", "Amount"), text("status", "Status"), text("paypalId", "PayPal id"),
      money("principal", "Plan principal"), date("agreed", "Agreed"),
    ],
  },
  cash: {
    name: "Cash by rail",
    description: "Cash per rail and account. USD and KES are not added together.",
    fields: [
      text("company", "Company"), text("rail", "Rail"), text("account", "Account"), text("currency", "Currency"),
      decimal("balance", "Balance", "In the account's own currency"), money("balanceUsd", "Balance (USD accounts)"), text("mode", "Mode"),
    ],
  },
  mrr: {
    name: "MRR",
    description: "Monthly recurring revenue from PayPal Subscriptions, in US dollars.",
    fields: [date("month", "Month"), money("mrr", "MRR"), money("newMrr", "New MRR"), money("churn", "Churned MRR"), int("customers", "Customers"), text("latest", "Latest month")],
  },
  agentSpend: {
    name: "Agent spend",
    description: "What each agent spent on models, and moved out, per day. US dollars.",
    fields: [text("company", "Company"), text("agent", "Agent"), date("day", "Day"), money("model", "Model spend"), money("paidOut", "Paid out"), int("toolCalls", "Tool calls")],
  },
  mandates: {
    name: "Mandates",
    description: "AP2-style intent mandates the owners signed with a passkey, and what is used today.",
    fields: [
      text("id", "Mandate"), text("company", "Company"), text("agent", "Agent"), text("owner", "Signed by"), text("scope", "Scope"),
      money("perPayment", "Per payment"), money("daily", "Per day"), money("usedToday", "Used today"), text("signedWith", "Signed with"),
      date("expires", "Expires"), text("status", "Status"),
    ],
  },
  settlements: {
    name: "Agent-to-agent settlements",
    description: "Invoices settled between the companies' agents; both ledgers post from one PayPal id.",
    fields: [date("at", "Settled"), text("payer", "Payer"), text("payee", "Payee"), text("invoice", "Invoice"), money("amount", "Amount"), text("paypalId", "PayPal id"), text("status", "Status"), text("route", "Route")],
  },
};

/** The feed as AG Studio's `data.sources`. */
export function sources(feed: Feed) {
  const rows = tables(feed);
  return Object.entries(FIELDS).map(([id, t]) => ({
    id,
    name: t.name,
    description: t.description,
    data: rows[id] ?? [],
    fields: t.fields.map((f) => ({
      id: f.id,
      name: f.name,
      format: f.format,
      ...(f.format === "currencyFormat" ? { formatOptions: { format: USD } } : {}),
      ...(f.description ? { description: f.description } : {}),
    })),
  }));
}

/** Reads the feed from saas-web. */
export async function loadFeed(url = "/feed.json"): Promise<Feed> {
  const res = await fetch(url, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`The feed answered ${res.status}`);
  const feed = (await res.json()) as Feed;
  if (feed.version !== 1) throw new Error(`Feed version ${feed.version}; this board reads version 1`);
  return feed;
}

/**
 * Counterparties: one row per customer, vendor or company the books name, so one page filter on
 * `counterparties.name` narrows every table that names it (the Treasurer's `board.filter`).
 */
export function counterparties(feed: Feed): { name: string; kind: string }[] {
  const seen = new Map<string, string>();
  const add = (name: string, kind: string) => {
    if (name && !seen.has(name)) seen.set(name, kind);
  };
  for (const r of feed.receivables) add(r.customer, "customer");
  for (const p of feed.payables) add(p.vendor, "vendor");
  for (const c of feed.companies) add(c.name, "company");
  return [...seen.entries()].map(([name, kind]) => ({ name, kind }));
}

/** How the tables join: each names a counterparty (many rows to one). */
export const RELATIONSHIPS = [
  { id: "receivables-counterparty", source: "receivables.customer", target: "counterparties.name", type: "many-to-one" },
  { id: "ageing-counterparty", source: "ageing.customer", target: "counterparties.name", type: "many-to-one" },
  { id: "payables-counterparty", source: "payables.vendor", target: "counterparties.name", type: "many-to-one" },
  { id: "instalments-counterparty", source: "instalments.debtor", target: "counterparties.name", type: "many-to-one" },
  { id: "settlements-counterparty", source: "settlements.payer", target: "counterparties.name", type: "many-to-one" },
] as const;

/** Every source, with the counterparties dimension. */
export function allSources(feed: Feed) {
  return [
    ...sources(feed),
    {
      id: "counterparties",
      name: "Counterparties",
      description: "Every customer, vendor and company the books name.",
      data: counterparties(feed),
      fields: [
        { id: "name", name: "Counterparty", format: "textFormat" },
        { id: "kind", name: "Kind", format: "textFormat" },
      ],
    },
  ];
}
