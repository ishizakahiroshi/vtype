// Offscreen document: the only place that runs speech recognition (plan C7b C1).
//
// Uses vtype-core's browser engine (createSpeechRecognizer; no Whisper in v1). The recognition
// language is this document's navigator.language.
//
// Session rules
// - One session at a time. A start for a new session first ends the old one with
//   `superseded` (reported to the old owner), then aborts its recognition.
// - Recording lasts until the user presses the mic again. vtype-core runs Chrome's recognizer
//   with continuous = false (not configurable), so Chrome ends each recognition after one
//   utterance or ~8 s of silence. While the session is open, every such end starts the next
//   recognition cycle. After SILENT_CYCLE_LIMIT cycles in a row without any text the session
//   ends with `silence` (a forgotten recording must not keep the microphone forever).
// - Fatal errors (not-allowed, network, audio-capture, aborted, ...) end the session with
//   `error` and the code. `no-speech` is a silent cycle, not an error.
// - On the user's stop, a pending interim result may still become final: Chrome delivers it
//   from the previous instance after stop() (vtype-core contract note 1, isCurrent: false).
//   We wait up to STOP_GRACE_MS for that final result before reporting `ended: user`.
// - Everything is keyed on recognitionId (contract note 2): results from instances older than
//   the session's first one are dropped, and a late error of an old instance that ends the
//   current recording is treated like a normal end (the cycle restarts).

//
// Input modes (native plan C2): each session carries a mode and a replacement table from the
// background. The mode picks the recognition language (recognitionLangFor), and every result is
// shaped before it is sent: interim results synchronously, final ones through the optional
// ReadingProvider (kanji -> katakana), which is asynchronous. Every event of a session is sent
// through one promise chain, so a final that waits for the dictionary is never overtaken by the
// `ended` that follows it.

import {
  createSpeechRecognizer,
  recognitionLangFor,
  transformTranscript,
  transformTranscriptSync,
  type InputMode,
  type ReadingProvider,
  type ReplacementRule,
  type SpeechRecognizer,
  type SpeechRecognizerError,
} from "vtype-core";
import {
  isBackgroundToOffscreen,
  type EndReason,
  type OffscreenToBackground,
  type Owner,
  type SessionEvent,
} from "../shared/messages";
import { createReadingProvider } from "./reading";

export const SILENT_CYCLE_LIMIT = 3;
export const STOP_GRACE_MS = 1500;

/** Recognizer errors that only mean "nothing was heard" and keep the session going. */
const SILENT_ERRORS: ReadonlySet<string> = new Set(["no-speech"]);

interface OffscreenChrome {
  runtime: {
    sendMessage(message: unknown): Promise<unknown>;
    onMessage: { addListener(listener: (message: unknown) => void): void };
  };
}

interface Session {
  readonly id: string;
  readonly owner: Owner;
  readonly mode: InputMode;
  readonly rules: readonly ReplacementRule[];
  /** Every message of this session goes out through this chain, in order. */
  outbox: Promise<void>;
  /** Results from recognition instances before this one belong to an earlier session. */
  firstRecognitionId: number;
  currentRecognitionId: number;
  started: boolean;
  /** A recognition cycle was started and has not ended yet. */
  cycleActive: boolean;
  userStopping: boolean;
  silentCycles: number;
  heardTextThisCycle: boolean;
  lastError: SpeechRecognizerError | null;
  /** recognitionId of an interim result not yet followed by its final, or null. */
  pendingInterimId: number | null;
  graceTimer: ReturnType<typeof setTimeout> | null;
}

export interface Offscreen {
  readonly recognizer: SpeechRecognizer;
  /** The open session's id, or null. */
  readonly sessionId: string | null;
}

export interface OffscreenOptions {
  chrome: OffscreenChrome;
  /** A ready recognizer (its language is then its own), or a factory given the mode-aware language. */
  recognizer?: SpeechRecognizer;
  createRecognizer?: (lang: () => string) => SpeechRecognizer;
  /** Normal mode's language. Default: this document's navigator.language. */
  baseLang?: () => string;
  /** Kanji -> katakana reading for kana mode. Without it kana mode converts kana only. */
  reading?: ReadingProvider;
}

export function createOffscreen(options: OffscreenOptions): Offscreen {
  const chrome = options.chrome;
  const baseLang = options.baseLang ?? (() => globalThis.navigator?.language || "en-US");
  let session: Session | null = null;
  const lang = (): string => recognitionLangFor(session?.mode ?? "normal", baseLang());
  const recognizer =
    options.recognizer ?? (options.createRecognizer ?? ((l) => createSpeechRecognizer({ lang: l })))(lang);
  const reading = options.reading;

  function send(s: Session, event: SessionEvent): void {
    const message: OffscreenToBackground = {
      target: "background",
      type: "session-event",
      sessionId: s.id,
      owner: s.owner,
      event,
    };
    void chrome.runtime.sendMessage(message).catch(() => undefined);
  }

  function post(s: Session, event: SessionEvent): void {
    s.outbox = s.outbox.then(() => send(s, event));
  }

  /** A result, shaped for the session's mode. The final one may wait for the dictionary. */
  function postResult(s: Session, event: Extract<SessionEvent, { kind: "result" }>): void {
    const options = { mode: s.mode, rules: s.rules };
    if (!event.isFinal || reading === undefined || s.mode !== "kana") {
      post(s, { ...event, transcript: transformTranscriptSync(event.transcript, options) });
      return;
    }
    s.outbox = s.outbox.then(async () => {
      let transcript: string;
      try {
        transcript = await transformTranscript(event.transcript, { ...options, reading });
      } catch {
        // The dictionary failed to load: kana-only conversion is still better than nothing.
        transcript = transformTranscriptSync(event.transcript, options);
      }
      send(s, { ...event, transcript });
    });
  }

  function end(s: Session, reason: EndReason, code?: string): void {
    if (s.graceTimer !== null) clearTimeout(s.graceTimer);
    s.graceTimer = null;
    if (session === s) session = null;
    post(s, code === undefined ? { kind: "ended", reason } : { kind: "ended", reason, code });
  }

  function startCycle(s: Session): void {
    if (session !== s || s.userStopping) return;
    s.currentRecognitionId = recognizer.getRecognitionId();
    s.heardTextThisCycle = false;
    s.lastError = null;
    const outcome = recognizer.start();
    if (!outcome.started) {
      end(s, "error", outcome.error ?? "start-failed");
      return;
    }
    s.cycleActive = true;
  }

  recognizer.on("start", ({ recognitionId }) => {
    const s = session;
    if (s === null || recognitionId < s.firstRecognitionId) return;
    if (!s.started) {
      s.started = true;
      post(s, { kind: "started", recognitionId });
    }
  });

  recognizer.on("activity", ({ recognitionId, kind }) => {
    const s = session;
    if (s === null || recognitionId < s.firstRecognitionId) return;
    post(s, { kind: "activity", recognitionId, activity: kind });
  });

  recognizer.on("result", (r) => {
    const s = session;
    if (s === null || r.recognitionId < s.firstRecognitionId) return;
    if (r.isFinal) {
      if (r.transcript.trim() !== "") {
        s.heardTextThisCycle = true;
        s.silentCycles = 0;
      }
      if (s.pendingInterimId === r.recognitionId) s.pendingInterimId = null;
    } else if (r.transcript !== "") {
      s.pendingInterimId = r.recognitionId;
    }
    postResult(s, {
      kind: "result",
      recognitionId: r.recognitionId,
      isCurrent: r.isCurrent,
      transcript: r.transcript,
      isFinal: r.isFinal,
    });
    if (s.userStopping && s.pendingInterimId === null) end(s, "user");
  });

  recognizer.on("error", (e) => {
    const s = session;
    if (s !== null) s.lastError = e;
  });

  /** One recognition cycle is over: end the session or start the next cycle. */
  function cycleEnded(s: Session, endedId: number): void {
    s.cycleActive = false;
    const err = s.lastError;
    s.lastError = null;
    // An error counts only if it came from the instance that just ended. A late error of an
    // older instance that happened to end this recording is treated as a normal end.
    if (err !== null && err.recognitionId === endedId && !SILENT_ERRORS.has(err.code)) {
      end(s, "error", err.code);
      return;
    }
    if (!s.heardTextThisCycle) s.silentCycles += 1;
    if (s.silentCycles >= SILENT_CYCLE_LIMIT) {
      end(s, "silence");
      return;
    }
    // vtype-core replaces the instance right after emitting `stop`; start the next cycle on
    // the new instance.
    setTimeout(() => startCycle(s), 0);
  }

  recognizer.on("stop", ({ recognitionId }) => {
    const s = session;
    if (s === null || s.userStopping || !s.cycleActive) return;
    cycleEnded(s, recognitionId);
  });

  // vtype-core emits `stop` only for a recording that had started. An error or end that
  // arrives before the native `start` (for example `not-allowed` about 1 ms after start() in
  // an offscreen document without a microphone grant) emits `error` and then only `state`.
  recognizer.on("state", (state) => {
    const s = session;
    if (s === null || s.userStopping || !s.cycleActive) return;
    if (state.starting || state.recording) return;
    cycleEnded(s, s.currentRecognitionId);
  });

  function onStart(sessionId: string, owner: Owner, mode: InputMode, rules: readonly ReplacementRule[]): void {
    const old = session;
    if (old !== null) {
      if (old.id === sessionId) return; // duplicate start
      end(old, "superseded");
      recognizer.abort();
    }
    const s: Session = {
      id: sessionId,
      owner,
      mode,
      rules,
      outbox: Promise.resolve(),
      firstRecognitionId: recognizer.getRecognitionId(),
      currentRecognitionId: recognizer.getRecognitionId(),
      started: false,
      cycleActive: false,
      userStopping: false,
      silentCycles: 0,
      heardTextThisCycle: false,
      lastError: null,
      pendingInterimId: null,
      graceTimer: null,
    };
    session = s;
    // abort() above replaced the instance synchronously; start on the next turn like a cycle.
    setTimeout(() => {
      s.firstRecognitionId = recognizer.getRecognitionId();
      startCycle(s);
    }, 0);
  }

  function onStop(sessionId: string, owner: Owner | null): void {
    const s = session;
    if (s === null || s.id !== sessionId) {
      // Unknown session (already ended, or this document was recreated): let the requester
      // finish with what it has.
      if (owner !== null) {
        void chrome.runtime
          .sendMessage({
            target: "background",
            type: "session-event",
            sessionId,
            owner,
            event: { kind: "ended", reason: "user" },
          } satisfies OffscreenToBackground)
          .catch(() => undefined);
      }
      return;
    }
    if (s.userStopping) return;
    s.userStopping = true;
    recognizer.stop();
    if (s.pendingInterimId === null) {
      end(s, "user");
      return;
    }
    s.graceTimer = setTimeout(() => end(s, "user"), STOP_GRACE_MS);
  }

  function onAbort(sessionId: string | undefined, tabId: number | undefined): void {
    const s = session;
    if (s === null) return;
    if (sessionId !== undefined && s.id !== sessionId) return;
    if (sessionId === undefined && (tabId === undefined || s.owner.kind !== "tab" || s.owner.tabId !== tabId)) return;
    end(s, "aborted");
    recognizer.abort();
  }

  // Owners of sessions this document no longer knows, so a late stop can still be answered.
  const owners = new Map<string, Owner>();

  chrome.runtime.onMessage.addListener((message) => {
    if (!isBackgroundToOffscreen(message)) return;
    if (message.type === "start") {
      owners.set(message.sessionId, message.owner);
      if (owners.size > 32) owners.delete(owners.keys().next().value as string);
      onStart(message.sessionId, message.owner, message.mode, message.rules);
    } else if (message.type === "stop") {
      onStop(message.sessionId, owners.get(message.sessionId) ?? null);
    } else {
      onAbort(message.sessionId, message.tabId);
    }
  });

  return {
    recognizer,
    get sessionId() {
      return session?.id ?? null;
    },
  };
}

const extensionChrome = (globalThis as { chrome?: OffscreenChrome & { runtime: { id?: string } } }).chrome;
if (extensionChrome?.runtime?.id !== undefined) {
  createOffscreen({ chrome: extensionChrome, reading: createReadingProvider() });
}
