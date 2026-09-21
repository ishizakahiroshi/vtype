// The diagnostic log (off by default). The rule the whole feature hangs on is that an entry
// never holds what was said: the privacy policy and the store submission both promise that
// transcripts are not stored, so the tests below spell the spoken words out and then assert
// they are nowhere in the log.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DIAG_LOG_KEY,
  MAX_DIAG_ENTRIES,
  appendDiag,
  clearDiagLog,
  describeSessionEvent,
  formatDiagLog,
  readDiagLog,
  sanitizeDiagLog,
  watchDiagLog,
  type DiagEntry,
} from "../src/shared/diagnostics";
import {
  DIAGNOSTICS_KEY,
  readDiagnostics,
  watchDiagnostics,
  writeDiagnostics,
  type StorageChangeListener,
  type StorageView,
} from "../src/shared/settings";
import { initOptionsPage } from "../src/options/options";
import { FakeChromeHub, FakeSpeechRecognition, flush } from "./fake-chrome";

const SPOKEN = "これは秘密の文章です";

interface Stub {
  readonly view: StorageView;
  readonly sync: Record<string, unknown>;
  readonly local: Record<string, unknown>;
}

/** chrome.storage with both areas, change events and an optional refusal (policy, quota). */
function stubStorage(initial: { sync?: Record<string, unknown>; local?: Record<string, unknown> } = {}, broken = false): Stub {
  const sync: Record<string, unknown> = structuredClone(initial.sync ?? {});
  const local: Record<string, unknown> = structuredClone(initial.local ?? {});
  const listeners: StorageChangeListener[] = [];
  const refuse = (): never => {
    throw new Error("storage is unavailable");
  };
  const areaView = (items: Record<string, unknown>, area: string) => ({
    get: async (keys: string | string[] | null) => {
      if (broken) refuse();
      const wanted = keys === null ? Object.keys(items) : typeof keys === "string" ? [keys] : keys;
      const out: Record<string, unknown> = {};
      for (const key of wanted) if (key in items) out[key] = structuredClone(items[key]);
      return out;
    },
    set: async (next: Record<string, unknown>) => {
      if (broken) refuse();
      const changes: Record<string, { newValue?: unknown }> = {};
      for (const [key, value] of Object.entries(next)) {
        items[key] = structuredClone(value);
        changes[key] = { newValue: structuredClone(value) };
      }
      for (const l of [...listeners]) l(changes, area);
    },
  });
  return {
    view: {
      sync: areaView(sync, "sync"),
      local: areaView(local, "local"),
      onChanged: {
        addListener: (l) => listeners.push(l),
        removeListener: (l) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
    sync,
    local,
  };
}

/** Let the pending storage promises run. */
async function settle(times = 6): Promise<void> {
  for (let i = 0; i < times; i++) await Promise.resolve();
}

describe("what an entry may contain", () => {
  it("describes a result by its length, never by its text", () => {
    const line = describeSessionEvent({
      kind: "result",
      recognitionId: 3,
      isCurrent: true,
      isFinal: true,
      transcript: SPOKEN,
    });
    expect(line).toBe(`id=3 result final=true current=true len=${SPOKEN.length}`);
    expect(line).not.toContain(SPOKEN);
  });

  it("describes the other events without inventing fields", () => {
    expect(describeSessionEvent({ kind: "started", recognitionId: 1 })).toBe("id=1 started");
    expect(describeSessionEvent({ kind: "activity", recognitionId: 2, activity: "speechstart" })).toBe(
      "id=2 activity speechstart",
    );
    expect(describeSessionEvent({ kind: "ended", reason: "silence" })).toBe("ended reason=silence");
    expect(describeSessionEvent({ kind: "ended", reason: "error", code: "not-allowed" })).toBe(
      "ended reason=error code=not-allowed",
    );
  });
});

describe("the stored log", () => {
  it("keeps entries in order and caps the oldest away", async () => {
    const store = stubStorage();
    for (let i = 0; i < MAX_DIAG_ENTRIES + 5; i++) await appendDiag(store.view, `line ${i}`, 1000 + i);

    const entries = await readDiagLog(store.view);
    expect(entries).toHaveLength(MAX_DIAG_ENTRIES);
    expect(entries[0]?.line).toBe("line 5");
    expect(entries[entries.length - 1]?.line).toBe(`line ${MAX_DIAG_ENTRIES + 4}`);
  });

  it("does not lose an entry when appends overlap", async () => {
    const store = stubStorage();
    await Promise.all([
      appendDiag(store.view, "a", 1),
      appendDiag(store.view, "b", 2),
      appendDiag(store.view, "c", 3),
    ]);
    expect((await readDiagLog(store.view)).map((e) => e.line)).toEqual(["a", "b", "c"]);
  });

  it("drops anything that is not an entry, and survives a storage that refuses", async () => {
    expect(sanitizeDiagLog([{ t: 1, line: "ok" }, { t: "x", line: "bad" }, null, 7])).toEqual([{ t: 1, line: "ok" }]);
    expect(sanitizeDiagLog("not an array")).toEqual([]);

    const broken = stubStorage({}, true);
    await expect(appendDiag(broken.view, "x")).resolves.toBeUndefined();
    expect(await readDiagLog(broken.view)).toEqual([]);
    expect(await clearDiagLog(broken.view)).toBe(0);

    expect(await readDiagLog(null)).toEqual([]);
    await expect(appendDiag(null, "x")).resolves.toBeUndefined();
  });

  it("empties on request and says how many were forgotten", async () => {
    const store = stubStorage();
    await appendDiag(store.view, "a", 1);
    await appendDiag(store.view, "b", 2);
    expect(await clearDiagLog(store.view)).toBe(2);
    expect(await readDiagLog(store.view)).toEqual([]);
    expect(await clearDiagLog(store.view)).toBe(0);
  });

  it("tells a listener about a log written elsewhere", async () => {
    const store = stubStorage();
    const seen: DiagEntry[][] = [];
    const stop = watchDiagLog(store.view, (entries) => seen.push(entries));

    await appendDiag(store.view, "a", 1);
    expect(seen.at(-1)?.map((e) => e.line)).toEqual(["a"]);

    stop();
    await appendDiag(store.view, "b", 2);
    expect(seen).toHaveLength(1);
  });

  it("formats each line with the wall clock and the gap from the previous one", () => {
    const base = new Date(2026, 8, 21, 18, 33, 5, 120).getTime();
    const text = formatDiagLog([
      { t: base, line: "id=1 started" },
      { t: base + 1500, line: "id=1 activity speechstart" },
    ]);
    const lines = text.split("\n");
    expect(lines[0]).toContain("18:33:05.120");
    expect(lines[0]).toContain("id=1 started");
    expect(lines[1]).toContain("+ 1500ms");
    expect(formatDiagLog([])).toBe("");
  });
});

describe("the setting that switches it on", () => {
  it("is off unless it is stored as exactly true", async () => {
    expect(await readDiagnostics(stubStorage().view)).toBe(false);
    expect(await readDiagnostics(stubStorage({ sync: { [DIAGNOSTICS_KEY]: "yes" } }).view)).toBe(false);
    expect(await readDiagnostics(stubStorage({ sync: { [DIAGNOSTICS_KEY]: true } }).view)).toBe(true);
    expect(await readDiagnostics(null)).toBe(false);
    expect(await readDiagnostics(stubStorage({}, true).view)).toBe(false);
  });

  it("is written, and a watcher hears it", async () => {
    const store = stubStorage();
    const seen: boolean[] = [];
    watchDiagnostics(store.view, (on) => seen.push(on));

    expect(await writeDiagnostics(store.view, true)).toBe(true);
    expect(store.sync[DIAGNOSTICS_KEY]).toBe(true);
    expect(seen).toEqual([true]);

    expect(await writeDiagnostics(store.view, false)).toBe(true);
    expect(seen).toEqual([true, false]);
    expect(await writeDiagnostics(null, true)).toBe(false);
  });
});

describe("the background records a session", () => {
  let hub: FakeChromeHub;

  beforeEach(() => {
    vi.useFakeTimers();
    FakeSpeechRecognition.reset();
    hub = new FakeChromeHub();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  async function session(store: Stub): Promise<void> {
    hub.startBackground(store.view);
    hub.contentRuntime(1, 0).onMessage.addListener(() => undefined);
    await flush(vi);
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId: "s1" });
    await flush(vi);
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    sr.fireResult(SPOKEN, true);
    await flush(vi);
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "stop", sessionId: "s1" });
    await flush(vi);
  }

  it("writes nothing while the setting is off", async () => {
    const store = stubStorage();
    await session(store);
    expect(store.local[DIAG_LOG_KEY]).toBeUndefined();
  });

  it("writes the session, and not one word of what was said", async () => {
    const store = stubStorage({ sync: { [DIAGNOSTICS_KEY]: true } });
    await session(store);

    const entries = await readDiagLog(store.view);
    const text = entries.map((e) => e.line).join("\n");
    expect(entries.length).toBeGreaterThan(2);
    expect(text).toContain("start requested tab=1 frame=0");
    expect(text).toContain("stop requested");
    expect(text).toContain(`len=${SPOKEN.length}`);
    // The one thing this feature must never do.
    expect(text).not.toContain(SPOKEN);
    expect(text).not.toContain("秘密");
  });

  it("stops recording as soon as the setting is switched off", async () => {
    const store = stubStorage({ sync: { [DIAGNOSTICS_KEY]: true } });
    hub.startBackground(store.view);
    hub.contentRuntime(1, 0).onMessage.addListener(() => undefined);
    await flush(vi);

    await writeDiagnostics(store.view, false);
    await flush(vi);
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId: "s1" });
    await flush(vi);

    expect(await readDiagLog(store.view)).toEqual([]);
  });
});

describe("the settings page", () => {
  /** The part of the options markup this feature owns. */
  function mount(): void {
    document.body.innerHTML = [
      `<h2 id="diag-title"></h2><p id="diag-lead"></p>`,
      `<label for="diag"><input type="checkbox" id="diag"><span id="diag-label"></span></label>`,
      `<p id="diag-hint"></p>`,
      `<div id="diag-actions"><button id="diag-copy" type="button"></button><button id="diag-clear" type="button"></button></div>`,
      `<p id="diag-empty"></p><pre id="diag-log" hidden></pre><p id="status"></p>`,
    ].join("");
  }

  const toggle = (): HTMLInputElement => document.getElementById("diag") as HTMLInputElement;
  const log = (): HTMLElement => document.getElementById("diag-log") as HTMLElement;
  const status = (): string => document.getElementById("status")?.textContent ?? "";

  it("shows the stored log, and says so when there is none", async () => {
    const store = stubStorage({ local: { [DIAG_LOG_KEY]: [{ t: Date.now(), line: "id=1 started" }] } });
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    expect(document.getElementById("diag-empty")?.hidden).toBe(false);
    await settle();

    expect(log().hidden).toBe(false);
    expect(log().textContent).toContain("id=1 started");
    expect(document.getElementById("diag-empty")?.hidden).toBe(true);
  });

  it("reflects the stored setting and writes what is ticked", async () => {
    const store = stubStorage({ sync: { [DIAGNOSTICS_KEY]: true } });
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    expect(toggle().checked).toBe(true);

    toggle().checked = false;
    toggle().dispatchEvent(new Event("change"));
    await settle();
    expect(store.sync[DIAGNOSTICS_KEY]).toBe(false);
    expect(status()).toBe("Saved.");
  });

  it("puts the tick back when the setting could not be stored", async () => {
    const store = stubStorage({}, true);
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    toggle().checked = true;
    toggle().dispatchEvent(new Event("change"));
    await settle();
    expect(toggle().checked).toBe(false);
    expect(status()).toContain("Could not save");
  });

  it("follows a recording made while the page is open", async () => {
    const store = stubStorage();
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();
    expect(log().hidden).toBe(true);

    await appendDiag(store.view, "id=1 started", Date.now());
    await settle();
    expect(log().hidden).toBe(false);
    expect(log().textContent).toContain("id=1 started");
  });

  it("empties the log on request", async () => {
    const store = stubStorage({ local: { [DIAG_LOG_KEY]: [{ t: Date.now(), line: "id=1 started" }] } });
    mount();
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    document.getElementById("diag-clear")?.dispatchEvent(new Event("click"));
    await settle();
    expect(log().hidden).toBe(true);
    expect(status()).toBe("The log was emptied.");
    expect(await readDiagLog(store.view)).toEqual([]);
  });

  it("copies the log, and says so when the clipboard refuses", async () => {
    const store = stubStorage({ local: { [DIAG_LOG_KEY]: [{ t: Date.now(), line: "id=1 started" }] } });
    mount();
    const copied: string[] = [];
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copied.push(text);
        },
      },
    });
    initOptionsPage({ storage: store.view, language: "en" });
    await settle();

    document.getElementById("diag-copy")?.dispatchEvent(new Event("click"));
    await settle();
    expect(copied[0]).toContain("id=1 started");
    expect(status()).toBe("Copied.");

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async () => {
          throw new Error("denied");
        },
      },
    });
    document.getElementById("diag-copy")?.dispatchEvent(new Event("click"));
    await settle();
    expect(status()).toContain("Could not copy");
  });
});
