// Test harness (not a test file): an in-memory Chrome extension message bus plus a fake
// SpeechRecognition that the real vtype-core recognizer drives.
//
// Delivery rules mirror Chrome's:
// - runtime.sendMessage from the background or the offscreen document reaches the *other*
//   extension pages (never the sender, never content scripts);
// - runtime.sendMessage from a content script reaches every extension page (background and
//   offscreen), with sender.tab / sender.frameId;
// - tabs.sendMessage(tabId, msg, {frameId}) reaches only that frame's content script (all
//   frames of the tab when frameId is omitted); it rejects when nobody listens;
// - delivery is asynchronous.

import { createSpeechRecognizer } from "vtype-core";
import { createBackground, type Background, type BackgroundChrome } from "../src/background/index";
import { createOffscreen, type Offscreen } from "../src/offscreen/offscreen";
import type { ContentRuntime } from "../src/content/controller";
import type { StorageView } from "../src/shared/settings";

type Listener = (message: unknown, sender: { tab?: { id?: number }; frameId?: number }) => void;

const NO_RECEIVER = "Could not establish connection. Receiving end does not exist.";

// ---- fake SpeechRecognition ---------------------------------------------------------------

export class FakeSpeechRecognition {
  static instances: FakeSpeechRecognition[] = [];
  lang = "";
  continuous = false;
  interimResults = false;
  maxAlternatives = 1;
  onstart: ((ev: unknown) => void) | null = null;
  onaudiostart: ((ev: unknown) => void) | null = null;
  onsoundstart: ((ev: unknown) => void) | null = null;
  onspeechstart: ((ev: unknown) => void) | null = null;
  onspeechend: ((ev: unknown) => void) | null = null;
  onsoundend: ((ev: unknown) => void) | null = null;
  onaudioend: ((ev: unknown) => void) | null = null;
  onresult: ((ev: unknown) => void) | null = null;
  onnomatch: ((ev: unknown) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  onend: ((ev: unknown) => void) | null = null;
  startCalls = 0;
  stopCalls = 0;
  abortCalls = 0;

  constructor() {
    FakeSpeechRecognition.instances.push(this);
  }
  start(): void {
    this.startCalls++;
  }
  stop(): void {
    this.stopCalls++;
  }
  abort(): void {
    this.abortCalls++;
  }
  fireStart(): void {
    this.onstart?.({});
  }
  fireResult(transcript: string, isFinal: boolean): void {
    const result = Object.assign([{ transcript, confidence: 0.9 }], { isFinal });
    this.onresult?.({ resultIndex: 0, results: [result] });
  }
  fireError(error: string): void {
    this.onerror?.({ error, message: "" });
  }
  fireEnd(): void {
    this.onend?.({});
  }
  static reset(): void {
    FakeSpeechRecognition.instances = [];
  }
  /** The instance vtype-core started most recently. */
  static started(): FakeSpeechRecognition {
    const s = [...FakeSpeechRecognition.instances].reverse().find((i) => i.startCalls > 0);
    if (s === undefined) throw new Error("no started recognition");
    return s;
  }
}

// ---- message bus -------------------------------------------------------------------------

export interface Delivered {
  tabId: number;
  frameId: number;
  message: unknown;
}

export class FakeChromeHub {
  readonly backgroundListeners: Listener[] = [];
  readonly offscreenListeners: Listener[] = [];
  readonly contentListeners = new Map<string, Array<(m: unknown) => void>>();
  readonly toContent: Delivered[] = [];
  readonly toOffscreen: unknown[] = [];
  readonly createdTabs: string[] = [];
  readonly createDocumentCalls: string[] = [];
  readonly removedListeners: Array<(tabId: number) => void> = [];
  readonly installedListeners: Array<(d: { reason: string }) => void> = [];
  offscreenOpen = false;
  offscreen: Offscreen | null = null;
  background: Background | null = null;
  /** Use getContexts (true) or only hasDocument (false) for the existence check. */
  withGetContexts = true;
  /** Hold createDocument until release() (for concurrency tests). */
  holdCreate = false;
  private releaseCreate: (() => void) | null = null;

  private deliver(listeners: Listener[], message: unknown, sender: Parameters<Listener>[1]): Promise<unknown> {
    if (listeners.length === 0) return Promise.reject(new Error(NO_RECEIVER));
    return new Promise((resolve) => {
      queueMicrotask(() => {
        for (const l of [...listeners]) l(structuredClone(message), sender);
        resolve(undefined);
      });
    });
  }

  backgroundChrome(storage?: StorageView): BackgroundChrome {
    const hub = this;
    const chrome: BackgroundChrome = {
      ...(storage === undefined ? {} : { storage }),
      runtime: {
        getURL: (path) => `chrome-extension://synthetic-id/${path}`,
        sendMessage: (message) => {
          hub.toOffscreen.push(message);
          return hub.deliver(hub.offscreenListeners, message, {});
        },
        onMessage: { addListener: (l) => hub.backgroundListeners.push(l) },
        onInstalled: { addListener: (l) => hub.installedListeners.push(l) },
      },
      tabs: {
        sendMessage: (tabId, message, options) => {
          const keys = [...hub.contentListeners.keys()].filter((k) =>
            options?.frameId === undefined ? k.startsWith(`${tabId}:`) : k === `${tabId}:${options.frameId}`,
          );
          const listeners = keys.flatMap((k) => hub.contentListeners.get(k) ?? []);
          if (listeners.length === 0) return Promise.reject(new Error(NO_RECEIVER));
          hub.toContent.push({ tabId, frameId: options?.frameId ?? -1, message });
          return new Promise((resolve) => {
            queueMicrotask(() => {
              for (const l of listeners) l(structuredClone(message));
              resolve(undefined);
            });
          });
        },
        create: async ({ url }) => {
          hub.createdTabs.push(url);
          return {};
        },
        onRemoved: { addListener: (l) => hub.removedListeners.push(l) },
      },
      offscreen: {
        createDocument: async ({ url }) => {
          hub.createDocumentCalls.push(url);
          if (hub.holdCreate) await new Promise<void>((r) => (hub.releaseCreate = r));
          if (hub.offscreenOpen) throw new Error("Only a single offscreen document may be created.");
          hub.openOffscreen();
        },
        hasDocument: async () => hub.offscreenOpen,
      },
    };
    if (this.withGetContexts) {
      chrome.runtime.getContexts = async () => (hub.offscreenOpen ? [{ contextType: "OFFSCREEN_DOCUMENT" }] : []);
    }
    return chrome;
  }

  release(): void {
    this.releaseCreate?.();
    this.releaseCreate = null;
  }

  startBackground(storage?: StorageView): Background {
    this.background = createBackground(this.backgroundChrome(storage));
    return this.background;
  }

  /** What createDocument does: load offscreen.ts with the real vtype-core recognizer. */
  openOffscreen(): void {
    const hub = this;
    this.offscreenOpen = true;
    this.offscreen = createOffscreen({
      chrome: {
        runtime: {
          sendMessage: (message) => hub.deliver(hub.backgroundListeners, message, {}),
          onMessage: { addListener: (l) => hub.offscreenListeners.push(l) },
        },
      },
      recognizer: createSpeechRecognizer({
        lang: "ja-JP",
        SpeechRecognition: FakeSpeechRecognition as never,
        isChromium: true,
      }),
    });
  }

  /** chrome.runtime as seen by the content script in one frame. */
  contentRuntime(tabId: number, frameId: number): ContentRuntime {
    const hub = this;
    const key = `${tabId}:${frameId}`;
    return {
      sendMessage: (message) => {
        const sender = { tab: { id: tabId }, frameId };
        // Content-script messages reach every extension page: background and offscreen.
        return hub.deliver([...hub.backgroundListeners, ...hub.offscreenListeners], message, sender);
      },
      onMessage: {
        addListener: (l) => hub.contentListeners.set(key, [...(hub.contentListeners.get(key) ?? []), l]),
        removeListener: (l) => hub.contentListeners.set(key, (hub.contentListeners.get(key) ?? []).filter((x) => x !== l)),
      },
    };
  }

  /** The tab's content script goes away (navigation / close). */
  dropContent(tabId: number): void {
    for (const k of [...this.contentListeners.keys()]) if (k.startsWith(`${tabId}:`)) this.contentListeners.delete(k);
  }

  closeTab(tabId: number): void {
    this.dropContent(tabId);
    for (const l of this.removedListeners) l(tabId);
  }

  /** Messages the background sent to one tab/frame, as session events. */
  eventsTo(tabId: number, frameId: number): Array<{ sessionId: string; event: { kind: string; [k: string]: unknown } }> {
    return this.toContent
      .filter((d) => d.tabId === tabId && d.frameId === frameId)
      .map((d) => d.message as { sessionId: string; event: { kind: string } });
  }
}

/** Let queued microtasks and 0 ms timers run (fake timers). */
export async function flush(vi: { advanceTimersByTimeAsync(ms: number): Promise<unknown> }, ms = 0): Promise<void> {
  for (let i = 0; i < 5; i++) await vi.advanceTimersByTimeAsync(ms === 0 ? 0 : ms / 5);
}
