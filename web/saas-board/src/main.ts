// The treasury board: AG Studio on saas-web's feed, in the Agentrix look, with its AI harness
// swapped for one that streams from an adk-rust agent (harness.ts).

import { AgStudioAiModule, AgStudioLicenseManager, createStudio, createWidgets, type AgStudioApi } from "ag-studio";
import { allSources, loadFeed, RELATIONSHIPS, type Feed } from "./feed";
import { AgentrixHarness, type BoardTools } from "./harness";
import { initialReport, PAGES } from "./report";
import { agentrixTheme } from "./theme";
import { WIDGETS, type BoardContext } from "./widgets";
import "./style.css";

interface BoardConfig {
  /** AG Studio's licence key, when one is set on the node (AG_STUDIO_LICENSE_KEY). */
  licenseKey?: string | null;
  /** The agent door's public host, for links. */
  doorHost?: string;
}

async function config(): Promise<BoardConfig> {
  try {
    const res = await fetch("/board/config.json", { headers: { accept: "application/json" } });
    return res.ok ? ((await res.json()) as BoardConfig) : {};
  } catch {
    return {};
  }
}

/** What the Treasurer may ask the board to do, run against the Studio API. */
function boardTools(api: () => AgStudioApi | undefined): BoardTools {
  return {
    names: ["board.filter", "board.page"],
    async run(name, args) {
      const studio = api();
      if (!studio) throw new Error("The board isn't ready yet.");
      const state = structuredClone(studio.getState()) as any;
      if (name === "board.filter") {
        const who = typeof args.counterparty === "string" ? args.counterparty : null;
        for (const page of state.pages) {
          const rest = (page.filter?.page ?? []).filter((x: any) => x?.field?.id !== "counterparties.name");
          page.filter = {
            ...(page.filter ?? {}),
            page: who ? [...rest, { field: { id: "counterparties.name" }, view: { expanded: true }, model: { operator: "isIn", value: [who] } }] : rest,
          };
        }
        if (who) state.panels = { ...(state.panels ?? {}), filters: { ...(state.panels?.filters ?? {}), collapsed: false } };
        studio.setState(state);
        return who ? `Board filtered to ${who}.` : "Board filter cleared.";
      }
      if (name === "board.page") {
        const page = String(args.page ?? "");
        if (!state.pages.some((p: any) => p.id === page)) throw new Error(`No page called ${page}.`);
        state.selectedPageId = page;
        studio.setState(state);
        return `Opened ${page}.`;
      }
      throw new Error(`The board has no tool called ${name}.`);
    },
  };
}

function header(feed: Feed, cfg: BoardConfig, licensed: boolean) {
  const source = document.querySelector<HTMLElement>("[data-source]");
  if (source) {
    const label: Record<string, string> = { demo: "Demo books", fakebooks: "saas-core FakeBooks, seeded", books: "Zavora ERA" };
    source.textContent = `${label[feed.source] ?? feed.source} · as of ${feed.asOf}`;
    source.dataset.tone = feed.source === "books" ? "ok" : "wait";
  }
  const lic = document.querySelector<HTMLElement>("[data-licence]");
  if (lic && !licensed) {
    lic.hidden = false;
    lic.textContent = "AG Studio trial key not set";
    lic.title = "Set AG_STUDIO_LICENSE_KEY on the node to remove the watermark.";
  }
  const local = location.hostname.endsWith("localhost") || location.hostname === "127.0.0.1";
  const door = cfg.doorHost ?? (local ? `pay.localhost${location.port ? `:${location.port === "18184" ? "18183" : location.port}` : ""}` : "pay.agentrix.website");
  for (const a of document.querySelectorAll<HTMLAnchorElement>('a[href^="https://pay."]')) {
    a.href = `${local ? "http" : "https"}://${door}/`;
  }
  if (location.port === "18184") {
    for (const a of document.querySelectorAll<HTMLAnchorElement>('a[href="/console"], a.brand')) {
      a.href = `http://money.localhost:18183${a.getAttribute("href") ?? "/"}`;
    }
  }
}

const read = (text: string) => ({
  label: (p: any) => ({ text, pill: p.result?.success ? p.result.response : undefined }),
});

/** The rail's words for each step the Treasurer takes. */
const TOOL_DISPLAY: Record<string, unknown> = {
  "books.ageing": read("Read the ageing from Zavora ERA"),
  "books.payables": read("Read the bills due"),
  "books.cash": read("Read cash by rail"),
  "books.mrr": read("Read MRR from PayPal Subscriptions"),
  "settle.debt_plans": read("Read the instalment plans"),
  "meter.agent_spend": read("Read the agents' spend"),
  "mandates.list": read("Read the signed mandates"),
  "audit.verify": read("Checked the signed audit log"),
  "treasury.briefing": read("Read today's position"),
  "board.filter": {
    label: (p: any) => ({
      text: typeof p.args?.counterparty === "string" ? `Filtered the board to ${p.args.counterparty}` : "Cleared the board's filter",
      pill: "board",
    }),
  },
  "board.page": { label: (p: any) => ({ text: `Opened the ${p.args?.page ?? ""} page`, pill: "board" }) },
};

function failed(message: string) {
  const main = document.getElementById("studio");
  if (!main) return;
  const p = document.createElement("p");
  p.className = "board-failed";
  p.textContent = message;
  main.replaceChildren(p);
}

async function start() {
  const [cfg, feed] = await Promise.all([config(), loadFeed()]);
  const licensed = Boolean(cfg.licenseKey);
  if (cfg.licenseKey) AgStudioLicenseManager.setLicenseKey(cfg.licenseKey);
  header(feed, cfg, licensed);

  if (matchMedia("(prefers-color-scheme: dark)").matches) document.body.dataset.agThemeMode = "dark";

  let studio: AgStudioApi | undefined;
  const context: BoardContext = { audit: feed.audit, asOf: feed.asOf, source: feed.source };
  const root = document.getElementById("studio");
  if (!root) throw new Error("No #studio element");

  studio = createStudio(root, {
    // Edit mode: the AI panel (the Treasurer) is an edit-mode panel, and the owner may rearrange
    // the board. The chat sits on the right, open; fields and widget settings on the left.
    mode: "edit",
    panels: { edit: { left: ["data", "edit"], right: ["ai", "filters"] } },
    theme: agentrixTheme,
    context,
    data: { sources: allSources(feed), relationships: RELATIONSHIPS } as any,
    initialState: initialReport(feed),
    widgets: ((defaults: any) =>
      createWidgets({
        additionalTypes: WIDGETS as any,
        menu: [...(defaults?.menu ?? []), { label: "Agentrix", widgetIds: WIDGETS.map((w) => w.id) }],
      } as any)) as any,
    // How the Treasurer's steps read in the chat: what it read from the books, what it did to the board.
    aiToolDisplay: TOOL_DISPLAY,
    ai: () =>
      new AgentrixHarness({
        endpoint: "/board/agent",
        userId: "owner",
        tools: boardTools(() => studio),
      }),
  } as any, { modules: [AgStudioAiModule] });

  // Keep the board current: the books change when agents settle, pay or collect.
  setInterval(async () => {
    try {
      const next = await loadFeed();
      studio?.setProperty("data" as any, { sources: allSources(next), relationships: RELATIONSHIPS } as any);
      studio?.setProperty("context" as any, { audit: next.audit, asOf: next.asOf, source: next.source } as any);
      header(next, cfg, licensed);
    } catch {
      // Keep showing the last good feed.
    }
  }, 30_000);

  (window as any).agentrixBoard = { studio, pages: PAGES };
}

start().catch((e) => failed(`The board couldn't start: ${e instanceof Error ? e.message : String(e)}`));
