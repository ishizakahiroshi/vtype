// Content-script side of voice input (plan C7b C3): the panel's mic starts and stops a
// session, results are shown in the panel only, and on stop the text goes into the field that
// had focus when recording started.
//
// - The field is remembered at start. Moving focus elsewhere does not change where text goes.
// - Interim and final results are shown in the panel (setTranscript) and never written to the
//   field while recording ("no auto-confirm on silence" is the decided spec).
// - On the user's stop the confirmed text, plus an interim tail Chrome did not finalise, is
//   inserted once at the caret through C6 insertAtCursor (which waits for IME composition).
// - When the text cannot be inserted (field gone, turned into a password field, IME timeout)
//   or the session ended some other way (silence, error, superseded), the text stays in the
//   panel. The next recording continues after it, so it goes into the field on the next stop.
// - The content script never touches the speech recognition or microphone APIs: recognition
//   runs in the offscreen document, reached through the background (shared/messages.ts).

import type { Anchor } from "./anchor";
import { resolveTarget } from "./detect";
import { insertAtCursor, type InsertResult } from "./insert";
import type { Panel } from "../ui/panel";
import {
  isBackgroundToContent,
  type ContentToBackground,
  type EndReason,
  type SessionEvent,
} from "../shared/messages";

/** Give up waiting for `ended` after a stop (background or offscreen gone) and finish locally. */
export const STOP_TIMEOUT_MS = 5000;
/** Give up waiting for `started` (the offscreen document never answered). */
export const START_TIMEOUT_MS = 10_000;

export interface ContentRuntime {
  sendMessage(message: unknown): Promise<unknown> | void;
  onMessage: {
    addListener(listener: (message: unknown) => void): void;
    removeListener(listener: (message: unknown) => void): void;
  };
}

export interface ControllerOptions {
  anchor: Anchor;
  /** chrome.runtime; null outside an extension (then the mic only shows a message). */
  runtime: ContentRuntime | null;
  language?: string;
}

export type ControllerPhase = "idle" | "recording" | "stopping";

export interface Controller {
  readonly phase: ControllerPhase;
  readonly sessionId: string | null;
  /** Field chosen at start (null when idle). */
  readonly field: Element | null;
  /** Text kept in the panel (confirmed, not inserted yet). */
  readonly pendingText: string;
  /** Hook the panel's buttons up. Safe to call repeatedly with the same panel. */
  wire(ui: Panel): void;
  /** What the panel's mic does. */
  toggle(): void;
  dispose(): void;
}

interface Texts {
  noField: string;
  unavailable: string;
  didNotStart: string;
  notAllowed: string;
  openPermission: string;
  noSpeech: string;
  network: string;
  audioCapture: string;
  superseded: string;
  aborted: string;
  unsupported: string;
  otherError: (code: string) => string;
  keptFieldGone: string;
  keptNotTarget: string;
  keptComposing: string;
  keptSilence: string;
}

const TEXTS: Record<"en" | "ja", Texts> = {
  en: {
    noField: "Click a text field first.",
    unavailable: "vtype is not available on this page. Reload the page and try again.",
    didNotStart: "Voice input did not start. Try again.",
    notAllowed: "The microphone is not allowed for vtype.",
    openPermission: "Allow it",
    noSpeech: "Nothing was heard.",
    network: "Could not reach the speech recognition service.",
    audioCapture: "No usable microphone was found.",
    superseded: "Stopped: voice input started somewhere else.",
    aborted: "Voice input was stopped.",
    unsupported: "This browser cannot do speech recognition.",
    otherError: (code) => `Speech recognition failed (${code}).`,
    keptFieldGone: "The text field is gone, so the text was kept here.",
    keptNotTarget: "That field cannot take the text, so it was kept here.",
    keptComposing: "Text entry was busy (IME), so the text was kept here.",
    keptSilence: "Stopped after silence. Press the mic to continue; the text is kept.",
  },
  ja: {
    noField: "先に入力欄をクリックしてください。",
    unavailable: "このページでは vtype を使えません。ページを再読み込みしてください。",
    didNotStart: "音声入力を開始できませんでした。もう一度押してください。",
    notAllowed: "vtype にマイクが許可されていません。",
    openPermission: "許可する",
    noSpeech: "聞き取れませんでした。",
    network: "音声認識サービスに接続できませんでした。",
    audioCapture: "使えるマイクが見つかりません。",
    superseded: "別の場所で音声入力が始まったため停止しました。",
    aborted: "音声入力を停止しました。",
    unsupported: "このブラウザでは音声認識を使えません。",
    otherError: (code) => `音声認識でエラーが発生しました（${code}）。`,
    keptFieldGone: "入力欄が見つからないため、文字をここに残しました。",
    keptNotTarget: "この欄には入れられないため、文字をここに残しました。",
    keptComposing: "変換中のため入れられませんでした。文字をここに残しました。",
    keptSilence: "無音が続いたため停止しました。マイクを押すと続けられます（文字は残っています）。",
  },
};

export function textsFor(language: string | undefined): Texts {
  return language?.toLowerCase().startsWith("ja") ? TEXTS.ja : TEXTS.en;
}

const PERMISSION_CODES: ReadonlySet<string> = new Set(["not-allowed", "service-not-allowed", "permission_denied"]);

/**
 * Join recognition segments. Chrome's segments carry no separator; Japanese needs none, while
 * two Latin words need a space.
 */
export function joinSegments(a: string, b: string): string {
  const next = b.trim();
  if (next === "") return a;
  if (a === "") return next;
  const needsSpace = /[A-Za-z0-9.,!?;:)'"]$/.test(a) && /^[A-Za-z0-9('"]/.test(next);
  return a + (needsSpace ? " " : "") + next;
}

let sessionCounter = 0;
function newSessionId(): string {
  sessionCounter += 1;
  const random = Math.random().toString(36).slice(2, 10);
  return `s${Date.now().toString(36)}-${sessionCounter}-${random}`;
}

export function createController(options: ControllerOptions): Controller {
  const { anchor, runtime } = options;
  const t = textsFor(options.language ?? globalThis.navigator?.language);

  let ui: Panel | null = null;
  let phase: ControllerPhase = "idle";
  let sessionId: string | null = null;
  let field: Element | null = null;
  let confirmed = "";
  let interim = "";
  let startTimer: ReturnType<typeof setTimeout> | null = null;
  let stopTimer: ReturnType<typeof setTimeout> | null = null;

  function render(): void {
    ui?.setTranscript(confirmed, interim);
  }

  function message(text: string | null, withPermissionAction = false): void {
    if (ui === null) return;
    if (withPermissionAction) {
      ui.setMessage(text, { label: t.openPermission, run: openPermissionPage });
    } else {
      ui.setMessage(text);
    }
  }

  function send(msg: ContentToBackground): boolean {
    if (runtime === null) return false;
    try {
      const result = runtime.sendMessage(msg);
      if (result !== undefined && typeof (result as Promise<unknown>).catch === "function") {
        (result as Promise<unknown>).catch(() => undeliverable(msg));
      }
      return true;
    } catch {
      undeliverable(msg); // "Extension context invalidated" after the extension was reloaded
      return true;
    }
  }

  /** The background could not be reached for `msg`. */
  function undeliverable(msg: ContentToBackground): void {
    if (msg.type === "open-permission" || sessionId !== msg.sessionId) return;
    // A failed start keeps whatever is in the panel; a failed stop still inserts what the
    // user dictated, as a stop would have.
    if (msg.type === "start") failLocally(t.unavailable);
    else void finish("user");
  }

  function openPermissionPage(): void {
    send({ target: "background", type: "open-permission" });
  }

  function clearTimers(): void {
    if (startTimer !== null) clearTimeout(startTimer);
    if (stopTimer !== null) clearTimeout(stopTimer);
    startTimer = null;
    stopTimer = null;
  }

  function toIdle(): void {
    clearTimers();
    phase = "idle";
    sessionId = null;
    ui?.setState("idle");
  }

  /** End the session without the background (unreachable): keep the text. */
  function failLocally(text: string): void {
    confirmed = joinSegments(confirmed, interim);
    interim = "";
    field = null;
    toIdle();
    render();
    message(text);
  }

  function start(): void {
    const target = anchor.target;
    if (target === null || resolveTarget(target) !== target) {
      message(t.noField);
      return;
    }
    if (runtime === null) {
      message(t.unavailable);
      return;
    }
    const id = newSessionId();
    sessionId = id;
    field = target;
    phase = "recording";
    interim = "";
    ui?.setState("recording");
    message(null);
    render();
    send({ target: "background", type: "start", sessionId: id });
    if (sessionId !== id) return; // failed synchronously
    startTimer = setTimeout(() => {
      if (sessionId === id && phase === "recording") failLocally(t.didNotStart);
    }, START_TIMEOUT_MS);
  }

  function stop(): void {
    const id = sessionId;
    if (id === null) return;
    phase = "stopping";
    ui?.setState("processing");
    send({ target: "background", type: "stop", sessionId: id });
    if (sessionId !== id) return; // failed synchronously and already finished
    stopTimer = setTimeout(() => {
      if (sessionId === id) void finish("user");
    }, STOP_TIMEOUT_MS);
  }

  function keptMessage(result: InsertResult): string {
    if (result.ok) return "";
    if (result.reason === "disconnected") return t.keptFieldGone;
    if (result.reason === "composition-timeout") return t.keptComposing;
    return t.keptNotTarget;
  }

  function errorText(code: string | undefined): { text: string; permission: boolean } {
    if (code !== undefined && PERMISSION_CODES.has(code)) return { text: t.notAllowed, permission: true };
    if (code === "no-speech") return { text: t.noSpeech, permission: false };
    if (code === "network") return { text: t.network, permission: false };
    if (code === "audio-capture" || code === "audio_capture") return { text: t.audioCapture, permission: false };
    if (code === "aborted") return { text: t.superseded, permission: false };
    if (code === "unsupported") return { text: t.unsupported, permission: false };
    return { text: t.otherError(code ?? "unknown"), permission: false };
  }

  async function finish(reason: EndReason, code?: string): Promise<void> {
    const target = field;
    const text = joinSegments(confirmed, interim); // an interim tail Chrome never finalised is kept
    confirmed = text;
    interim = "";
    field = null;
    toIdle();
    render();

    if (reason === "user") {
      if (text === "" || target === null) return;
      // The text stays visible while insertAtCursor may wait for an IME composition to end.
      const result = await insertAtCursor(target, text);
      if (result.ok) {
        // A recording started meanwhile appends after `text`: drop only what was inserted.
        confirmed = confirmed.startsWith(text) ? confirmed.slice(text.length).trimStart() : confirmed;
        render();
        if (phase === "idle") message(null);
      } else {
        message(keptMessage(result));
      }
      return;
    }
    if (reason === "silence") {
      message(text === "" ? t.noSpeech : t.keptSilence);
    } else if (reason === "superseded") {
      message(t.superseded);
    } else if (reason === "aborted") {
      message(t.aborted);
    } else {
      const e = errorText(code);
      message(e.text, e.permission);
    }
  }

  function handle(event: SessionEvent): void {
    if (event.kind === "started") {
      if (startTimer !== null) clearTimeout(startTimer);
      startTimer = null;
      return;
    }
    if (event.kind === "result") {
      // Late results of a replaced instance (isCurrent: false) are kept on purpose.
      if (event.isFinal) {
        confirmed = joinSegments(confirmed, event.transcript);
        interim = "";
      } else {
        interim = event.transcript;
      }
      render();
      return;
    }
    void finish(event.reason, event.code);
  }

  function onMessage(msg: unknown): void {
    if (!isBackgroundToContent(msg)) return;
    if (sessionId === null || msg.sessionId !== sessionId) return; // stale or another frame's
    handle(msg.event);
  }

  runtime?.onMessage.addListener(onMessage);

  function toggle(): void {
    if (phase === "idle") start();
    else if (phase === "recording") stop();
  }

  return {
    get phase() {
      return phase;
    },
    get sessionId() {
      return sessionId;
    },
    get field() {
      return field;
    },
    get pendingText() {
      return confirmed;
    },
    wire(panel: Panel): void {
      if (ui === panel) return;
      ui = panel;
      panel.onMic = toggle;
      render();
    },
    toggle,
    dispose(): void {
      if (sessionId !== null) send({ target: "background", type: "stop", sessionId });
      clearTimers();
      runtime?.onMessage.removeListener(onMessage);
      if (ui !== null && ui.onMic === toggle) ui.onMic = null;
    },
  };
}
