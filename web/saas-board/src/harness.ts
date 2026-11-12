// The swapped-in harness: AG Studio's chat panel, driven by an adk-rust agent instead of
// Studio's own loop.
//
// `AgentrixHarness` implements `AgAiHarness` over adk-server's REST API (served by saas-web at
// /board/agent): each thread is an adk session, each message one `POST /api/run_sse` that
// streams AG-UI events back. The Treasurer reads the same books the board draws and shows each
// read as a tool call. When it asks the board to do something (`board.filter`, `board.page`),
// this harness runs that call in the browser against the Studio API and reports the outcome in
// the transcript: the agent drives the dashboard.
//
// Threads and transcripts are kept in localStorage, so a reload keeps the conversation.

import type {
  AgAiAgentDescriptor,
  AgAiChatArtifact,
  AgAiChatMessage,
  AgAiChatSession,
  AgAiChatSessionEventListener,
  AgAiHarness,
  AgAiHarnessEventListener,
  AgAiNewThreadOptions,
  AgAiPromptStarter,
  AgAiRunStatus,
  AgAiThreadSummary,
  AgAiToolCallView,
} from "ag-studio";
import { Folder, emptyTranscript, readSse, type Transcript } from "./agui";

/** The adk-server app the Treasurer is served as. */
export const APP = "treasurer";

export const TREASURER: AgAiAgentDescriptor = {
  id: APP,
  name: "Treasurer",
  description: "Agentrix's treasury desk, an adk-rust agent: receivables, payables, debt plans, cash, MRR, agent spend, mandates and the audit log.",
  available: true,
  userFacing: true,
};

export const PROMPT_STARTERS: AgAiPromptStarter[] = [
  { label: "Today's position", prompt: "Give me today's treasury position." },
  { label: "What's overdue?", prompt: "Which invoices are overdue, and by how long?" },
  { label: "Kilele's instalment plan", prompt: "How is Kilele Bakery's instalment plan going?" },
  { label: "Bills due this week", prompt: "What bills are due this week, and which need my OK?" },
  { label: "Show only Kilele", prompt: "Show only Kilele Bakery on the board." },
  { label: "Does the audit log verify?", prompt: "Does the audit log verify?" },
];

/** What the board can do when the agent asks (client-side tools). */
export interface BoardTools {
  /** Tool names the browser runs itself. */
  names: string[];
  /** Runs one; resolves with the words for the transcript. */
  run(name: string, args: Record<string, unknown>): Promise<string>;
}

export interface HarnessOptions {
  /** Where adk-server is mounted: `/board/agent`. */
  endpoint: string;
  /** The adk user id threads belong to. */
  userId: string;
  /** Client-side tools. */
  tools?: BoardTools;
  /** Where threads are kept (localStorage by default; tests pass a Map). */
  store?: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  /** fetch, for tests. */
  fetch?: typeof fetch;
}

interface Saved {
  threads: AgAiThreadSummary[];
  transcripts: Record<string, AgAiChatMessage[]>;
}

const STORE_KEY = "agentrix.board.threads.v2";

function memoryStore(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
  const m = new Map<string, string>();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k) };
}

function defaultStore() {
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.getItem(STORE_KEY);
      return localStorage;
    }
  } catch {
    // Storage blocked: keep threads for this page only.
  }
  return memoryStore();
}

export class AgentrixHarness implements AgAiHarness {
  readonly agents: ReadonlyArray<AgAiAgentDescriptor> = [TREASURER];
  readonly promptStarters = PROMPT_STARTERS;
  threads: ReadonlyArray<AgAiThreadSummary> = [];

  private readonly listeners = new Set<AgAiHarnessEventListener>();
  private readonly sessions = new Map<string, TreasurerSession>();
  private readonly summaries = new Map<string, AgAiThreadSummary>();
  private readonly transcripts: Record<string, AgAiChatMessage[]> = {};
  private readonly store: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  readonly fetch: typeof fetch;

  constructor(readonly opts: HarnessOptions) {
    this.store = opts.store ?? defaultStore();
    this.fetch = opts.fetch ?? ((...a) => fetch(...a));
    try {
      const saved = JSON.parse(this.store.getItem(STORE_KEY) ?? "null") as Saved | null;
      for (const t of saved?.threads ?? []) this.summaries.set(t.threadId, t);
      Object.assign(this.transcripts, saved?.transcripts ?? {});
    } catch {
      // A damaged save starts fresh.
    }
    this.refresh(false);
  }

  addEventListener(_type: "changed", listener: AgAiHarnessEventListener): void {
    this.listeners.add(listener);
  }

  removeEventListener(_type: "changed", listener: AgAiHarnessEventListener): void {
    this.listeners.delete(listener);
  }

  getAgent(agentId: string) {
    return this.agents.find((a) => a.id === agentId);
  }

  getThread(threadId: string) {
    return this.summaries.get(threadId);
  }

  getSession(threadId: string): AgAiChatSession | undefined {
    return this.sessions.get(threadId);
  }

  async openThread(threadId: string): Promise<AgAiChatSession> {
    const open = this.sessions.get(threadId);
    if (open) return open;
    const summary = this.summaries.get(threadId);
    if (!summary) throw new Error(`No conversation ${threadId}`);
    const session = new TreasurerSession(this, summary, this.transcripts[threadId] ?? []);
    this.sessions.set(threadId, session);
    return session;
  }

  async createThread(opts: AgAiNewThreadOptions): Promise<AgAiChatSession> {
    if (!this.getAgent(opts.agentId)) throw new Error(`No agent called ${opts.agentId}`);
    const threadId = `board-${crypto.randomUUID()}`;
    const summary: AgAiThreadSummary = { threadId, agentId: opts.agentId, title: opts.title, updatedAt: Date.now() };
    this.summaries.set(threadId, summary);
    this.refresh();
    return this.openThread(threadId);
  }

  async deleteThread(threadId: string): Promise<void> {
    this.sessions.get(threadId)?.close();
    this.sessions.delete(threadId);
    if (!this.summaries.delete(threadId)) return;
    delete this.transcripts[threadId];
    this.refresh();
    const path = `${this.opts.endpoint}/api/apps/${APP}/users/${encodeURIComponent(this.opts.userId)}/sessions/${encodeURIComponent(threadId)}`;
    await this.fetch(path, { method: "DELETE" }).catch(() => undefined);
  }

  dispose(): void {
    for (const s of this.sessions.values()) s.close();
    this.sessions.clear();
    this.listeners.clear();
  }

  /** A session's transcript or title changed: save, re-sort, tell the panel. */
  touched(threadId: string, messages: AgAiChatMessage[], title?: string): void {
    const s = this.summaries.get(threadId);
    if (!s) return;
    this.transcripts[threadId] = messages;
    this.summaries.set(threadId, { ...s, title: s.title ?? title, updatedAt: Date.now() });
    this.refresh();
  }

  private refresh(notify = true): void {
    this.threads = [...this.summaries.values()].filter((t) => !t.parentThreadId).sort((a, b) => b.updatedAt - a.updatedAt);
    try {
      const saved: Saved = { threads: [...this.summaries.values()], transcripts: this.transcripts };
      this.store.setItem(STORE_KEY, JSON.stringify(saved));
    } catch {
      // Full or blocked storage: the conversation still works.
    }
    if (notify) for (const l of [...this.listeners]) l({ type: "changed" });
  }
}

/** One conversation with the Treasurer. */
class TreasurerSession implements AgAiChatSession {
  readonly threadId: string;
  readonly artifacts: ReadonlyArray<AgAiChatArtifact> = [];
  private t: Transcript;
  private readonly folder: Folder;
  private readonly listeners = new Set<AgAiChatSessionEventListener>();
  private abort: AbortController | undefined;
  private sessionReady = false;

  constructor(
    private readonly harness: AgentrixHarness,
    summary: AgAiThreadSummary,
    saved: AgAiChatMessage[],
  ) {
    this.threadId = summary.threadId;
    this.t = { ...emptyTranscript(), messages: saved };
    const names = new Set(harness.opts.tools?.names ?? []);
    this.folder = new Folder((name) => names.has(name));
  }

  get messages(): ReadonlyArray<AgAiChatMessage> {
    return this.t.messages;
  }

  get status(): AgAiRunStatus {
    return this.t.status;
  }

  get sharedState(): unknown {
    return this.t.sharedState;
  }

  addEventListener(_type: "changed", listener: AgAiChatSessionEventListener): void {
    this.listeners.add(listener);
  }

  removeEventListener(_type: "changed", listener: AgAiChatSessionEventListener): void {
    this.listeners.delete(listener);
  }

  sendMessage(text: string): void {
    const words = text.trim();
    if (!words || this.t.status === "running") return;
    this.set(this.folder.start(this.t, words), words.slice(0, 60));
    void this.run(words);
  }

  cancel(): void {
    this.abort?.abort();
    this.abort = undefined;
    if (this.t.status === "running") this.set(this.folder.finish(this.t, "idle"));
  }

  close(): void {
    this.abort?.abort();
    this.abort = undefined;
    this.listeners.clear();
  }

  private set(t: Transcript, title?: string): void {
    this.t = t;
    this.harness.touched(this.threadId, t.messages, title);
    for (const l of [...this.listeners]) l({ type: "changed" });
  }

  private sessionPath(): string {
    const { endpoint, userId } = this.harness.opts;
    return `${endpoint}/api/apps/${APP}/users/${encodeURIComponent(userId)}/sessions/${encodeURIComponent(this.threadId)}`;
  }

  /** adk-server keeps sessions in memory: make sure ours exists (after a reload or a restart). */
  private async ensureSession(signal: AbortSignal): Promise<void> {
    if (this.sessionReady) return;
    const path = this.sessionPath();
    const got = await this.harness.fetch(path, { signal, headers: { accept: "application/json" } });
    if (!got.ok) {
      const made = await this.harness.fetch(path, { method: "POST", signal, headers: { "content-type": "application/json" }, body: "{}" });
      if (!made.ok) throw new Error(`The Treasurer couldn't open the conversation (${made.status}).`);
    }
    this.sessionReady = true;
  }

  private async run(text: string): Promise<void> {
    const abort = new AbortController();
    this.abort = abort;
    const { endpoint, userId } = this.harness.opts;
    try {
      await this.ensureSession(abort.signal);
      const res = await this.harness.fetch(`${endpoint}/api/run_sse`, {
        method: "POST",
        signal: abort.signal,
        headers: { "content-type": "application/json", accept: "text/event-stream" },
        body: JSON.stringify({
          appName: APP,
          userId,
          sessionId: this.threadId,
          newMessage: { role: "user", parts: [{ text }] },
          streaming: true,
          uiProtocol: "ag_ui",
          uiTransport: "protocol_native",
        }),
      });
      if (!res.ok || !res.body) throw new Error(`The Treasurer answered ${res.status}.`);
      for await (const event of readSse(res.body, abort.signal)) {
        this.set(this.folder.apply(this.t, event));
      }
      if (this.t.status === "running") this.set(this.folder.finish(this.t, "idle"));
      await this.runClientCalls(this.t.clientCalls);
    } catch (e) {
      if (abort.signal.aborted) return;
      const message = e instanceof Error ? e.message : "The Treasurer stopped.";
      this.set(this.folder.apply(this.t, { type: "RUN_ERROR", message }));
    } finally {
      if (this.abort === abort) this.abort = undefined;
    }
  }

  /** Runs the calls the agent asked the board to make, in order. */
  private async runClientCalls(calls: AgAiToolCallView[]): Promise<void> {
    const tools = this.harness.opts.tools;
    if (!tools) return;
    for (const call of calls) {
      try {
        const words = await tools.run(call.name, call.args as Record<string, unknown>);
        this.set(this.folder.settle(this.t, call.toolCallId, true, words, call.args));
      } catch (e) {
        this.set(this.folder.settle(this.t, call.toolCallId, false, e instanceof Error ? e.message : "The board couldn't do that."));
      }
    }
  }
}
