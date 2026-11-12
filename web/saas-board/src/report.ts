// The board as it opens: two pages laid out on Studio's 24-column grid. Every widget reads a
// table from feed.ts by `table.field`. People can change all of it in edit mode; the Treasurer
// can filter it from the chat.

import type { Feed } from "./feed";

const f = (id: string, aggregation?: string) => (aggregation ? { id, aggregation } : { id });
const title = (text: string, sub?: string) => ({
  title: { enabled: true, text },
  ...(sub ? { subtitle: { enabled: true, text: sub } } : {}),
});
const at = (xTrack: number, yTrack: number, xSpan: number, ySpan: number) => ({ xTrack, yTrack, xSpan, ySpan });
const only = (field: string, values: string[]) => [{ field: { id: field }, model: { operator: "isIn", value: values } }];

export const PAGES = { treasury: "treasury", agents: "agents" } as const;

/** The report for this feed (the latest MRR month is picked by name). */
export function initialReport(feed: Feed): any {
  const latest = feed.mrr[feed.mrr.length - 1]?.month;
  const plan = feed.debtPlans[0];
  const debtor = plan ? (feed.companies.find((c) => c.id === plan.debtor)?.name ?? plan.debtor) : "";
  return {
    selectedPageId: PAGES.treasury,
    panels: { filters: { collapsed: true }, data: { collapsed: true }, edit: { collapsed: true }, ai: { collapsed: false } },
    pages: [
      {
        id: PAGES.treasury,
        widgets: {
          cash: { type: "value", dataMapping: { value: [f("cash.balanceUsd", "sum")] }, format: title("Cash on PayPal", "USD, both companies") },
          owed: { type: "value", dataMapping: { value: [f("receivables.open", "sum")] }, format: title("Owed to Agentrix", "Open receivables") },
          late: { type: "value", dataMapping: { value: [f("receivables.pastDue", "sum")] }, format: title("Past due", "Agentrix, open and late") },
          mrr: { type: "value", dataMapping: { value: [f("mrr.mrr", "sum")] }, format: title("MRR", latest ? `PayPal Subscriptions, ${latest}` : "PayPal Subscriptions") },
          gauge: { type: "mandateGauge", dataMapping: { used: [f("mandates.usedToday", "sum")], limit: [f("mandates.daily", "sum")] }, format: title("Founder's mandate", "Payables, used today") },
          audit: { type: "auditBadge", dataMapping: {}, format: title("Audit log") },
          ageing: {
            type: "column-chart-stacked",
            dataMapping: {
              categoryKey: [f("ageing.customer")],
              valueKey: [f("ageing.current", "sum"), f("ageing.d1to30", "sum"), f("ageing.d31to60", "sum"), f("ageing.d61to90", "sum"), f("ageing.over90", "sum")],
            },
            format: title("Receivables ageing", "Agentrix, open by customer and days late"),
          },
          plan: {
            type: "debtTimeline",
            dataMapping: { label: [f("instalments.label")], due: [f("instalments.due")], amount: [f("instalments.amount", "sum")], status: [f("instalments.status")] },
            format: title(plan ? `Debt plan · ${plan.invoiceNumber}` : "Debt plan", plan ? `${debtor}, agreed agent to agent` : "No instalment plan"),
          },
          payables: {
            type: "grid",
            dataMapping: { cols: [f("payables.bill"), f("payables.vendor"), f("payables.due"), f("payables.open", "sum"), f("payables.mandate"), f("payables.status")] },
            format: title("Payables due", "PayPal Payouts inside the founder's mandate"),
          },
          mrrTrend: {
            type: "line-chart",
            dataMapping: { categoryKey: [f("mrr.month")], valueKey: [f("mrr.mrr", "sum")] },
            format: title("MRR", feed.mrr.length > 1 ? `Last ${feed.mrr.length} months` : "This month, from the books' recurring invoices"),
          },
          receivables: {
            type: "grid",
            dataMapping: { cols: [f("receivables.invoice"), f("receivables.customer"), f("receivables.due"), f("receivables.daysOverdue", "max"), f("receivables.open", "sum"), f("receivables.status")] },
            format: title("Receivables", "Invoices in Zavora ERA"),
          },
          cashByRail: {
            type: "grid",
            dataMapping: { cols: [f("cash.company"), f("cash.rail"), f("cash.account"), f("cash.currency"), f("cash.balance", "sum"), f("cash.mode")] },
            format: { ...title("Cash by rail", "PayPal and M-Pesa, each in its own currency"), style: { grandTotalRow: { enabled: false } } },
          },
          settlements: {
            type: "grid",
            dataMapping: { cols: [f("settlements.at"), f("settlements.payer"), f("settlements.payee"), f("settlements.invoice"), f("settlements.amount", "sum"), f("settlements.paypalId")] },
            format: title("Settled agent to agent", "Both ledgers post from one PayPal id"),
          },
        },
        widgetLayout: {
          cash: at(0, 0, 6, 6),
          owed: at(6, 0, 6, 6),
          late: at(12, 0, 6, 6),
          mrr: at(18, 0, 6, 6),
          ageing: at(0, 6, 12, 14),
          gauge: at(12, 6, 6, 14),
          audit: at(18, 6, 6, 14),
          plan: at(0, 20, 10, 16),
          payables: at(10, 20, 14, 16),
          receivables: at(0, 36, 14, 14),
          mrrTrend: at(14, 36, 10, 14),
          cashByRail: at(0, 50, 12, 12),
          settlements: at(12, 50, 12, 12),
        },
        filter: {
          widget: {
            mrr: only("mrr.latest", ["yes"]),
            owed: only("receivables.company", ["Agentrix"]),
            late: only("receivables.company", ["Agentrix"]),
            ageing: only("ageing.company", ["Agentrix"]),
            receivables: [...only("receivables.bucket", ["current", "1-30", "31-60", "61-90", "90+"]), ...only("receivables.company", ["Agentrix"])],
            payables: [...only("payables.status", ["open", "scheduled", "held"]), ...only("payables.company", ["Agentrix"])],
            gauge: only("mandates.id", ["mandate-agentrix-payables"]),
          },
        },
      },
      {
        id: PAGES.agents,
        widgets: {
          spend: {
            type: "column-chart-stacked",
            dataMapping: { categoryKey: [f("agentSpend.day")], valueKey: [f("agentSpend.model", "sum")] },
            format: title("Model spend per day", "Every agent of both companies, at cost"),
          },
          byAgent: {
            type: "bar-chart-grouped",
            dataMapping: { categoryKey: [f("agentSpend.agent")], valueKey: [f("agentSpend.model", "sum"), f("agentSpend.toolCalls", "sum")] },
            format: title("By agent", "Last 7 days"),
          },
          mandates: {
            type: "grid",
            dataMapping: { cols: [f("mandates.company"), f("mandates.agent"), f("mandates.owner"), f("mandates.perPayment", "sum"), f("mandates.daily", "sum"), f("mandates.usedToday", "sum"), f("mandates.status"), f("mandates.expires")] },
            format: title("Mandates", "AP2-style intent mandates, signed with a passkey"),
          },
          kileleGauge: { type: "mandateGauge", dataMapping: { used: [f("mandates.usedToday", "sum")], limit: [f("mandates.daily", "sum")] }, format: title("Kilele's mandate", "Payables, used today") },
        },
        widgetLayout: {
          spend: at(0, 0, 14, 14),
          byAgent: at(14, 0, 10, 14),
          mandates: at(0, 14, 18, 10),
          kileleGauge: at(18, 14, 6, 10),
        },
        filter: { widget: { kileleGauge: only("mandates.id", ["mandate-kilele-payables"]) } },
      },
    ],
  };
}
