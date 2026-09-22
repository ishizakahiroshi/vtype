// The desktop link (native plan C4): the background's end of Chrome Native Messaging.
//
// The desktop app (packages/native) asks for a recording; the extension runs it in the offscreen
// document like any other session, with the desktop app as its owner, and reports back what was
// heard. Only the final text is typed there; interim text is shown in a bubble.
//
// It connects only when the user has both granted the optional `nativeMessaging` permission and
// switched the link on in the options page. A lost connection is retried from 1 s, doubling up to
// 60 s. A missing desktop app ("host not found") or one that does not allow this extension is not
// retried: nothing changes until the user installs it and presses "try again".
//
// The status lives in chrome.storage.session for the options page. The port also keeps the
// service worker alive while it is open (Chrome 105+); when Chrome stops the worker anyway, the
// next start of the worker connects again.

import type { InputMode } from "vtype-core";
import { describeBrowser } from "../shared/report";
import {
  BRIDGE_STATUS_KEY,
  DEFAULT_INPUT_MODE,
  NATIVE_CONFIG_KEY,
  readDesktopBridge,
  readInputMode,
  watchDesktopBridge,
  watchInputMode,
  writeInputMode,
  writeSessionValue,
  type StorageView,
} from "../shared/settings";
import type { SessionEvent } from "../shared/messages";
import {
  NATIVE_HOST,
  isNativeConfig,
  isNativeToExtension,
  type ExtensionToNative,
  type NativeSessionEvent,
  type NativeToExtension,
} from "../shared/native-messages";

export type BridgeState = "off" | "connecting" | "connected" | "not-installed" | "error";

export interface BridgeStatus {
  readonly state: BridgeState;
  readonly nativeVersion?: string;
  readonly os?: string;
  /** Why the link stopped (for `error`). */
  readonly error?: string;
}

interface ChromeEvent<L> {
  addListener(listener: L): void;
}

export interface NativePort {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: ChromeEvent<(message: unknown) => void>;
  onDisconnect: ChromeEvent<(port: NativePort) => void>;
}

/** The part of the extension API the bridge uses. */
export interface BridgeChrome {
  runtime: {
    connectNative?: (application: string) => NativePort;
    lastError?: { message?: string };
    getManifest?: () => { version: string };
    openOptionsPage?: () => Promise<void>;
  };
  permissions?: {
    contains(query: { permissions: string[] }): Promise<boolean>;
    onAdded?: ChromeEvent<(p: { permissions?: string[] }) => void>;
    onRemoved?: ChromeEvent<(p: { permissions?: string[] }) => void>;
  };
}

/** What the background does for the bridge. */
export interface BridgeHost {
  /** Start a recording owned by the desktop app. `mode` overrides the stored one for it alone. */
  startSession(mode: InputMode | undefined): void;
  stopSession(): void;
  recording(): boolean;
}

export interface Timers {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface NativeBridge {
  readonly status: BridgeStatus;
  /** An event of the session the desktop app owns. */
  sessionEvent(event: SessionEvent): void;
  /** The mode or the recording changed: tell the desktop app. */
  stateChanged(): void;
  /** The options page asks to try again (e.g. after installing the desktop app). */
  retry(): void;
  /** The options page changed the desktop app's settings. */
  setConfig(config: unknown): boolean;
}

export const RETRY_FIRST_MS = 1000;
export const RETRY_MAX_MS = 60_000;
export const NATIVE_PERMISSION = "nativeMessaging";

export function createNativeBridge(
  chrome: BridgeChrome,
  storage: StorageView | null,
  host: BridgeHost,
  timers: Timers = globalThis,
): NativeBridge {
  let status: BridgeStatus = { state: "off" };
  let port: NativePort | null = null;
  let retryDelay = RETRY_FIRST_MS;
  let retryTimer: unknown = null;
  let permitted = false;
  let switchedOn = false;
  let mode: InputMode = DEFAULT_INPUT_MODE;

  function setStatus(next: BridgeStatus): void {
    status = next;
    void writeSessionValue(storage, BRIDGE_STATUS_KEY, next);
  }

  function post(message: ExtensionToNative): void {
    try {
      port?.postMessage(message);
    } catch {
      // The port closed between the check and the post; onDisconnect handles it.
    }
  }

  function postState(): void {
    post({ type: "state", mode, recording: host.recording() });
  }

  function cancelRetry(): void {
    if (retryTimer !== null) timers.clearTimeout(retryTimer);
    retryTimer = null;
  }

  function wanted(): boolean {
    return permitted && switchedOn && typeof chrome.runtime.connectNative === "function";
  }

  function connect(): void {
    cancelRetry();
    if (port !== null || !wanted()) return;
    setStatus({ state: "connecting" });
    let next: NativePort;
    try {
      next = chrome.runtime.connectNative!(NATIVE_HOST);
    } catch (err) {
      setStatus({ state: "error", error: err instanceof Error ? err.message : String(err) });
      return;
    }
    port = next;
    next.onMessage.addListener((message) => {
      if (port === next) onNativeMessage(message);
    });
    next.onDisconnect.addListener(() => {
      if (port === next) onDisconnect();
    });
    const nav = (globalThis as { navigator?: Parameters<typeof describeBrowser>[0] }).navigator;
    const browser = describeBrowser(nav);
    post({
      type: "hello",
      extensionVersion: chrome.runtime.getManifest?.().version ?? "",
      ...(browser === "" ? {} : { browser }),
    });
  }

  function disconnect(): void {
    cancelRetry();
    const old = port;
    port = null;
    try {
      old?.disconnect();
    } catch {
      // Already closed.
    }
    setStatus({ state: "off" });
  }

  function onDisconnect(): void {
    const reason = chrome.runtime.lastError?.message ?? "";
    port = null;
    // Nobody is left to receive what the desktop app's recording hears.
    if (host.recording()) host.stopSession();
    if (!wanted()) {
      setStatus({ state: "off" });
      return;
    }
    if (/not found/i.test(reason)) {
      setStatus({ state: "not-installed" });
      return;
    }
    if (/forbidden/i.test(reason)) {
      setStatus({ state: "error", error: "forbidden" });
      return;
    }
    setStatus({ state: "connecting", ...(reason === "" ? {} : { error: reason }) });
    retryTimer = timers.setTimeout(() => {
      retryTimer = null;
      connect();
    }, retryDelay);
    retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
  }

  function onNativeMessage(message: unknown): void {
    if (!isNativeToExtension(message)) return;
    const m: NativeToExtension = message;
    switch (m.type) {
      case "hello":
        retryDelay = RETRY_FIRST_MS;
        setStatus({ state: "connected", nativeVersion: m.nativeVersion, os: m.os });
        postState();
        break;
      case "start":
        host.startSession(m.mode);
        break;
      case "stop":
        host.stopSession();
        break;
      case "set-mode":
        mode = m.mode;
        void writeInputMode(storage, m.mode).then(postState);
        break;
      case "get-state":
        postState();
        break;
      case "native-config":
        void writeSessionValue(storage, NATIVE_CONFIG_KEY, m.config);
        break;
      case "open-options":
        void chrome.runtime.openOptionsPage?.().catch(() => undefined);
        break;
    }
  }

  function toNativeEvent(event: SessionEvent): NativeSessionEvent | null {
    switch (event.kind) {
      case "started":
        return { kind: "started" };
      case "result":
        return event.isFinal ? { kind: "final", text: event.transcript } : { kind: "interim", text: event.transcript };
      case "ended":
        return event.code === undefined
          ? { kind: "ended", reason: event.reason }
          : { kind: "ended", reason: event.reason, code: event.code };
      default:
        return null; // activity drives the web page's waveform only
    }
  }

  async function evaluate(): Promise<void> {
    if (wanted()) connect();
    else if (port !== null || status.state !== "off") disconnect();
  }

  // What is allowed and what is switched on decide whether to connect.
  void Promise.all([
    chrome.permissions?.contains({ permissions: [NATIVE_PERMISSION] }).catch(() => false) ?? Promise.resolve(false),
    readDesktopBridge(storage),
    readInputMode(storage),
  ]).then(([p, on, m]) => {
    permitted = p;
    switchedOn = on;
    mode = m;
    void evaluate();
  });
  watchDesktopBridge(storage, (on) => {
    switchedOn = on;
    void evaluate();
  });
  watchInputMode(storage, (m) => {
    mode = m;
    postState();
  });
  chrome.permissions?.onAdded?.addListener((p) => {
    if (p.permissions?.includes(NATIVE_PERMISSION)) {
      permitted = true;
      void evaluate();
    }
  });
  chrome.permissions?.onRemoved?.addListener((p) => {
    if (p.permissions?.includes(NATIVE_PERMISSION)) {
      permitted = false;
      void evaluate();
    }
  });

  return {
    get status() {
      return status;
    },
    sessionEvent(event) {
      const e = toNativeEvent(event);
      if (e === null) return;
      post({ type: "session", event: e });
      if (e.kind === "started" || e.kind === "ended") postState();
    },
    stateChanged: postState,
    retry() {
      retryDelay = RETRY_FIRST_MS;
      if (port === null) {
        void (chrome.permissions?.contains({ permissions: [NATIVE_PERMISSION] }).catch(() => false) ??
          Promise.resolve(false)).then((p) => {
          permitted = p;
          void evaluate();
        });
      }
    },
    setConfig(config) {
      if (!isNativeConfig(config) || port === null) return false;
      post({ type: "set-native-config", config });
      return true;
    },
  };
}
