import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSpeechRecognizer } from "vtype-core";
import { RETRY_FIRST_MS, SETUP_CLOSE_MS, createSpeechPage, socketUrl, type SocketLike, type SpeechPage } from "../src/speech/speech";
import { FakeSpeechRecognition, flush } from "./fake-chrome";

// The desktop app's speech page with the real offscreen session logic and vtype-core recognizer
// over a fake SpeechRecognition; the desktop app is a fake WebSocket.

class FakeSocket implements SocketLike {
  static all: FakeSocket[] = [];
  readyState = 0;
  readonly sent: Array<Record<string, unknown>> = [];
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }
  send(data: string): void {
    this.sent.push(JSON.parse(data) as Record<string, unknown>);
  }
  close(): void {
    this.readyState = 3;
  }
  open(): void {
    this.readyState = 1;
    this.onopen?.({});
  }
  receive(message: unknown): void {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
  drop(): void {
    this.readyState = 3;
    this.onclose?.({});
  }
  ofType(type: string): Array<Record<string, unknown>> {
    return this.sent.filter((m) => m.type === type);
  }
  events(): Array<Record<string, unknown>> {
    return this.ofType("session").map((m) => m.event as Record<string, unknown>);
  }
}

const PAGE = "http://127.0.0.1:47200/t/synthetic-token/speech";

let page: SpeechPage;
let closed = 0;

function socket(): FakeSocket {
  const s = FakeSocket.all.at(-1);
  if (s === undefined) throw new Error("no socket");
  return s;
}

function makePage(href = `${PAGE}?consent=1`, mic = "granted", doc: Document | null = null): SpeechPage {
  return createSpeechPage({
    href,
    doc,
    language: "ja",
    connect: (url) => new FakeSocket(url),
    queryMicrophone: async () => mic,
    getUserMedia: async () => ({ getTracks: () => [{ stop: () => undefined }] }) as unknown as MediaStream,
    createRecognizer: (lang) =>
      createSpeechRecognizer({ lang, SpeechRecognition: FakeSpeechRecognition as never, isChromium: true }),
    baseLang: () => "ja-JP",
    closeWindow: () => {
      closed += 1;
    },
  });
}

async function connected(): Promise<FakeSocket> {
  const s = socket();
  s.open();
  s.receive({ type: "hello", nativeVersion: "0.1.0", os: "windows" });
  await flush(vi);
  return s;
}

async function started(s: FakeSocket): Promise<FakeSpeechRecognition> {
  s.receive({ type: "start" });
  await flush(vi);
  const sr = FakeSpeechRecognition.started();
  sr.fireStart();
  await flush(vi);
  return sr;
}

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
  FakeSocket.all = [];
  closed = 0;
});

afterEach(() => {
  vi.useRealTimers();
});

describe("the speech page and the desktop app", () => {
  it("opens the WebSocket next to the page, token included", () => {
    expect(socketUrl(`${PAGE}?consent=1`)).toBe("ws://127.0.0.1:47200/t/synthetic-token/ws");
    page = makePage();
    expect(socket().url).toBe("ws://127.0.0.1:47200/t/synthetic-token/ws");
  });

  it("says hello first", async () => {
    page = makePage();
    const s = socket();
    s.open();
    await flush(vi);
    expect(s.sent[0]).toMatchObject({ type: "hello", extensionVersion: "desktop-page" });
  });

  it("start -> interim and final results go out as session events, with the state", async () => {
    page = makePage();
    const s = await connected();
    const sr = await started(s);
    sr.fireResult("こんに", false);
    sr.fireResult("こんにちは", true);
    await flush(vi);
    expect(s.events()).toEqual([
      { kind: "started" },
      { kind: "interim", text: "こんに" },
      { kind: "final", text: "こんにちは" },
    ]);
    expect(s.ofType("state").at(-1)).toEqual({ type: "state", mode: "normal", recording: true });
  });

  it("the recognizer's activity goes out too, for the ripple around the desktop app's mic", async () => {
    page = makePage();
    const s = await connected();
    const sr = await started(s);
    sr.onspeechstart?.({});
    await flush(vi);
    expect(s.events()).toContainEqual({ kind: "activity", activity: "speechstart" });
  });

  it("stop ends the session with ended:user and reports not recording", async () => {
    page = makePage();
    const s = await connected();
    const sr = await started(s);
    sr.fireResult("done", true);
    await flush(vi);
    s.receive({ type: "stop" });
    await flush(vi);
    expect(s.events().at(-1)).toEqual({ kind: "ended", reason: "user" });
    expect(s.ofType("state").at(-1)).toEqual({ type: "state", mode: "normal", recording: false });
    expect(page.offscreen.sessionId).toBeNull();
  });

  it("the desktop app's replacement table is applied to what is recognised", async () => {
    page = makePage();
    const s = await connected();
    s.receive({
      type: "native-config",
      config: {
        hotkey: null,
        icon: { visible: true, x: null, y: null, hideOnFullscreen: true },
        inject: "auto",
        besideField: { enabled: false, trigger: "focus" },
        extraExtensionIds: [],
        replacements: [{ from: "ブイタイプ", to: "vtype" }],
      },
    });
    const sr = await started(s);
    sr.fireResult("ブイタイプで入力", true);
    await flush(vi);
    expect(s.events().at(-1)).toEqual({ kind: "final", text: "vtypeで入力" });
  });

  it("set-mode is used by the next start", async () => {
    page = makePage();
    const s = await connected();
    s.receive({ type: "set-mode", mode: "en" });
    await flush(vi);
    expect(s.ofType("state").at(-1)).toMatchObject({ mode: "en" });
    const sr = await started(s);
    expect(sr.lang).toBe("en-US");
  });

  it("ignores messages that are not the desktop app's vocabulary", async () => {
    page = makePage();
    const s = await connected();
    const before = s.sent.length;
    s.receive({ type: "session", event: { kind: "started" } });
    s.receive({ type: "start", mode: "shouting" });
    s.onmessage?.({ data: "not json" });
    await flush(vi);
    expect(s.sent.length).toBe(before);
    expect(FakeSpeechRecognition.instances.some((i) => i.startCalls > 0)).toBe(false);
  });

  it("a lost connection stops the recording and is retried", async () => {
    page = makePage();
    const s = await connected();
    await started(s);
    s.drop();
    await flush(vi);
    expect(page.offscreen.sessionId).toBeNull();
    expect(FakeSocket.all).toHaveLength(1);
    await flush(vi, RETRY_FIRST_MS);
    expect(FakeSocket.all).toHaveLength(2);
  });

  it("does not put the session logic here (createOffscreen owns it)", () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const source = readFileSync(join(here, "..", "src", "speech", "speech.ts"), "utf8");
    expect(source).not.toMatch(/SILENT_CYCLE_LIMIT|STOP_GRACE_MS/);
  });
});

describe("first run", () => {
  it("before consent a start is refused and nothing is recognised", async () => {
    page = makePage(PAGE, "prompt");
    const s = await connected();
    s.receive({ type: "start" });
    await flush(vi);
    expect(FakeSpeechRecognition.instances.some((i) => i.startCalls > 0)).toBe(false);
    expect(s.events()).toEqual([{ kind: "ended", reason: "error", code: "consent-required" }]);
    expect(page.step).toBe("consent");
  });

  it("in a window that stays, the consent button sends consent, then asks for the microphone", async () => {
    document.body.innerHTML = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), "..", "src", "speech", "speech.html"),
      "utf8",
    ).replace(/<script[^>]*><\/script>/, "");
    page = makePage(`${PAGE}?stay=1`, "prompt", document);
    const s = await connected();
    expect(document.getElementById("consent-lead")?.textContent).toContain("Google へ送られ");
    expect(document.getElementById("consent-step")?.hidden).toBe(false);
    document.getElementById("consent-button")?.click();
    await flush(vi);
    expect(s.ofType("consent")).toHaveLength(1);
    expect(s.ofType("page-state").at(-1)).toEqual({ type: "page-state", consented: true, micGranted: true });
    expect(page.step).toBe("ready");
    expect(document.getElementById("ready-step")?.hidden).toBe(false);
    expect(document.getElementById("consent-step")?.hidden).toBe(true);
  });

  it("with consent in the URL and the microphone granted, the page is ready at once", async () => {
    page = makePage();
    const s = await connected();
    expect(page.step).toBe("ready");
    expect(s.ofType("page-state").at(-1)).toEqual({ type: "page-state", consented: true, micGranted: true });
    expect(s.ofType("consent")).toHaveLength(0);
  });
});

describe("the window", () => {
  it("the first-run window closes itself once the user agreed (the microphone is the desktop app's job)", async () => {
    page = makePage(`${PAGE}?setup=1`, "prompt");
    await connected();
    await page.consent();
    await flush(vi);
    expect(page.step).toBe("ready");
    expect(closed).toBe(0);
    await flush(vi, SETUP_CLOSE_MS);
    expect(closed).toBe(1);
  });

  it("the first-run window stays while the user has not agreed", async () => {
    page = makePage(`${PAGE}?setup=1`, "prompt");
    await connected();
    await flush(vi, SETUP_CLOSE_MS * 2);
    expect(closed).toBe(0);
  });

  it("an off-screen page that lost the microphone closes itself so it can come back on screen", async () => {
    page = makePage(`${PAGE}?consent=1`, "prompt");
    const s = await connected();
    expect(s.ofType("page-state").at(-1)).toEqual({ type: "page-state", consented: true, micGranted: false });
    await flush(vi, 500);
    expect(closed).toBe(1);
  });

  it("a window that stays never closes itself, even without the microphone", async () => {
    page = makePage(`${PAGE}?consent=1&stay=1`, "prompt");
    await connected();
    await flush(vi, SETUP_CLOSE_MS * 2);
    expect(page.step).toBe("microphone");
    expect(closed).toBe(0);
  });

  it("an off-screen page that is ready stays", async () => {
    page = makePage();
    await connected();
    await flush(vi, SETUP_CLOSE_MS * 2);
    expect(closed).toBe(0);
  });

  it("reports the page state only once the microphone was read", async () => {
    let answer: (state: string) => void = () => undefined;
    page = createSpeechPage({
      href: `${PAGE}?consent=1`,
      doc: null,
      connect: (url) => new FakeSocket(url),
      queryMicrophone: () => new Promise((resolve) => (answer = resolve)),
      closeWindow: () => undefined,
      createRecognizer: (lang) =>
        createSpeechRecognizer({ lang, SpeechRecognition: FakeSpeechRecognition as never, isChromium: true }),
    });
    const s = await connected();
    expect(s.ofType("page-state")).toEqual([]);
    answer("granted");
    await flush(vi);
    expect(s.ofType("page-state")).toEqual([{ type: "page-state", consented: true, micGranted: true }]);
  });

  function pageWithWindow(href: string): EventTarget {
    const win = new EventTarget();
    page = createSpeechPage({
      href,
      doc: null,
      win,
      connect: (url) => new FakeSocket(url),
      queryMicrophone: async () => "granted",
      closeWindow: () => undefined,
      createRecognizer: (lang) =>
        createSpeechRecognizer({ lang, SpeechRecognition: FakeSpeechRecognition as never, isChromium: true }),
    });
    return win;
  }

  it("a click on the off-screen window's taskbar button asks for the settings", async () => {
    const win = pageWithWindow(`${PAGE}?consent=1`);
    const s = await connected();
    // Chrome may focus the window when it starts it: that is not the user.
    win.dispatchEvent(new Event("focus"));
    expect(s.ofType("open-settings")).toEqual([]);
    win.dispatchEvent(new Event("blur"));
    win.dispatchEvent(new Event("focus"));
    expect(s.ofType("open-settings")).toEqual([{ type: "open-settings" }]);
  });

  it("the on-screen windows do not open the settings when focused", async () => {
    for (const href of [`${PAGE}?setup=1`, `${PAGE}?consent=1&stay=1`]) {
      FakeSocket.all = [];
      const win = pageWithWindow(href);
      const s = await connected();
      win.dispatchEvent(new Event("blur"));
      win.dispatchEvent(new Event("focus"));
      expect(s.ofType("open-settings")).toEqual([]);
    }
  });
});
