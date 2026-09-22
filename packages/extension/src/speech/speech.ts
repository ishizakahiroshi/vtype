// The desktop app's speech page (standalone plan C2).
//
// The desktop app (packages/native) serves this page on 127.0.0.1 and opens it in a Chrome of
// its own, so it can recognise speech without the extension. Recognition is the offscreen
// document's createOffscreen, unchanged: this file only hands it a `chrome.runtime` look-alike
// whose messages travel over a WebSocket to the desktop app, and speaks the same JSON as the
// extension's Native Messaging bridge (shared/native-messages.ts), so the Rust side keeps its
// vocabulary.
//
//   page URL   http://127.0.0.1:<port>/t/<token>/speech[?consent=1][&setup=1]
//   WebSocket  ws://127.0.0.1:<port>/t/<token>/ws      (the page's own directory + "ws")
//   dictionary /t/<token>/dict/                        (relative, so reading.ts works as is)
//
// First run: nothing is recognised before the user agrees that their voice goes to Google
// through Chrome's speech recognition (Microsoft Store policy 10.5.2). The desktop app keeps the
// answer in its config.json and says so with `?consent=1` in the URL. Then the microphone is
// asked for once; the grant stays with the desktop app's Chrome profile.
//
// The window: the desktop app opens it on screen for the first run (`setup=1`) and off screen
// otherwise. A page cannot move its own window off screen (Chrome pulls `window.moveTo` back
// onto it), so once the setup is done the page closes itself and the desktop app starts it again
// off screen. An off-screen page that finds consent or the microphone missing closes itself too,
// and comes back on screen.
//
// A lost WebSocket is retried from 1 s, doubling up to 30 s. A recording that loses the desktop
// app is stopped: nobody is left to type what it hears.

import type { InputMode, ReadingProvider, SpeechRecognizer } from "vtype-core";
import { createOffscreen, type Offscreen } from "../offscreen/offscreen";
import { createReadingProvider } from "../offscreen/reading";
import { translator } from "../shared/i18n";
import {
  NATIVE_OWNER,
  isOffscreenToBackground,
  type BackgroundToOffscreen,
} from "../shared/messages";
import {
  isNativeToExtension,
  toNativeEvent,
  type ExtensionToNative,
} from "../shared/native-messages";
import { describeBrowser } from "../shared/report";
import { DEFAULT_INPUT_MODE } from "../shared/settings";

export const RETRY_FIRST_MS = 1000;
export const RETRY_MAX_MS = 30_000;
/** What the page calls itself in its `hello` (the desktop app has no extension version to show). */
export const PAGE_VERSION = "desktop-page";
/** How long the first-run window stays up after the setup is done, so the user sees it worked. */
export const SETUP_CLOSE_MS = 1500;

/** The part of a WebSocket the page uses. */
export interface SocketLike {
  readonly readyState: number;
  send(data: string): void;
  close(): void;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
}

const OPEN = 1;

type GetUserMedia = (constraints: MediaStreamConstraints) => Promise<MediaStream>;

export type PageStep = "consent" | "microphone" | "ready";

export interface SpeechPageOptions {
  /** Opens the WebSocket. Default: `new WebSocket(url)`. */
  connect?: (url: string) => SocketLike;
  /** The page's own URL. Default: `location.href`. */
  href?: string;
  doc?: Document | null;
  language?: string;
  getUserMedia?: GetUserMedia;
  queryMicrophone?: () => Promise<string>;
  /** Passed through to createOffscreen (tests give a fake recognizer). */
  recognizer?: SpeechRecognizer;
  createRecognizer?: (lang: () => string) => SpeechRecognizer;
  baseLang?: () => string;
  reading?: ReadingProvider;
  /** Closes this window. Default: `window.close()` (it works for the `--app` window). */
  closeWindow?: () => void;
}

export interface SpeechPage {
  readonly offscreen: Offscreen;
  readonly step: PageStep;
  /** The user agreed (the "agree and start" button). */
  consent(): Promise<void>;
  /** Ask for the microphone (the button on the microphone step). */
  requestMicrophone(): Promise<void>;
}

/** `ws://host/t/<token>/ws` for a page at `http://host/t/<token>/speech`. */
export function socketUrl(href: string): string {
  const url = new URL(href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = url.pathname.replace(/[^/]*$/, "ws");
  url.search = "";
  url.hash = "";
  return url.toString();
}

export function createSpeechPage(options: SpeechPageOptions = {}): SpeechPage {
  const nav = globalThis.navigator as (Navigator & Parameters<typeof describeBrowser>[0]) | undefined;
  const href = options.href ?? globalThis.location?.href ?? "http://127.0.0.1/speech";
  const connect = options.connect ?? ((url: string) => new WebSocket(url) as unknown as SocketLike);
  const doc = options.doc === undefined ? (globalThis.document ?? null) : options.doc;
  const t = translator(options.language ?? nav?.language);
  const getUserMedia: GetUserMedia | undefined =
    options.getUserMedia ?? (nav?.mediaDevices ? (c) => nav.mediaDevices.getUserMedia(c) : undefined);
  const queryMicrophone =
    options.queryMicrophone ??
    (async () => (await nav!.permissions.query({ name: "microphone" as PermissionName })).state);

  const params = new URL(href).searchParams;
  let consented = params.get("consent") === "1";
  const setup = params.get("setup") === "1";
  const closeWindow = options.closeWindow ?? (() => globalThis.close?.());
  /** The microphone's state was read once; until then the page state is not reported. */
  let micKnown = false;
  let closing = false;
  let micGranted = false;
  let mode: InputMode = DEFAULT_INPUT_MODE;
  let sessionId: string | null = null;
  let socket: SocketLike | null = null;
  let connected = false;
  let retryDelay = RETRY_FIRST_MS;
  let sessionCount = 0;

  // ---- the chrome.runtime that createOffscreen talks to -------------------------------------

  const offscreenListeners: Array<(message: unknown) => void> = [];

  function toOffscreen(message: BackgroundToOffscreen): void {
    for (const l of [...offscreenListeners]) l(message);
  }

  const offscreen = createOffscreen({
    chrome: {
      runtime: {
        sendMessage: async (message) => {
          fromOffscreen(message);
          return undefined;
        },
        onMessage: { addListener: (l) => void offscreenListeners.push(l) },
      },
    },
    ...(options.recognizer === undefined ? {} : { recognizer: options.recognizer }),
    ...(options.createRecognizer === undefined ? {} : { createRecognizer: options.createRecognizer }),
    ...(options.baseLang === undefined ? {} : { baseLang: options.baseLang }),
    ...(options.reading === undefined ? {} : { reading: options.reading }),
  });

  function fromOffscreen(message: unknown): void {
    if (!isOffscreenToBackground(message) || message.sessionId !== sessionId) return;
    const event = toNativeEvent(message.event);
    if (event === null) return;
    if (event.kind === "ended") sessionId = null;
    post({ type: "session", event });
    if (event.kind === "started" || event.kind === "ended") postState();
  }

  // ---- the desktop app ----------------------------------------------------------------------

  function post(message: ExtensionToNative): void {
    if (socket === null || socket.readyState !== OPEN) return;
    try {
      socket.send(JSON.stringify(message));
    } catch {
      // Closing; onclose handles it.
    }
  }

  function postState(): void {
    post({ type: "state", mode, recording: sessionId !== null });
  }

  function postPageState(): void {
    if (micKnown) post({ type: "page-state", consented, micGranted });
  }

  /**
   * The window is in the wrong place for what is needed now: the first-run window is done, or an
   * off-screen page needs the user. It closes; the desktop app starts it again in the right place.
   */
  function placeWindow(): void {
    if (closing || !micKnown || socket === null || !connected) return;
    const ready = step() === "ready";
    if (setup && ready) {
      closing = true;
      setTimeout(closeWindow, SETUP_CLOSE_MS);
    } else if (!setup && !ready) {
      closing = true;
      setTimeout(closeWindow, 200);
    }
  }

  function startSession(requested: InputMode | undefined): void {
    if (sessionId !== null) return;
    if (!consented) {
      // Nothing is recognised before the user agreed; the desktop app shows why.
      post({ type: "session", event: { kind: "ended", reason: "error", code: "consent-required" } });
      return;
    }
    sessionCount += 1;
    sessionId = globalThis.crypto?.randomUUID?.() ?? `desktop-${Date.now()}-${sessionCount}`;
    // Replacement rules come with the desktop app's settings page (standalone plan C5).
    toOffscreen({ target: "offscreen", type: "start", sessionId, owner: NATIVE_OWNER, mode: requested ?? mode, rules: [] });
  }

  function stopSession(): void {
    if (sessionId === null) return;
    toOffscreen({ target: "offscreen", type: "stop", sessionId });
  }

  function onDesktopMessage(data: unknown): void {
    let message: unknown;
    try {
      message = typeof data === "string" ? JSON.parse(data) : null;
    } catch {
      return;
    }
    if (!isNativeToExtension(message)) return;
    switch (message.type) {
      case "hello":
        retryDelay = RETRY_FIRST_MS;
        connected = true;
        render();
        postState();
        postPageState();
        placeWindow();
        break;
      case "start":
        startSession(message.mode);
        break;
      case "stop":
        stopSession();
        break;
      case "set-mode":
        mode = message.mode;
        postState();
        break;
      case "get-state":
        postState();
        break;
      case "native-config":
      case "open-options":
        break; // the settings live in the desktop app from C5 on
    }
  }

  function open(): void {
    let next: SocketLike;
    try {
      next = connect(socketUrl(href));
    } catch {
      retry();
      return;
    }
    socket = next;
    next.onopen = () => {
      if (socket !== next) return;
      const browser = describeBrowser(nav);
      post({ type: "hello", extensionVersion: PAGE_VERSION, ...(browser === "" ? {} : { browser }) });
      postPageState();
    };
    next.onmessage = (ev) => {
      if (socket === next) onDesktopMessage(ev.data);
    };
    next.onclose = () => {
      if (socket !== next) return;
      socket = null;
      connected = false;
      render();
      stopSession();
      retry();
    };
    next.onerror = () => undefined; // onclose follows
  }

  function retry(): void {
    setTimeout(open, retryDelay);
    retryDelay = Math.min(retryDelay * 2, RETRY_MAX_MS);
  }

  // ---- first run: consent and microphone ----------------------------------------------------

  function step(): PageStep {
    if (!consented) return "consent";
    return micGranted ? "ready" : "microphone";
  }

  function setText(id: string, text: string, className?: string): void {
    const el = doc?.getElementById(id);
    if (el === null || el === undefined) return;
    el.textContent = text;
    if (className !== undefined) el.className = className;
  }

  function show(id: string, visible: boolean): void {
    const el = doc?.getElementById(id);
    if (el !== null && el !== undefined) el.hidden = !visible;
  }

  function render(): void {
    const s = step();
    show("consent-step", s === "consent");
    show("microphone-step", s === "microphone");
    show("ready-step", s === "ready");
    setText("connection", connected ? "" : t("speech_disconnected"));
  }

  async function refreshMicrophone(): Promise<void> {
    try {
      micGranted = (await queryMicrophone()) === "granted";
    } catch {
      micGranted = false;
    }
  }

  async function requestMicrophone(): Promise<void> {
    if (getUserMedia === undefined) {
      setText("microphone-status", t("speech_mic_denied"), "err");
      return;
    }
    try {
      // Only to get the grant: Web Speech API opens the microphone itself.
      const stream = await getUserMedia({ audio: true });
      for (const track of stream.getTracks()) track.stop();
      micGranted = true;
      setText("microphone-status", "", "");
    } catch {
      micGranted = false;
      setText("microphone-status", t("speech_mic_denied"), "err");
    }
    render();
    postPageState();
    placeWindow();
  }

  async function consent(): Promise<void> {
    if (!consented) {
      consented = true;
      post({ type: "consent" });
    }
    render();
    postPageState();
    // The click is the user's gesture: ask for the microphone right away.
    if (!micGranted) await requestMicrophone();
    else placeWindow();
  }

  setText("title", t("speech_title"));
  setText("consent-lead", t("speech_consent_lead"));
  setText("consent-button", t("speech_consent_button"));
  setText("microphone-lead", t("speech_mic_lead"));
  setText("microphone-button", t("speech_mic_button"));
  setText("ready-lead", t("speech_ready"));
  doc?.getElementById("consent-button")?.addEventListener("click", () => void consent());
  doc?.getElementById("microphone-button")?.addEventListener("click", () => void requestMicrophone());
  render();
  void refreshMicrophone().then(() => {
    micKnown = true;
    render();
    postPageState();
    placeWindow();
  });
  open();

  return {
    offscreen,
    get step() {
      return step();
    },
    consent,
    requestMicrophone,
  };
}

if (typeof document !== "undefined" && document.getElementById("consent-step") !== null) {
  createSpeechPage({ reading: createReadingProvider() });
}
