import { describe, expect, it } from "vitest";
import { Folder, emptyTranscript, parsePartial, patch, readSse } from "./agui";

// The events adk-server streamed for "Show only Kilele Bakery on the board." (trimmed ids).
const RUN = [
  { type: "RUN_STARTED", threadId: "t", runId: "r" },
  { type: "TOOL_CALL_START", toolCallId: "c1", toolCallName: "settle.debt_plans" },
  { type: "TOOL_CALL_ARGS", toolCallId: "c1", delta: '{"asOf":"2026-' },
  { type: "TOOL_CALL_ARGS", toolCallId: "c1", delta: '11-10"}' },
  { type: "TOOL_CALL_END", toolCallId: "c1" },
  { type: "TOOL_CALL_RESULT", toolCallId: "c1", messageId: "m", content: '{"plans":[{"plan":"plan-inv-1042"}]}' },
  { type: "TOOL_CALL_START", toolCallId: "c2", toolCallName: "board.filter" },
  { type: "TOOL_CALL_ARGS", toolCallId: "c2", delta: '{"counterparty":"Kilele Bakery"}' },
  { type: "TOOL_CALL_END", toolCallId: "c2" },
  { type: "TEXT_MESSAGE_CHUNK", messageId: "x", role: "assistant", delta: "Showing only Kilele " },
  { type: "TEXT_MESSAGE_CHUNK", messageId: "x", role: "assistant", delta: "Bakery on the board." },
  { type: "TEXT_MESSAGE_END", messageId: "x" },
  { type: "RUN_FINISHED", threadId: "t", runId: "r" },
];

describe("Folder", () => {
  it("draws each tool call and each text stream as its own message", () => {
    const f = new Folder((name) => name.startsWith("board."));
    let t = f.start(emptyTranscript(), "Show only Kilele Bakery on the board.");
    expect(t.status).toBe("running");
    for (const e of RUN) t = f.apply(t, e);
    expect(t.status).toBe("idle");
    expect(t.messages.map((m) => [m.role, m.parts.map((p) => p.type).join()])).toEqual([
      ["user", "text"],
      ["assistant", "tool_call"],
      ["assistant", "tool_call"],
      ["assistant", "text"],
    ]);
    const [, read, filter, text] = t.messages;
    const call = (m: typeof read) => (m.parts[0].type === "tool_call" ? m.parts[0].toolCall : undefined);
    expect(call(read)).toMatchObject({ name: "settle.debt_plans", status: "complete", args: { asOf: "2026-11-10" } });
    expect(call(read)?.result).toMatchObject({ success: true, response: "1 plans" });
    // The board's own tool waits for the browser to run it, and is handed over to do so.
    expect(call(filter)).toMatchObject({ name: "board.filter", status: "executing", args: { counterparty: "Kilele Bakery" } });
    expect(t.clientCalls.map((c) => c.toolCallId)).toEqual(["c2"]);
    expect(text.parts[0]).toEqual({ type: "text", text: "Showing only Kilele Bakery on the board." });

    t = f.settle(t, "c2", true, "Board filtered to Kilele Bakery.");
    expect(call(t.messages[2])).toMatchObject({ status: "complete", result: { success: true, response: "Board filtered to Kilele Bakery." } });
  });

  it("keeps every snapshot immutable", () => {
    const f = new Folder();
    const t0 = f.start(emptyTranscript(), "hi");
    const t1 = f.apply(t0, { type: "TEXT_MESSAGE_CHUNK", messageId: "x", delta: "a" });
    const t2 = f.apply(t1, { type: "TEXT_MESSAGE_CHUNK", messageId: "x", delta: "b" });
    expect(t1.messages).not.toBe(t2.messages);
    expect(t1.messages[1].parts[0]).toEqual({ type: "text", text: "a" });
    expect(t2.messages[1].parts[0]).toEqual({ type: "text", text: "ab" });
  });

  it("shows an error, and cancels calls a stopped run left open", () => {
    const f = new Folder();
    let t = f.start(emptyTranscript(), "hi");
    t = f.apply(t, { type: "TOOL_CALL_START", toolCallId: "c", toolCallName: "books.cash" });
    t = f.apply(t, { type: "RUN_ERROR", message: "The Treasurer answered 502." });
    expect(t.status).toBe("error");
    expect(t.messages.at(-1)?.parts[0]).toEqual({ type: "error", message: "The Treasurer answered 502." });
    const c = t.messages[1].parts[0];
    expect(c.type === "tool_call" && c.toolCall.status).toBe("cancelled");
  });

  it("ignores what it doesn't know", () => {
    const f = new Folder();
    const t = f.start(emptyTranscript(), "hi");
    expect(f.apply(t, { type: "SOMETHING_NEW" })).toBe(t);
    expect(f.apply(t, null)).toBe(t);
  });
});

describe("parsePartial", () => {
  it("reads complete and half-streamed JSON", () => {
    expect(parsePartial('{"a":1}')).toEqual({ a: 1 });
    expect(parsePartial('{"a":"Kil')).toEqual({ a: "Kil" });
    expect(parsePartial('{"a":[1,2')).toEqual({ a: [1, 2] });
    expect(parsePartial("nope")).toBeUndefined();
  });
});

describe("patch", () => {
  it("applies AG-UI state deltas", () => {
    expect(patch({ a: 1 }, [{ op: "add", path: "/b", value: 2 }, { op: "replace", path: "/a", value: 3 }])).toEqual({ a: 3, b: 2 });
    expect(patch({ l: [1, 2] }, [{ op: "add", path: "/l/-", value: 3 }, { op: "remove", path: "/l/0" }])).toEqual({ l: [2, 3] });
  });
});

describe("readSse", () => {
  it("splits frames and skips non-JSON", async () => {
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        const enc = new TextEncoder();
        c.enqueue(enc.encode('data: {"type":"RUN_STARTED"}\n\n: keep-alive\n\ndata: {"type":"TEXT_'));
        c.enqueue(enc.encode('MESSAGE_CHUNK","delta":"hi"}\r\n\r\ndata: not json\n\n'));
        c.close();
      },
    });
    const out: unknown[] = [];
    for await (const e of readSse(body)) out.push(e);
    expect(out).toEqual([{ type: "RUN_STARTED" }, { type: "TEXT_MESSAGE_CHUNK", delta: "hi" }]);
  });
});
