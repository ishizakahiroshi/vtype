// The desktop link: the background, the real offscreen document and vtype-core, with a fake
// Native Messaging port standing in for the desktop app.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RETRY_FIRST_MS, RETRY_MAX_MS } from "../src/background/native-bridge";
import {
  BRIDGE_STATUS_KEY,
  DESKTOP_BRIDGE_KEY,
  INPUT_MODE_KEY,
  NATIVE_CONFIG_KEY,
} from "../src/shared/settings";
import { FakeChromeHub, FakeSpeechRecognition, flush, type FakeNativePort } from "./fake-chrome";
import { stubStorage, type Stub } from "./stub-storage";

let hub: FakeChromeHub;
let store: Stub;
let session: Record<string, unknown>;

/** A stub with chrome.storage.session too. */
function storageWithSession(initial: { sync?: Record<string, unknown> } = {}): Stub {
  const s = stubStorage(initial);
  session = {};
  (s.view as { session?: unknown }).session = {
    get: async (keys: string[]) => Object.fromEntries(keys.filter((k) => k in session).map((k) => [k, session[k]])),
    set: async (items: Record<string, unknown>) => {
      Object.assign(session, structuredClone(items));
    },
  };
  return s;
}

async function boot(opts: { permission?: boolean; on?: boolean; installed?: boolean } = {}): Promise<void> {
  hub = new FakeChromeHub();
  hub.nativePermission = opts.permission ?? true;
  hub.nativeInstalled = opts.installed ?? true;
  store = storageWithSession({ sync: { [DESKTOP_BRIDGE_KEY]: opts.on ?? true } });
  hub.startBackground(store.view);
  await flush(vi);
}

/** The desktop app answers the extension's hello. */
async function nativeHello(port: FakeNativePort = hub.nativePort!): Promise<void> {
  port.receive({ type: "hello", nativeVersion: "0.1.0", os: "Windows 10.0.26200" });
  await flush(vi);
}

const sessionKinds = (port: FakeNativePort): string[] =>
  port.ofType("session").map((m) => {
    const e = m.event as { kind: string; reason?: string; text?: string };
    return e.kind === "ended" ? `ended:${e.reason}` : e.kind === "started" ? "started" : `${e.kind}:${e.text}`;
  });

beforeEach(() => {
  vi.useFakeTimers();
  FakeSpeechRecognition.reset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("connecting", () => {
  it("does not connect without the permission, or while the link is switched off", async () => {
    await boot({ permission: false });
    expect(hub.nativePorts).toHaveLength(0);
    await boot({ on: false });
    expect(hub.nativePorts).toHaveLength(0);
  });

  it("connects once both are there, says hello, and reports connected", async () => {
    await boot();
    expect(hub.nativePorts).toHaveLength(1);
    expect(hub.nativePort!.ofType("hello")).toEqual([{ type: "hello", extensionVersion: "0.1.0" }]);
    expect(session[BRIDGE_STATUS_KEY]).toEqual({ state: "connecting" });
    await nativeHello();
    expect(session[BRIDGE_STATUS_KEY]).toEqual({ state: "connected", nativeVersion: "0.1.0", os: "Windows 10.0.26200" });
    expect(hub.nativePort!.ofType("state")).toEqual([{ type: "state", mode: "normal", recording: false }]);
  });

  it("connects when the permission is granted later, and disconnects when it is switched off", async () => {
    await boot({ permission: false });
    hub.setNativePermission(true);
    await flush(vi);
    expect(hub.nativePorts).toHaveLength(1);
    await store.view.sync!.set({ [DESKTOP_BRIDGE_KEY]: false });
    await flush(vi);
    expect(hub.nativePort!.disconnected).toBe(true);
    expect(session[BRIDGE_STATUS_KEY]).toEqual({ state: "off" });
  });

  it("reconnects after a lost connection, waiting longer each time", async () => {
    await boot();
    await nativeHello();
    hub.nativePort!.drop();
    await flush(vi);
    expect(hub.nativePorts).toHaveLength(1);
    await flush(vi, RETRY_FIRST_MS);
    expect(hub.nativePorts).toHaveLength(2);
    hub.nativePort!.drop();
    await flush(vi, RETRY_FIRST_MS);
    expect(hub.nativePorts).toHaveLength(2); // now waits 2 s
    await flush(vi, RETRY_FIRST_MS);
    expect(hub.nativePorts).toHaveLength(3);
  });

  it("stops trying when the desktop app is not installed, until asked to try again", async () => {
    await boot({ installed: false });
    await flush(vi);
    expect(session[BRIDGE_STATUS_KEY]).toEqual({ state: "not-installed" });
    await flush(vi, RETRY_MAX_MS * 2);
    expect(hub.nativePorts).toHaveLength(1);

    hub.nativeInstalled = true;
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "native-retry" });
    await flush(vi);
    expect(hub.nativePorts).toHaveLength(2);
    await nativeHello();
    expect((session[BRIDGE_STATUS_KEY] as { state: string }).state).toBe("connected");
  });

  it("connects again from a restarted service worker", async () => {
    await boot();
    await nativeHello();
    hub.backgroundListeners.length = 0;
    hub.startBackground(store.view);
    await flush(vi);
    expect(hub.nativePorts).toHaveLength(2);
  });
});

describe("a recording the desktop app starts", () => {
  it("streams started, interim, final and ended to the desktop app", async () => {
    await boot();
    await nativeHello();
    const port = hub.nativePort!;
    port.receive({ type: "start" });
    await flush(vi);
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    sr.fireResult("こんに", false);
    sr.fireResult("こんにちは", true);
    await flush(vi);
    port.receive({ type: "stop" });
    await flush(vi);
    expect(sessionKinds(port)).toEqual(["started", "interim:こんに", "final:こんにちは", "ended:user"]);
    expect(port.ofType("state").at(-1)).toEqual({ type: "state", mode: "normal", recording: false });
  });

  it("uses the mode the desktop app asked for, for that recording only", async () => {
    await boot();
    await nativeHello();
    hub.nativePort!.receive({ type: "start", mode: "en" });
    await flush(vi);
    expect(FakeSpeechRecognition.started().lang).toBe("en-US");
    expect(store.sync[INPUT_MODE_KEY]).toBeUndefined();
  });

  it("supersedes a tab's recording, and is superseded by one", async () => {
    await boot();
    await nativeHello();
    const tabEvents: string[] = [];
    hub.contentRuntime(1, 0).onMessage.addListener((m) => {
      const e = (m as { event: { kind: string; reason?: string } }).event;
      tabEvents.push(e.kind === "ended" ? `ended:${e.reason}` : e.kind);
    });
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId: "tab-1" });
    await flush(vi);
    FakeSpeechRecognition.started().fireStart();
    await flush(vi);

    hub.nativePort!.receive({ type: "start" });
    await flush(vi);
    expect(tabEvents.at(-1)).toBe("ended:superseded");

    FakeSpeechRecognition.started().fireStart();
    await flush(vi);
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "start", sessionId: "tab-2" });
    await flush(vi);
    expect(sessionKinds(hub.nativePort!).at(-1)).toBe("ended:superseded");
  });

  it("stops the recording when the desktop app goes away", async () => {
    await boot();
    await nativeHello();
    hub.nativePort!.receive({ type: "start" });
    await flush(vi);
    const sr = FakeSpeechRecognition.started();
    sr.fireStart();
    await flush(vi);
    hub.nativePort!.drop();
    await flush(vi);
    expect(sr.stopCalls).toBe(1);
  });
});

describe("settings over the link", () => {
  it("stores a mode the desktop app sets, and reports it", async () => {
    await boot();
    await nativeHello();
    hub.nativePort!.receive({ type: "set-mode", mode: "kana" });
    await flush(vi);
    expect(store.sync[INPUT_MODE_KEY]).toBe("kana");
    expect(hub.nativePort!.ofType("state").at(-1)).toEqual({ type: "state", mode: "kana", recording: false });
  });

  it("tells the desktop app when the mode changes in the options page", async () => {
    await boot();
    await nativeHello();
    await store.view.sync!.set({ [INPUT_MODE_KEY]: "en" });
    await flush(vi);
    expect(hub.nativePort!.ofType("state").at(-1)).toEqual({ type: "state", mode: "en", recording: false });
  });

  it("keeps the desktop app's settings for the options page, and sends changes back", async () => {
    await boot();
    await nativeHello();
    const config = {
      hotkey: null,
      icon: { visible: true, x: null, y: null, hideOnFullscreen: true },
      inject: "auto",
      besideField: { enabled: false, trigger: "focus" },
      extraExtensionIds: [],
    };
    hub.nativePort!.receive({ type: "native-config", config });
    await flush(vi);
    expect(session[NATIVE_CONFIG_KEY]).toEqual(config);

    const changed = { ...config, inject: "paste" };
    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "native-config-set", config: changed });
    await flush(vi);
    expect(hub.nativePort!.ofType("set-native-config")).toEqual([{ type: "set-native-config", config: changed }]);

    await hub.contentRuntime(1, 0).sendMessage({ target: "background", type: "native-config-set", config: { bogus: 1 } });
    await flush(vi);
    expect(hub.nativePort!.ofType("set-native-config")).toHaveLength(1);
  });

  it("opens the options page when the desktop app asks", async () => {
    await boot();
    await nativeHello();
    hub.nativePort!.receive({ type: "open-options" });
    await flush(vi);
    expect(hub.optionsPageOpened).toHaveLength(1);
  });

  it("ignores messages it does not understand", async () => {
    await boot();
    await nativeHello();
    const before = hub.nativePort!.sent.length;
    hub.nativePort!.receive({ type: "launch-missiles" });
    await flush(vi);
    expect(hub.nativePort!.sent.length).toBe(before);
  });
});
