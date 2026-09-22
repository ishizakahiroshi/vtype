// Input modes end to end: storage -> background -> offscreen -> the owning tab. The recognizer is
// the real vtype-core one over a fake SpeechRecognition (fake-chrome.ts).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  INPUT_MODE_KEY,
  REPLACEMENTS_KEY,
  readInputMode,
  readReplacements,
  writeInputMode,
  writeReplacements,
} from "../src/shared/settings";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";
import { stubStorage, type Stub } from "./stub-storage";

let hub: FakeChromeHub;
let store: Stub;
const received: Array<Record<string, unknown>> = [];

function setup(initial: { sync?: Record<string, unknown>; local?: Record<string, unknown> } = {}): void {
  store = stubStorage(initial);
  hub = new FakeChromeHub();
  hub.startBackground(store.view);
  hub.contentRuntime(1, 0).onMessage.addListener((m) => {
    received.push((m as { event: Record<string, unknown> }).event);
  });
}

async function start(sessionId = "s1"): Promise<FakeSpeechRecognition> {
  await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId });
  await flush(vi);
  const sr = FakeSpeechRecognition.started();
  sr.fireStart();
  await flush(vi);
  return sr;
}

const results = (): Array<{ transcript: unknown; isFinal: unknown }> =>
  received.filter((e) => e.kind === "result").map((e) => ({ transcript: e.transcript, isFinal: e.isFinal }));

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
  received.length = 0;
});

afterEach(() => {
  vi.useRealTimers();
});

describe("the stored settings", () => {
  it("default to normal mode and no table, and ignore what they cannot read", async () => {
    expect(await readInputMode(stubStorage().view)).toBe("normal");
    expect(await readInputMode(stubStorage({ sync: { [INPUT_MODE_KEY]: "klingon" } }).view)).toBe("normal");
    expect(await readInputMode(stubStorage({}, true).view)).toBe("normal");
    expect(await readReplacements(stubStorage({ local: { [REPLACEMENTS_KEY]: "junk" } }).view)).toEqual([]);
    expect(await readInputMode(null)).toBe("normal");
  });

  it("keep the mode in sync and the table in local", async () => {
    const s = stubStorage();
    expect(await writeInputMode(s.view, "kana")).toBe(true);
    expect(await writeReplacements(s.view, [{ from: "a", to: "b" }, { from: "", to: "x" }])).toBe(true);
    expect(s.sync[INPUT_MODE_KEY]).toBe("kana");
    expect(s.local[REPLACEMENTS_KEY]).toEqual([{ from: "a", to: "b" }]);
    expect(s.sync[REPLACEMENTS_KEY]).toBeUndefined();
  });
});

describe("a session in each mode", () => {
  it("behaves as before when nothing is stored (normal, no table)", async () => {
    setup();
    const sr = await start();
    expect(sr.lang).toBe("ja-JP");
    sr.fireResult("ぶいたいぷ", false);
    sr.fireResult("ぶいたいぷです", true);
    await flush(vi);
    expect(results()).toEqual([
      { transcript: "ぶいたいぷ", isFinal: false },
      { transcript: "ぶいたいぷです", isFinal: true },
    ]);
  });

  it("recognizes English in en mode", async () => {
    setup({ sync: { [INPUT_MODE_KEY]: "en" } });
    const sr = await start();
    expect(sr.lang).toBe("en-US");
  });

  it("delivers interim and final results in katakana in kana mode", async () => {
    setup({ sync: { [INPUT_MODE_KEY]: "kana" } });
    const sr = await start();
    expect(sr.lang).toBe("ja-JP");
    sr.fireResult("てすと", false);
    sr.fireResult("てすとです", true);
    await flush(vi);
    expect(results()).toEqual([
      { transcript: "テスト", isFinal: false },
      { transcript: "テストデス", isFinal: true },
    ]);
  });

  it("applies the replacement table, and keeps a replaced word as registered in kana mode", async () => {
    setup({
      sync: { [INPUT_MODE_KEY]: "kana" },
      local: { [REPLACEMENTS_KEY]: [{ from: "ぶいたいぷ", to: "vtype" }] },
    });
    const sr = await start();
    sr.fireResult("ぶいたいぷをつかう", true);
    await flush(vi);
    expect(results()).toEqual([{ transcript: "vtypeヲツカウ", isFinal: true }]);
  });

  it("follows a mode changed in the options page for the next session", async () => {
    setup();
    await writeInputMode(store.view, "en");
    await flush(vi);
    const sr = await start();
    expect(sr.lang).toBe("en-US");
  });

  it("keeps the order when the final result waits for the dictionary", async () => {
    setup({ sync: { [INPUT_MODE_KEY]: "kana" } });
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    hub.reading = async (text) => {
      await gate;
      return text.replace("東京", "とうきょう");
    };
    const sr = await start();
    sr.fireResult("東京に", false);
    sr.fireResult("東京に行く", true);
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "stop", sessionId: "s1" });
    await flush(vi);
    // The final has not been sent yet, so neither may the `ended` behind it.
    expect(received.map((e) => e.kind)).toEqual(["started", "result"]);
    release();
    await flush(vi);
    const kinds = received.map((e) => e.kind);
    expect(kinds.slice(-2)).toEqual(["result", "ended"]);
    expect(results().at(-1)).toEqual({ transcript: "トウキョウニ行ク", isFinal: true });
  });
});
