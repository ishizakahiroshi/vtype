import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SILENT_CYCLE_LIMIT, STOP_GRACE_MS } from "../src/offscreen/offscreen";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";

// The offscreen document with the real vtype-core recognizer over a fake SpeechRecognition.
// Content scripts are represented by listeners registered on the hub; the background is real.

let hub: FakeChromeHub;
const received = new Map<string, Array<{ sessionId: string; event: Record<string, unknown> }>>();

function listen(tabId: number, frameId: number): void {
  const key = `${tabId}:${frameId}`;
  received.set(key, []);
  hub.contentRuntime(tabId, frameId).onMessage.addListener((m) => {
    received.get(key)?.push(m as { sessionId: string; event: Record<string, unknown> });
  });
}

function events(tabId: number, frameId = 0): Array<Record<string, unknown>> {
  return (received.get(`${tabId}:${frameId}`) ?? []).map((m) => m.event);
}

async function startSession(tabId: number, sessionId: string, frameId = 0): Promise<FakeSpeechRecognition> {
  await hub.contentRuntime(tabId, frameId).sendMessage({ target: "background", type: "start", sessionId });
  await flush(vi);
  const sr = FakeSpeechRecognition.started();
  sr.fireStart();
  await flush(vi);
  return sr;
}

async function stopSession(tabId: number, sessionId: string): Promise<void> {
  await hub.contentRuntime(tabId, 0).sendMessage({ target: "background", type: "stop", sessionId });
  await flush(vi);
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
  received.clear();
  hub = new FakeChromeHub();
  hub.startBackground();
  listen(1, 0);
  listen(2, 0);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("offscreen recognition session", () => {
  it("forwards started, interim and final results to the owner", async () => {
    const sr = await startSession(1, "s1");
    sr.fireResult("hel", false);
    sr.fireResult("hello", true);
    await flush(vi);
    expect(events(1)).toEqual([
      { kind: "started", recognitionId: expect.any(Number) },
      { kind: "result", recognitionId: expect.any(Number), isCurrent: true, transcript: "hel", isFinal: false },
      { kind: "result", recognitionId: expect.any(Number), isCurrent: true, transcript: "hello", isFinal: true },
    ]);
    expect(sr.lang).toBe("ja-JP");
    expect(hub.offscreen?.sessionId).toBe("s1");
  });

  it("[C7c] forwards the recognizer's activity events to the owner", async () => {
    const sr = await startSession(1, "s1");
    sr.onaudiostart?.({});
    sr.onsoundstart?.({});
    sr.onspeechstart?.({});
    sr.onspeechend?.({});
    sr.onaudioend?.({});
    await flush(vi);
    expect(events(1).filter((e) => e.kind === "activity").map((e) => e.activity)).toEqual([
      "audiostart",
      "soundstart",
      "speechstart",
      "speechend",
      "audioend",
    ]);
    expect(events(1).every((e) => e.kind !== "activity" || typeof e.recognitionId === "number")).toBe(true);
  });

  it("[C7c] activity events of an instance older than the session are dropped", async () => {
    const first = await startSession(1, "s1");
    await hub.contentRuntime(2, 0).sendMessage({ target: "background", type: "start", sessionId: "s2" });
    await flush(vi);
    const before = events(2).length;
    first.onsoundstart?.({}); // the superseded instance still fires
    await flush(vi);
    expect(events(2).length).toBe(before);
  });

  it("user stop with nothing pending ends at once with reason user", async () => {
    const sr = await startSession(1, "s1");
    sr.fireResult("done", true);
    await stopSession(1, "s1");
    expect(sr.stopCalls).toBe(1);
    expect(events(1).at(-1)).toEqual({ kind: "ended", reason: "user" });
    expect(hub.offscreen?.sessionId).toBeNull();
  });

  it("user stop with a pending interim waits for the late final of the old instance (isCurrent: false)", async () => {
    const sr = await startSession(1, "s1");
    sr.fireResult("こんに", false);
    await stopSession(1, "s1");
    expect(events(1).some((e) => e.kind === "ended")).toBe(false);
    sr.fireResult("こんにちは", true); // Chrome delivers it after stop() from the replaced instance
    await flush(vi);
    const tail = events(1).slice(-2);
    expect(tail[0]).toMatchObject({ kind: "result", transcript: "こんにちは", isFinal: true, isCurrent: false });
    expect(tail[1]).toEqual({ kind: "ended", reason: "user" });
  });

  it("the grace period is bounded", async () => {
    const sr = await startSession(1, "s1");
    sr.fireResult("pending", false);
    await stopSession(1, "s1");
    await flush(vi, STOP_GRACE_MS - 100);
    expect(events(1).some((e) => e.kind === "ended")).toBe(false);
    await flush(vi, 200);
    expect(events(1).at(-1)).toEqual({ kind: "ended", reason: "user" });
  });

  it("Chrome ending a recognition by itself starts the next cycle (recording lasts until the user stops)", async () => {
    const first = await startSession(1, "s1");
    first.fireResult("one", true);
    first.fireEnd();
    await flush(vi);
    const second = FakeSpeechRecognition.started();
    expect(second).not.toBe(first);
    expect(second.startCalls).toBe(1);
    second.fireStart();
    second.fireResult("two", true);
    await flush(vi);
    expect(events(1).filter((e) => e.kind === "result").map((e) => e.transcript)).toEqual(["one", "two"]);
    expect(events(1).some((e) => e.kind === "ended")).toBe(false);
    expect(events(1).filter((e) => e.kind === "started")).toHaveLength(1);
  });

  it(`ends with silence after ${SILENT_CYCLE_LIMIT} cycles in a row without text; text resets the count`, async () => {
    let sr = await startSession(1, "s1");
    sr.fireError("no-speech");
    sr.fireEnd();
    await flush(vi);
    sr = FakeSpeechRecognition.started();
    sr.fireStart();
    sr.fireResult("heard", true); // resets
    sr.fireEnd();
    await flush(vi);
    for (let i = 0; i < SILENT_CYCLE_LIMIT; i++) {
      expect(events(1).some((e) => e.kind === "ended")).toBe(false);
      sr = FakeSpeechRecognition.started();
      sr.fireStart();
      sr.fireError("no-speech");
      sr.fireEnd();
      await flush(vi);
    }
    expect(events(1).at(-1)).toEqual({ kind: "ended", reason: "silence" });
  });

  it("not-allowed ends the session with the code (offscreen cannot prompt; C1 spike)", async () => {
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId: "s1" });
    await flush(vi);
    const sr = FakeSpeechRecognition.started();
    sr.fireError("not-allowed");
    sr.fireEnd();
    await flush(vi);
    expect(events(1).at(-1)).toEqual({ kind: "ended", reason: "error", code: "not-allowed" });
    expect(FakeSpeechRecognition.instances.filter((i) => i.startCalls > 0)).toHaveLength(1); // no retry loop
  });

  it("a late error from an older instance does not end the session", async () => {
    const first = await startSession(1, "s1");
    first.fireEnd(); // natural end: next cycle
    await flush(vi);
    const second = FakeSpeechRecognition.started();
    second.fireStart();
    first.fireError("network"); // late, from the replaced instance: ends the current recording in core
    await flush(vi);
    expect(events(1).some((e) => e.kind === "ended")).toBe(false);
    const third = FakeSpeechRecognition.started();
    expect(third).not.toBe(second);
  });

  it("stop for an unknown session is answered with ended:user so the page does not wait", async () => {
    const sr = await startSession(1, "s1");
    sr.fireResult("x", true);
    await stopSession(1, "s1");
    const before = events(1).length;
    await stopSession(1, "s1"); // again: the session is already gone
    expect(events(1).slice(before)).toEqual([{ kind: "ended", reason: "user" }]);
  });
});
