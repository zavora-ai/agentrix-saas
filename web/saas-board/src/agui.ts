// AG-UI over SSE, as adk-server streams it (`/api/run_sse` with uiProtocol "ag_ui" and
// uiTransport "protocol_native"), folded into AG Studio's chat-session snapshot. Pure functions:
// the harness owns the network and the listeners, this file owns the meaning of each event.

import type { AgAiChatMessage, AgAiChatMessagePart, AgAiEvent, AgAiRunStatus, AgAiToolCallView } from "ag-studio";

/** Reads `data:` payloads from an SSE body, one parsed JSON value per event. */
export async function* readSse(body: ReadableStream<Uint8Array>, signal?: AbortSignal): AsyncGenerator<unknown> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      if (signal?.aborted) return;
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let cut: number;
      while ((cut = buffer.search(/\r?\n\r?\n/)) >= 0) {
        const block = buffer.slice(0, cut);
        buffer = buffer.slice(cut).replace(/^\r?\n\r?\n/, "");
        const data = block
          .split(/\r?\n/)
          .filter((l) => l.startsWith("data:"))
          .map((l) => l.slice(5).replace(/^ /, ""))
          .join("\n");
        if (!data) continue;
        try {
          yield JSON.parse(data);
        } catch {
          // A keep-alive or a frame that isn't JSON: nothing to show.
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}

/** One conversation's state: what AG Studio's chat session exposes. */
export interface Transcript {
  messages: AgAiChatMessage[];
  status: AgAiRunStatus;
  sharedState?: unknown;
  /** Tool calls the browser runs itself (the agent asked the board to do something). */
  clientCalls: AgAiToolCallView[];
}

export const emptyTranscript = (): Transcript => ({ messages: [], status: "idle", clientCalls: [] });

interface RunCursor {
  /** This run's messages are created at or after this time. */
  at: number;
  /** Raw argument text per tool call, as it streams. */
  args: Map<string, string>;
  /** The transcript message each AG-UI text or reasoning stream writes into. */
  streams: Map<string, string>;
}

/**
 * Folds AG-UI events into a transcript, one run at a time, in the shape AG Studio's chat panel
 * draws: every tool call is a message of its own (consecutive ones form one rail of steps), and
 * every text or reasoning stream is an assistant message of its own.
 */
export class Folder {
  private cursor: RunCursor | undefined;
  private seq = 0;

  constructor(private readonly isClientTool: (name: string) => boolean = () => false) {}

  /** The reader's message; the run that answers it adds its own messages as they stream. */
  start(t: Transcript, userText: string, now = Date.now()): Transcript {
    this.cursor = { at: now, args: new Map(), streams: new Map() };
    return {
      ...t,
      status: "running",
      clientCalls: [],
      messages: [...t.messages, { id: this.id("u"), role: "user", parts: [{ type: "text", text: userText }], createdAt: now }],
    };
  }

  private id(prefix: string): string {
    this.seq += 1;
    return `${prefix}-${Date.now().toString(36)}-${this.seq}`;
  }

  private append(t: Transcript, part: AgAiChatMessagePart): { t: Transcript; id: string } {
    const id = this.id("a");
    return { id, t: { ...t, messages: [...t.messages, { id, role: "assistant", parts: [part], createdAt: Date.now() }] } };
  }

  private text(t: Transcript, kind: "text" | "reasoning", key: string, delta: string): Transcript {
    const cursor = this.cursor;
    if (!cursor || !delta) return t;
    const slot = `${kind}:${key}`;
    const target = cursor.streams.get(slot);
    if (!target) {
      const made = this.append(t, { type: kind, text: delta });
      cursor.streams.set(slot, made.id);
      return made.t;
    }
    return {
      ...t,
      messages: t.messages.map((m) =>
        m.id === target
          ? { ...m, parts: m.parts.map((p) => (p.type === kind ? { type: kind, text: p.text + delta } : p)) }
          : m,
      ),
    };
  }

  private tool(t: Transcript, id: string, f: (call: AgAiToolCallView) => AgAiToolCallView): Transcript {
    return {
      ...t,
      messages: t.messages.map((m) =>
        m.parts.some((p) => p.type === "tool_call" && p.toolCall.toolCallId === id)
          ? {
              ...m,
              parts: m.parts.map((p) => (p.type === "tool_call" && p.toolCall.toolCallId === id ? { type: "tool_call", toolCall: f(p.toolCall) } : p)),
            }
          : m,
      ),
    };
  }

  /** Applies one AG-UI event. Unknown events change nothing. */
  apply(t: Transcript, raw: unknown): Transcript {
    const e = raw as AgAiEvent & Record<string, unknown>;
    if (!e || typeof e !== "object" || typeof e.type !== "string") return t;
    switch (e.type) {
      case "RUN_STARTED":
        return { ...t, status: "running" };
      case "RUN_FINISHED":
        return this.finish(t, "idle");
      case "RUN_ERROR":
        return this.finish(this.append(t, { type: "error", message: e.message || "The agent stopped." }).t, "error");
      case "TEXT_MESSAGE_START":
        return t;
      case "TEXT_MESSAGE_CONTENT":
      case "TEXT_MESSAGE_CHUNK":
        return this.text(t, "text", e.messageId ?? "_", e.delta ?? "");
      case "TEXT_MESSAGE_END":
        return t;
      case "REASONING_MESSAGE_CONTENT":
      case "REASONING_MESSAGE_CHUNK":
        return this.text(t, "reasoning", e.messageId ?? "_", e.delta ?? "");
      case "TOOL_CALL_START":
      case "TOOL_CALL_CHUNK": {
        const id = e.toolCallId;
        if (!id) return t;
        const known = this.cursor?.args.has(id);
        if (!known) {
          this.cursor?.args.set(id, "");
          const call: AgAiToolCallView = { toolCallId: id, name: e.toolCallName ?? "tool", status: "streaming", args: {} };
          t = this.append(t, { type: "tool_call", toolCall: call }).t;
        }
        return e.type === "TOOL_CALL_CHUNK" && e.delta ? this.args(t, id, e.delta) : t;
      }
      case "TOOL_CALL_ARGS":
        return this.args(t, e.toolCallId, e.delta);
      case "TOOL_CALL_END": {
        const id = e.toolCallId;
        let name = "";
        t = this.tool(t, id, (c) => {
          name = c.name;
          return { ...c, status: this.isClientTool(c.name) ? "executing" : "ready" };
        });
        if (this.isClientTool(name)) {
          const call = this.find(t, id);
          if (call) t = { ...t, clientCalls: [...t.clientCalls, call] };
        }
        return t;
      }
      case "TOOL_CALL_RESULT": {
        let result: unknown = e.content;
        try {
          result = JSON.parse(e.content);
        } catch {
          // Plain text results stay text.
        }
        return this.tool(t, e.toolCallId, (c) => ({ ...c, status: "complete", result: { success: true, response: summarise(result), data: result } }));
      }
      case "STATE_SNAPSHOT":
        return { ...t, sharedState: e.snapshot };
      case "STATE_DELTA":
        return { ...t, sharedState: patch(t.sharedState, e.delta) };
      default:
        return t;
    }
  }

  /** Records the outcome of a tool the browser ran. */
  settle(t: Transcript, id: string, ok: boolean, response: string, data?: unknown): Transcript {
    return this.tool(t, id, (c) => ({
      ...c,
      status: ok ? "complete" : "error",
      result: ok ? { success: true, response, data } : { success: false, issues: [{ message: response }] },
    }));
  }

  private args(t: Transcript, id: string, delta: string): Transcript {
    const cursor = this.cursor;
    if (!cursor) return t;
    const text = (cursor.args.get(id) ?? "") + delta;
    cursor.args.set(id, text);
    const parsed = parsePartial(text);
    return parsed ? this.tool(t, id, (c) => ({ ...c, args: parsed })) : t;
  }

  private find(t: Transcript, id: string): AgAiToolCallView | undefined {
    for (const m of t.messages) {
      for (const p of m.parts) if (p.type === "tool_call" && p.toolCall.toolCallId === id) return p.toolCall;
    }
    return undefined;
  }

  /** Ends the run: calls still open are cancelled, unless the browser is running them. */
  finish(t: Transcript, status: AgAiRunStatus): Transcript {
    const since = this.cursor?.at ?? 0;
    const messages = t.messages.map((m) =>
      m.createdAt >= since && m.parts.some((p) => p.type === "tool_call" && ["streaming", "ready"].includes(p.toolCall.status))
        ? {
            ...m,
            parts: m.parts.map((p) =>
              p.type === "tool_call" && ["streaming", "ready"].includes(p.toolCall.status)
                ? { type: "tool_call" as const, toolCall: { ...p.toolCall, status: p.toolCall.result ? ("complete" as const) : ("cancelled" as const) } }
                : p,
            ),
          }
        : m,
    );
    return { ...t, messages, status };
  }
}

/** A short line for a tool's result, for the collapsed row. */
function summarise(result: unknown): string {
  if (result && typeof result === "object") {
    for (const [k, v] of Object.entries(result as Record<string, unknown>)) {
      if (Array.isArray(v)) return `${v.length} ${k}`;
    }
    return "Read from the books";
  }
  return String(result ?? "");
}

/** Parses JSON that may still be streaming: complete JSON, or the longest prefix closed off. */
export function parsePartial(text: string): Record<string, unknown> | undefined {
  try {
    const v = JSON.parse(text);
    return v && typeof v === "object" ? (v as Record<string, unknown>) : undefined;
  } catch {
    // Close open strings, arrays and objects and try again.
    const stack: string[] = [];
    let inString = false;
    let escaped = false;
    for (const ch of text) {
      if (inString) {
        if (escaped) escaped = false;
        else if (ch === "\\") escaped = true;
        else if (ch === '"') inString = false;
        continue;
      }
      if (ch === '"') inString = true;
      else if (ch === "{") stack.push("}");
      else if (ch === "[") stack.push("]");
      else if (ch === "}" || ch === "]") stack.pop();
    }
    const closed = text.replace(/[,:]\s*$/, "") + (inString ? '"' : "") + stack.reverse().join("");
    try {
      const v = JSON.parse(closed);
      return v && typeof v === "object" ? (v as Record<string, unknown>) : undefined;
    } catch {
      return undefined;
    }
  }
}

/** The JSON Patch operations AG-UI's STATE_DELTA uses (add, replace, remove) on plain JSON. */
export function patch(doc: unknown, ops: { op: string; path: string; value?: unknown }[]): unknown {
  let root = structuredClone(doc ?? {}) as Record<string, unknown>;
  for (const op of ops ?? []) {
    const keys = op.path.split("/").slice(1).map((k) => k.replace(/~1/g, "/").replace(/~0/g, "~"));
    if (keys.length === 0) {
      if (op.op !== "remove") root = op.value as Record<string, unknown>;
      continue;
    }
    let node = root as Record<string, unknown> | unknown[];
    for (const k of keys.slice(0, -1)) {
      const next = (node as Record<string, unknown>)[k];
      if (next === undefined || next === null || typeof next !== "object") (node as Record<string, unknown>)[k] = {};
      node = (node as Record<string, unknown>)[k] as Record<string, unknown>;
    }
    const last = keys[keys.length - 1];
    if (Array.isArray(node)) {
      const i = last === "-" ? node.length : Number(last);
      if (op.op === "remove") node.splice(i, 1);
      else if (op.op === "add") node.splice(i, 0, op.value);
      else node[i] = op.value;
    } else if (op.op === "remove") delete node[last];
    else node[last] = op.value;
  }
  return root;
}
