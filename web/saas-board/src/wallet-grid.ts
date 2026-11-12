// <agentrix-wallet src="…/wallet.json">: the wallet ledger and the three-way match (wallet
// entry · PayPal object · the books' payment and journal) as AG Grid Community tables (MIT), in
// the Agentrix look. The JSON is saas-web's grid view of saas-core's `WalletFeed`.
// Served by saas-web at /console/{site}/assets/wallet-grid.js; its children (a plain table) stay
// until the grid draws, so the page reads without it.

import { matchClass, matchWords } from "./wallet-words";
import { AllCommunityModule, ModuleRegistry, createGrid, themeQuartz, type ColDef, type GridOptions } from "ag-grid-community";

ModuleRegistry.registerModules([AllCommunityModule]);

const theme = themeQuartz.withParams({
  accentColor: "#38356d",
  foregroundColor: "#23223a",
  backgroundColor: "#ffffff",
  borderColor: "#e3e2ec",
  headerBackgroundColor: "#f6f6fa",
  headerTextColor: "#38356d",
  fontFamily: '"DM Sans", "Manrope", system-ui, sans-serif',
  fontSize: 14,
  rowHeight: 40,
  headerHeight: 40,
  wrapperBorderRadius: 12,
});

interface Entry {
  seq: number;
  at: string;
  kind: string;
  agent: string;
  currency: string;
  amountMinor: number;
  paypalId: string;
  paypalStatus: string;
  counterparty: string;
  cause: string;
  authority: string;
  source: string;
}
interface Match {
  seq: number;
  kind: string;
  currency: string;
  amountMinor: number;
  paypalId: string;
  /** What PayPal said when read back; null when PayPal wasn't read back. */
  paypalStatus: string | null;
  /** What PayPal had, signed like the entry; null when not read back. */
  paypalAmountMinor: number | null;
  /** What the books carry (the payment, or the fee journal); null when not posted. */
  booksAmountMinor: number | null;
  payment: string | null;
  journal: string | null;
  feeJournal: string | null;
  /** The books' own numbers (`PAY-2026-0009`, `JE-2026-0029`), when the feed carries them. */
  paymentNumber?: string | null;
  journalNumber?: string | null;
  feeJournalNumber?: string | null;
  state: string;
}

const money = (minor: number | null | undefined, currency = "USD") =>
  minor === null || minor === undefined
    ? "—"
    : new Intl.NumberFormat("en-US", { style: "currency", currency }).format(minor / 100);

/** A books id at a glance: ERA's UUIDs by their first 8 characters (the full id in the tooltip). */
function shortId(v: string | null | undefined): string {
  if (!v) return "—";
  return /^[0-9a-f]{8}-[0-9a-f]{4}-/i.test(v) ? `${v.slice(0, 8)}…` : v;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function grid<T>(host: HTMLElement, rows: T[], columns: ColDef<T>[], height: number) {
  const box = el("div", "ax-grid");
  box.style.height = `${height}px`;
  host.append(box);
  const options: GridOptions<T> = {
    theme,
    rowData: rows,
    columnDefs: columns,
    defaultColDef: { sortable: true, filter: true, resizable: true },
    autoSizeStrategy: { type: "fitGridWidth" },
    animateRows: false,
  };
  createGrid(box, options);
}

class AgentrixWallet extends HTMLElement {
  async connectedCallback() {
    const src = this.getAttribute("src");
    if (!src) return;
    let data: { entries?: Entry[]; matches?: Match[] };
    try {
      // POST, so the grid also reads inside Agentrix OS, where an app's GET returns a page.
      const res = await fetch(src, { method: "POST", headers: { accept: "application/json" } });
      if (!res.ok) return; // keep the server's plain table and its words
      data = await res.json();
    } catch {
      return;
    }
    const entries = data.entries ?? [];
    const matches = data.matches ?? [];
    this.replaceChildren();

    this.append(el("h2", "ax-grid-title", "Wallet ledger"));
    grid<Entry>(
      this,
      entries,
      [
        { field: "seq", headerName: "#", width: 70, maxWidth: 80 },
        { field: "at", headerName: "When" },
        { field: "kind", headerName: "Kind" },
        { field: "agent", headerName: "Agent" },
        {
          field: "amountMinor",
          headerName: "Amount",
          type: "rightAligned",
          valueFormatter: (p) => money(p.value, p.data?.currency),
        },
        { field: "paypalId", headerName: "PayPal id", cellClass: "ax-mono" },
        { field: "paypalStatus", headerName: "PayPal status" },
        { field: "counterparty", headerName: "With" },
        { field: "cause", headerName: "For" },
        { field: "authority", headerName: "On whose authority" },
        { field: "source", headerName: "Recorded from", hide: entries.length > 0 && entries.every((e) => e.source === entries[0].source) },
      ],
      Math.min(80 + entries.length * 40, 440),
    );

    this.append(el("h2", "ax-grid-title", "Three-way match: wallet · PayPal · the books"));
    grid<Match>(
      this,
      matches,
      [
        { field: "seq", headerName: "#", width: 70, maxWidth: 80 },
        { field: "kind", headerName: "Kind" },
        { field: "amountMinor", headerName: "Wallet", type: "rightAligned", valueFormatter: (p) => money(p.value, p.data?.currency) },
        { field: "paypalId", headerName: "PayPal id", cellClass: "ax-mono" },
        { field: "paypalStatus", headerName: "PayPal says", valueFormatter: (p) => p.value ?? "not read back" },
        { field: "paypalAmountMinor", headerName: "PayPal", type: "rightAligned", valueFormatter: (p) => money(p.value, p.data?.currency) },
        { field: "booksAmountMinor", headerName: "Books", type: "rightAligned", valueFormatter: (p) => money(p.value, p.data?.currency) },
        {
          field: "payment",
          headerName: "Books payment",
          cellClass: "ax-mono",
          valueGetter: (p) => p.data?.paymentNumber ?? shortId(p.data?.payment),
          tooltipField: "payment",
        },
        {
          field: "journal",
          headerName: "Journal",
          cellClass: "ax-mono",
          valueGetter: (p) => p.data?.journalNumber ?? shortId(p.data?.journal),
          tooltipField: "journal",
        },
        {
          field: "state",
          headerName: "Match",
          valueGetter: (p) => (p.data ? matchWords(p.data) : ""),
          cellClass: (p) => (p.data ? matchClass(p.data) : ""),
        },
      ],
      Math.min(80 + matches.length * 40, 440),
    );
  }
}

if (!customElements.get("agentrix-wallet")) customElements.define("agentrix-wallet", AgentrixWallet);

const style = document.createElement("style");
style.textContent = `
.ax-grid-title { font-size: 16px; margin: 18px 0 8px; color: var(--head, #1f1d3d); }
.ax-grid { width: 100%; }
.ax-mono { font-family: var(--font-mono, ui-monospace, monospace); font-size: 13px; }
.ax-ok { color: var(--ok, #11734a); font-weight: 600; }
.ax-risk { color: var(--risk, #a3302a); font-weight: 600; }
.ax-wait { color: var(--wait, #8a5300); }
`;
document.head.append(style);
