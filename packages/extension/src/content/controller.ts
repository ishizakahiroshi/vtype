// Content-script side of voice input (plan C7b C3, rewritten for C7d): the panel's mic starts
// and stops a session, and what is heard goes into the field as it is heard.
//
// - The field and the caret are remembered at start (beginLiveInsert). Moving focus elsewhere
//   does not change where the text goes.
// - Every interim result replaces the previous one in the field; a final one stays there and
//   the next interim follows it. Stopping only ends recognition: nothing is inserted again, so
//   the text cannot land twice (C7d replaced the old "collect in the panel, insert on stop").
// - The panel's text line is now only a fallback: it shows what could not be written into the
//   field (field gone, turned into a password field, IME still composing). That leftover is
//   inserted on the next stop or send.
// - The content script never touches the speech recognition or microphone APIs: recognition
//   runs in the offscreen document, reached through the background (shared/messages.ts).

import type { Anchor } from "./anchor";
import { deepActiveElement, resolveTarget } from "./detect";
import { beginLiveInsert, insertAtCursor, type InsertResult, type LiveInsert, type LiveResult } from "./insert";
import { submitFrom } from "./submit";
import type { Panel } from "../ui/panel";
import type { WaveformActivity } from "../ui/waveform";
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
  /**
   * C7e, the `hover` setting only: start a session that no one pressed a button for (the panel
   * opened by itself on hover). Unlike `toggle` it never stops a session and never shows a
   * message: a hover is not a press, so it must not produce errors the user did not ask for.
   * It does nothing while a session runs, without a usable field, outside an extension, or
   * after the microphone was refused on this page (a later successful start allows it again).
   */
  autoStart(): void;
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
  stoppedSilence: string;
  notSubmitted: string;
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
    stoppedSilence: "Stopped after a silence. Press the mic to go on.",
    notSubmitted: "The text is in the field. vtype could not tell how this page is sent, so it sent nothing.",
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
    stoppedSilence: "無音が続いたため停止しました。マイクを押すと続けられます。",
    notSubmitted: "文字は欄に入れました。このページの送信方法が分からないため、送信はしていません。",
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
  return a + separatorBefore(a, next) + next;
}

/** "" or " ": what has to go between `previous` and `next` (no space inside Japanese text). */
export function separatorBefore(previous: string, next: string): string {
  const trimmed = next.trim();
  if (previous === "" || trimmed === "") return "";
  return /[A-Za-z0-9.,!?;:)'"]$/.test(previous) && /^[A-Za-z0-9('"]/.test(trimmed) ? " " : "";
}

/**
 * C7g: since a mic can be pressed on a field that was never clicked, the field may not have
 * the caret at all. An unfocused `input` reports `selectionStart` 0, so what is dictated would
 * be pushed in front of the text that is already there. Focus it — without scrolling the page
 * out from under the user — and put the caret after the existing text.
 *
 * Does nothing when the field already has focus: then the caret is where the user put it, and
 * C7d's "insert at the caret" is exactly what is wanted.
 */
export function focusForDictation(field: Element): void {
  const doc = field.ownerDocument;
  if (doc === null || deepActiveElement(doc) === field) return;
  try {
    if (field instanceof HTMLElement) field.focus({ preventScroll: true });
  } catch {
    return; // a field that refuses focus is still dictated into, at whatever caret it reports
  }
  const editable = field as { value?: unknown; setSelectionRange?: (a: number, b: number) => void };
  try {
    if (typeof editable.value === "string" && typeof editable.setSelectionRange === "function") {
      editable.setSelectionRange(editable.value.length, editable.value.length);
      return;
    }
    const selection = doc.defaultView?.getSelection?.();
    if (selection === null || selection === undefined) return;
    const range = doc.createRange();
    range.selectNodeContents(field);
    range.collapse(false); // after the last child: the end of what is already written
    selection.removeAllRanges();
    selection.addRange(range);
  } catch {
    // Some inputs throw on setSelectionRange, and a detached selection can refuse a range.
    // Neither is worth refusing to record over.
  }
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
  /** Text that could not be written into the field and is shown in the panel instead. */
  let confirmed = "";
  let live: LiveInsert | null = null;
  /** Whether this session managed to write anything into the field (for the end message). */
  let wroteIntoField = false;
  let submitAfterFinish = false;
  /** C7e: cleared when the microphone is refused, so a hover does not retry it every time. */
  let autoStartAllowed = true;
  let startTimer: ReturnType<typeof setTimeout> | null = null;
  let stopTimer: ReturnType<typeof setTimeout> | null = null;

  function render(): void {
    // C7d: the panel shows text only when it could not be written into the field.
    ui?.setTranscript(confirmed, "");
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

  /** End the session without the background (unreachable): keep whatever the panel holds. */
  function failLocally(text: string): void {
    live?.end();
    live = null;
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
    // C7g: give the field the caret first if it has none, or the caret reads as position 0.
    focusForDictation(target);
    // C7d: from this moment what is heard goes into the field itself, at the caret it has now.
    live = beginLiveInsert(target);
    wroteIntoField = false;
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

  function keptMessage(result: InsertResult | LiveResult): string {
    if (result.ok) return "";
    if (result.reason === "disconnected") return t.keptFieldGone;
    if (result.reason === "composition-timeout") return t.keptComposing;
    if (result.reason === "ended") return t.keptNotTarget;
    return t.keptNotTarget;
  }

  /**
   * A live write did not reach the field: keep that text in the panel instead, so nothing the
   * user dictated is lost. It is inserted on the next successful stop or send.
   */
  function keepInPanel(text: string, result: LiveResult): void {
    if (text !== "") confirmed = joinSegments(confirmed, text);
    render();
    message(keptMessage(result));
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

  function doSubmit(target: Element): void {
    const result = submitFrom(target);
    message(result.submitted ? null : t.notSubmitted);
  }

  /**
   * Send button (C8). While recognition runs it interrupts, in this order: stop recognition,
   * insert the confirmed text at the caret, submit. It does not wait for the offscreen grace
   * period (up to 1.5 s for a trailing final result): pressing send should act at once, and
   * what the panel shows is what gets inserted. Nothing is submitted when the insert fails:
   * submitting an empty field is worse than not submitting.
   */
  async function onSend(): Promise<void> {
    if (phase !== "idle") {
      submitAfterFinish = true;
      const id = sessionId;
      if (id !== null) {
        send({ target: "background", type: "stop", sessionId: id });
        sessionId = null; // the session's later events (including its `ended`) no longer apply
      }
      await finish("user");
      return;
    }
    const target = anchor.target;
    if (target === null || resolveTarget(target) !== target) {
      message(t.noField);
      return;
    }
    if (confirmed !== "") {
      const text = confirmed;
      const result = await insertAtCursor(target, text);
      if (!result.ok) {
        message(keptMessage(result));
        return;
      }
      confirmed = confirmed.startsWith(text) ? confirmed.slice(text.length).trimStart() : confirmed;
      render();
    }
    doSubmit(target);
  }

  async function finish(reason: EndReason, code?: string): Promise<void> {
    const target = field;
    // C7d: what was heard is already in the field. Only text that could not be written there
    // is still held in the panel, and that is what may have to be inserted now.
    live?.end();
    live = null;
    const wroteAnything = wroteIntoField;
    wroteIntoField = false;
    const text = confirmed;
    field = null;
    toIdle();
    render();

    const wantSubmit = submitAfterFinish;
    submitAfterFinish = false;

    if (reason === "user") {
      if (text === "" || target === null) {
        // Nothing to insert: send still submits what is already in the field.
        const fallback = target ?? anchor.target;
        if (wantSubmit && fallback !== null) doSubmit(fallback);
        else if (wantSubmit) message(t.noField);
        return;
      }
      // The text stays visible while insertAtCursor may wait for an IME composition to end.
      const result = await insertAtCursor(target, text);
      if (result.ok) {
        // A recording started meanwhile appends after `text`: drop only what was inserted.
        confirmed = confirmed.startsWith(text) ? confirmed.slice(text.length).trimStart() : confirmed;
        render();
        if (phase === "idle") message(null);
        if (wantSubmit) doSubmit(target);
      } else {
        // No submit: the text is not in the field, and an empty submit cannot be taken back.
        message(keptMessage(result));
      }
      return;
    }
    if (reason === "silence") {
      if (text !== "") message(t.keptSilence);
      else message(wroteAnything ? t.stoppedSilence : t.noSpeech);
    } else if (reason === "superseded") {
      message(t.superseded);
    } else if (reason === "aborted") {
      message(t.aborted);
    } else {
      const e = errorText(code);
      // C7e: a refused microphone would fail again on the next hover, so stop trying by
      // itself. The panel's mic still starts (it is what the "Allow it" action leads back to).
      if (e.permission) autoStartAllowed = false;
      message(e.text, e.permission);
    }
  }

  /**
   * C7d: a result goes into the field right away. A final segment stays (the next interim
   * follows it); an interim replaces the previous one. What cannot be written is kept in the
   * panel instead.
   */
  async function writeLive(transcript: string, isFinal: boolean): Promise<void> {
    const session = live;
    if (session === null) return;
    const body = transcript.trim();
    if (isFinal) {
      if (body === "") {
        // Chrome sends empty finals during silence: just drop the interim shown so far.
        await session.update("");
        return;
      }
      // Text the panel is still holding (an earlier write failed) goes in ahead of this
      // segment, so the order stays the order it was spoken in.
      const carried = confirmed;
      const segment = carried === "" ? body : joinSegments(carried, body);
      const result = await session.commit(separatorBefore(session.committedText, segment) + segment);
      if (!result.ok) {
        keepInPanel(body, result);
        return;
      }
      if (carried !== "" && confirmed.startsWith(carried)) {
        confirmed = confirmed.slice(carried.length).trimStart();
        render();
        message(null);
      }
      wroteIntoField = true;
      return;
    }
    if (body === "") return;
    const result = await session.update(separatorBefore(session.committedText, body) + body);
    if (!result.ok && result.reason !== "ended") keepInPanel("", result);
  }

  function handle(event: SessionEvent): void {
    if (event.kind === "started") {
      if (startTimer !== null) clearTimeout(startTimer);
      startTimer = null;
      // Recognition really started, so the microphone is allowed: a refusal that switched
      // auto-start off earlier (the user has granted it since) no longer applies.
      autoStartAllowed = true;
      return;
    }
    if (event.kind === "activity") {
      // C7c: the waveform is driven by these events, never by microphone volume.
      ui?.setActivity(event.activity as WaveformActivity);
      return;
    }
    if (event.kind === "result") {
      // Late results of a replaced instance (isCurrent: false) are kept on purpose.
      ui?.waveform.noteTranscript(event.transcript, event.isFinal);
      void writeLive(event.transcript, event.isFinal);
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

  function autoStart(): void {
    if (phase !== "idle" || !autoStartAllowed || runtime === null) return;
    const target = anchor.target;
    if (target === null || resolveTarget(target) !== target) return;
    start();
  }

  function sendHandler(): void {
    void onSend();
  }

  /** × emptied the field: start writing again from the beginning of the empty field (C7d 6). */
  function clearHandler(): void {
    live?.resync();
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
      panel.onSend = sendHandler;
      panel.onClear = clearHandler;
      render();
    },
    toggle,
    autoStart,
    dispose(): void {
      if (sessionId !== null) send({ target: "background", type: "stop", sessionId });
      clearTimers();
      runtime?.onMessage.removeListener(onMessage);
      if (ui !== null && ui.onMic === toggle) ui.onMic = null;
      if (ui !== null && ui.onSend === sendHandler) ui.onSend = null;
      if (ui !== null && ui.onClear === clearHandler) ui.onClear = null;
      live?.end();
      live = null;
    },
  };
}
