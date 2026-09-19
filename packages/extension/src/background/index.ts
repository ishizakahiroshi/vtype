// Background service worker (plan C7b C1 / C2). A router, not a session owner:
//
// - creates the offscreen document on demand (one, reasons USER_MEDIA), guarding concurrent
//   creation and a service-worker restart that finds one already open;
// - relays content -> offscreen (start / stop) and offscreen -> the owning tab *and frame*
//   (tabs.sendMessage with {frameId}: the content script runs in every frame);
// - tells the offscreen document to abort when the owning tab closes or cannot be reached;
// - opens the microphone permission page on install and when a content script asks.
//
// Session state (which session is current, who owns it) lives in the offscreen document, which
// keeps running while Chrome stops and restarts this worker. Nothing here needs to survive a
// restart, so no storage permission is needed.

import {
  OFFSCREEN_PATH,
  PERMISSION_PATH,
  isContentToBackground,
  isOffscreenToBackground,
  type BackgroundToContent,
  type BackgroundToOffscreen,
  type Owner,
  type SessionEvent,
} from "../shared/messages";

interface MessageSender {
  tab?: { id?: number };
  frameId?: number;
}

interface ChromeEvent<L> {
  addListener(listener: L): void;
}

/** The part of the extension API the background uses (instead of @types/chrome). */
export interface BackgroundChrome {
  runtime: {
    getURL(path: string): string;
    sendMessage(message: unknown): Promise<unknown>;
    onMessage: ChromeEvent<(message: unknown, sender: MessageSender) => void>;
    onInstalled: ChromeEvent<(details: { reason: string }) => void>;
    getContexts?: (filter: { contextTypes: string[]; documentUrls?: string[] }) => Promise<unknown[]>;
  };
  tabs: {
    sendMessage(tabId: number, message: unknown, options?: { frameId?: number }): Promise<unknown>;
    create(props: { url: string }): Promise<unknown>;
    onRemoved: ChromeEvent<(tabId: number) => void>;
  };
  offscreen: {
    createDocument(params: { url: string; reasons: string[]; justification: string }): Promise<void>;
    hasDocument?: () => Promise<boolean>;
  };
}

export interface Background {
  /** How the last ensureOffscreen() found the document: for tests and diagnostics. */
  readonly lastOffscreenCheck: "getContexts" | "hasDocument" | "created" | "awaited" | null;
  ensureOffscreen(): Promise<void>;
}

export function createBackground(chrome: BackgroundChrome): Background {
  const offscreenUrl = chrome.runtime.getURL(OFFSCREEN_PATH);
  let creating: Promise<void> | null = null;
  let lastOffscreenCheck: Background["lastOffscreenCheck"] = null;

  async function offscreenExists(): Promise<boolean> {
    // getContexts (Chrome 116+) is the documented way; hasDocument is the older API.
    if (typeof chrome.runtime.getContexts === "function") {
      const contexts = await chrome.runtime.getContexts({
        contextTypes: ["OFFSCREEN_DOCUMENT"],
        documentUrls: [offscreenUrl],
      });
      lastOffscreenCheck = "getContexts";
      return contexts.length > 0;
    }
    if (typeof chrome.offscreen.hasDocument === "function") {
      lastOffscreenCheck = "hasDocument";
      return chrome.offscreen.hasDocument();
    }
    return false;
  }

  async function ensureOffscreen(): Promise<void> {
    if (creating !== null) {
      await creating;
      lastOffscreenCheck = "awaited";
      return;
    }
    if (await offscreenExists()) return;
    if (creating !== null) {
      // Another call started creating while we were checking.
      await creating;
      lastOffscreenCheck = "awaited";
      return;
    }
    creating = chrome.offscreen
      .createDocument({
        url: offscreenUrl,
        reasons: ["USER_MEDIA"],
        justification: "Run the browser's speech recognition for voice input into web text fields.",
      })
      .catch(async (err: unknown) => {
        // A restarted worker can race a document that already exists; that is fine.
        if (!(await offscreenExists())) throw err;
      });
    try {
      await creating;
      lastOffscreenCheck = "created";
    } finally {
      creating = null;
    }
  }

  function toOffscreen(message: BackgroundToOffscreen): Promise<unknown> {
    return chrome.runtime.sendMessage(message);
  }

  function toContent(owner: Owner, sessionId: string, event: SessionEvent): Promise<unknown> {
    const message: BackgroundToContent = { target: "content", type: "session-event", sessionId, event };
    return chrome.tabs.sendMessage(owner.tabId, message, { frameId: owner.frameId });
  }

  function openPermissionPage(): void {
    void chrome.tabs.create({ url: chrome.runtime.getURL(PERMISSION_PATH) }).catch(() => undefined);
  }

  async function start(sessionId: string, owner: Owner): Promise<void> {
    try {
      await ensureOffscreen();
      await toOffscreen({ target: "offscreen", type: "start", sessionId, owner });
    } catch {
      await toContent(owner, sessionId, { kind: "ended", reason: "error", code: "offscreen-unavailable" }).catch(
        () => undefined,
      );
    }
  }

  async function stop(sessionId: string, owner: Owner): Promise<void> {
    try {
      await toOffscreen({ target: "offscreen", type: "stop", sessionId });
    } catch {
      // No offscreen document is listening: there is no recording to stop. Let the content
      // script finish with what it already has instead of waiting.
      await toContent(owner, sessionId, { kind: "ended", reason: "user" }).catch(() => undefined);
    }
  }

  chrome.runtime.onMessage.addListener((message, sender) => {
    if (isContentToBackground(message)) {
      if (message.type === "open-permission") {
        openPermissionPage();
        return;
      }
      const tabId = sender.tab?.id;
      if (tabId === undefined) return; // not from a tab's content script
      const owner: Owner = { tabId, frameId: sender.frameId ?? 0 };
      if (message.type === "start") void start(message.sessionId, owner);
      else void stop(message.sessionId, owner);
      return;
    }
    if (isOffscreenToBackground(message)) {
      const { sessionId, owner, event } = message;
      toContent(owner, sessionId, event).catch(() => {
        // The tab navigated away or its frame is gone: nobody will ever stop this session.
        if (event.kind !== "ended") void toOffscreen({ target: "offscreen", type: "abort", sessionId }).catch(() => undefined);
      });
    }
  });

  chrome.tabs.onRemoved.addListener((tabId) => {
    void toOffscreen({ target: "offscreen", type: "abort", tabId }).catch(() => undefined);
  });

  chrome.runtime.onInstalled.addListener((details) => {
    if (details.reason === "install") openPermissionPage();
  });

  return {
    get lastOffscreenCheck() {
      return lastOffscreenCheck;
    },
    ensureOffscreen,
  };
}

const extensionChrome = (globalThis as { chrome?: BackgroundChrome & { runtime: { id?: string } } }).chrome;
if (extensionChrome?.runtime?.id !== undefined) createBackground(extensionChrome);
