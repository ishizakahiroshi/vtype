// vtype-core: speech recognition engine (Web Speech API).
//
// Ported from many-ai-cli `web/src/app/voice.ts`. Only the recognition side lives here.
// This module renders nothing: it never creates or looks up DOM nodes, never touches class
// names, i18n, storage or the network. Everything the original did to the page is surfaced
// as events and return values so the caller keeps its own buttons, bars and toasts.
//
// It must also run inside a Chrome extension offscreen document (there is a `window`, but no
// app DOM), so host globals are only read through the typed `host()` view below, and the
// SpeechRecognition constructor can be injected.

// ---------------------------------------------------------------------------
// Web Speech API shapes (TypeScript's lib.dom does not ship SpeechRecognition)
// ---------------------------------------------------------------------------

export interface SpeechRecognitionAlternativeLike {
  readonly transcript: string;
  readonly confidence?: number;
}

export interface SpeechRecognitionResultLike {
  readonly isFinal: boolean;
  readonly length: number;
  readonly [index: number]: SpeechRecognitionAlternativeLike;
}

export interface SpeechRecognitionResultListLike {
  readonly length: number;
  readonly [index: number]: SpeechRecognitionResultLike;
}

export interface SpeechRecognitionEventLike {
  readonly resultIndex: number;
  readonly results: SpeechRecognitionResultListLike;
  readonly currentTarget?: unknown;
}

export interface SpeechRecognitionErrorEventLike {
  readonly error?: string;
  readonly message?: string;
  readonly currentTarget?: unknown;
}

export interface SpeechRecognitionLike {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  maxAlternatives: number;
  start(): void;
  stop(): void;
  abort(): void;
  onstart: ((ev: unknown) => void) | null;
  onaudiostart: ((ev: unknown) => void) | null;
  onsoundstart: ((ev: unknown) => void) | null;
  onspeechstart: ((ev: unknown) => void) | null;
  onspeechend: ((ev: unknown) => void) | null;
  onsoundend: ((ev: unknown) => void) | null;
  onaudioend: ((ev: unknown) => void) | null;
  onresult: ((ev: SpeechRecognitionEventLike) => void) | null;
  onnomatch: ((ev: unknown) => void) | null;
  onerror: ((ev: SpeechRecognitionErrorEventLike) => void) | null;
  onend: ((ev: unknown) => void) | null;
  addEventListener(type: string, listener: (ev: never) => void): void;
  removeEventListener(type: string, listener: (ev: never) => void): void;
}

export type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

export interface NavigatorUADataLike {
  readonly brands?: ReadonlyArray<{ readonly brand: string; readonly version?: string }>;
  readonly mobile?: boolean;
  readonly platform?: string;
}

export interface NavigatorLike {
  readonly userAgent?: string;
  readonly userAgentData?: NavigatorUADataLike;
}

// ---------------------------------------------------------------------------
// Host globals (window in a page, window in an offscreen document)
// ---------------------------------------------------------------------------

interface HostGlobals {
  readonly SpeechRecognition?: SpeechRecognitionConstructor;
  readonly webkitSpeechRecognition?: SpeechRecognitionConstructor;
  readonly navigator?: NavigatorLike;
  readonly location?: { readonly origin?: string };
  readonly console?: {
    warn?(...args: unknown[]): void;
    error?(...args: unknown[]): void;
  };
  setTimeout(handler: () => void, ms?: number): unknown;
  clearTimeout(handle: unknown): void;
}

function host(): HostGlobals {
  return globalThis as unknown as HostGlobals;
}

type TimerHandle = unknown;

function setTimer(fn: () => void, ms: number): TimerHandle {
  return host().setTimeout(fn, ms);
}

// Original code calls clearTimeout(null) freely; that is a no-op, so skip the call instead.
function clearTimer(handle: TimerHandle): void {
  if (handle != null) host().clearTimeout(handle);
}

// ---------------------------------------------------------------------------
// Constants (values unchanged from voice.ts)
// ---------------------------------------------------------------------------

/** Max entries kept in the diagnostic event log; older ones are dropped (voice.ts VOICE_DIAG_EVENT_LIMIT). */
export const VOICE_DIAG_EVENT_LIMIT = 80;
/** audioend without result/end/error for this long means "stuck" (voice.ts VOICE_DIAG_STUCK_MS). */
export const VOICE_DIAG_STUCK_MS = 20000;
/** Extra time on top of the stuck timeout before the diagnostic run gives up. */
export const VOICE_DIAG_HARD_TIMEOUT_EXTRA_MS = 10000;
/** Hotword: delay before re-listening after its own `end`. */
export const HOTWORD_RESTART_DELAY_MS = 250;
/** Hotword: delay before re-arming after the main voice input stopped. */
export const HOTWORD_REARM_DELAY_MS = 300;
/** Hotword: grace after `end` for Chrome to release the mic capture (empirical). */
export const HOTWORD_MIC_RELEASE_WAIT_MS = 50;
/** Hotword: resolve stopForVoiceInput() even if `end` never arrives. */
export const HOTWORD_STOP_SAFETY_MS = 500;

/** Hotword errors after which listening must be turned off by the host. */
export const HOTWORD_FATAL_ERRORS: readonly string[] = [
  'not-allowed',
  'permission-denied',
  'audio-capture',
  'network',
  'service-not-allowed',
  'language-not-supported',
];

// ---------------------------------------------------------------------------
// Support detection (voice.ts lines 16-18, 34, 161)
// ---------------------------------------------------------------------------

export interface RecognitionSupport {
  /** SpeechRecognition exists and the browser is Chromium. Otherwise the engine is `unsupported`. */
  readonly supported: boolean;
  readonly speechRecognitionSupported: boolean;
  readonly chromiumDetected: boolean;
}

export interface SupportOptions {
  /**
   * Constructor to use instead of `window.SpeechRecognition || window.webkitSpeechRecognition`.
   * `null` means "treat as missing". Omit to read the host global.
   */
  SpeechRecognition?: SpeechRecognitionConstructor | null;
  /** Navigator to inspect instead of the host `navigator`. */
  navigator?: NavigatorLike;
  /** Override the Chromium check (for tests or hosts that already know). */
  isChromium?: boolean;
}

/** `window.SpeechRecognition || window.webkitSpeechRecognition`, or null. */
export function getSpeechRecognitionConstructor(): SpeechRecognitionConstructor | null {
  const g = host();
  return g.SpeechRecognition || g.webkitSpeechRecognition || null;
}

/**
 * Same test as voice.ts: userAgentData brands contain "Chromium"; only when brands are
 * unavailable, fall back to `Chrome/` in the user agent string.
 */
export function isChromiumBrowser(nav: NavigatorLike | undefined = host().navigator): boolean {
  const byBrands = nav?.userAgentData?.brands?.some((b) => /Chromium/.test(b.brand));
  return byBrands ?? /Chrome\//.test(nav?.userAgent ?? '');
}

interface ResolvedSupport extends RecognitionSupport {
  readonly ctor: SpeechRecognitionConstructor | null;
  readonly navigator: NavigatorLike | undefined;
}

function resolveSupport(options: SupportOptions): ResolvedSupport {
  const ctor = options.SpeechRecognition === undefined
    ? getSpeechRecognitionConstructor()
    : options.SpeechRecognition;
  const nav = options.navigator ?? host().navigator;
  const chromiumDetected = options.isChromium ?? isChromiumBrowser(nav);
  return {
    ctor,
    navigator: nav,
    speechRecognitionSupported: !!ctor,
    chromiumDetected,
    supported: !!ctor && chromiumDetected,
  };
}

/** Whether browser speech recognition can be used here (voice.ts `SpeechRecognition && isChromium`). */
export function detectSupport(options: SupportOptions = {}): RecognitionSupport {
  const { supported, speechRecognitionSupported, chromiumDetected } = resolveSupport(options);
  return { supported, speechRecognitionSupported, chromiumDetected };
}

// ---------------------------------------------------------------------------
// Error helpers (voice.ts normalizeVoiceErrorCode / classifyVoiceError / showVoiceError)
// ---------------------------------------------------------------------------

/** Error code from a SpeechRecognitionErrorEvent, a DOMException, an Error or a string. */
export function normalizeVoiceErrorCode(error: unknown): string {
  let raw: unknown = '';
  if (typeof error === 'string') {
    raw = error;
  } else if (error && typeof error === 'object') {
    const e = error as { error?: unknown; name?: unknown; message?: unknown };
    raw = e.error || e.name || e.message || '';
  }
  return String(raw || 'unknown').trim() || 'unknown';
}

export type DiagnosticStatus =
  | 'idle'
  | 'unsupported'
  | 'running'
  | 'healthy'
  | 'permission_denied'
  | 'audio_capture_failed'
  | 'speech_service_failed'
  | 'no_result'
  | 'profile_or_stt_stuck_suspected'
  | 'normal_profile_specific';

/** Map an error code to the diagnostic status it implies (voice.ts classifyVoiceError). */
export function classifyVoiceError(error: string): DiagnosticStatus {
  if (error === 'not-allowed' || error === 'permission-denied') return 'permission_denied';
  if (error === 'audio-capture') return 'audio_capture_failed';
  if (error === 'network' || error === 'service-not-allowed') return 'speech_service_failed';
  if (error === 'no-speech') return 'no_result';
  return 'speech_service_failed';
}

export type VoiceErrorKind = 'permission' | 'audio_capture' | 'network' | 'service' | 'language' | 'other';

/**
 * Which message the host should show for an error (the branches of voice.ts showVoiceError).
 * The host owns the wording; `other` corresponds to the generic "error: {code}" message.
 */
export function classifyVoiceErrorForDisplay(error: unknown): VoiceErrorKind {
  const code = normalizeVoiceErrorCode(error);
  if (code === 'not-allowed' || code === 'permission-denied') return 'permission';
  if (code === 'audio-capture') return 'audio_capture';
  if (code === 'network') return 'network';
  if (code === 'service-not-allowed') return 'service';
  if (code === 'language-not-supported') return 'language';
  return 'other';
}

/** Errors the original never showed to the user during voice input. */
export function isSilentRecognitionError(error: string | null | undefined): boolean {
  return error === 'no-speech' || error === 'aborted';
}

// ---------------------------------------------------------------------------
// Diagnostic helpers (voice.ts voiceDiagClass / shouldShowRecoveryGuide)
// ---------------------------------------------------------------------------

export type DiagnosticSeverity = 'ok' | 'err' | 'warn' | '';

/** Severity of a diagnostic status (voice.ts voiceDiagClass; the host maps it to its own styling). */
export function diagnosticSeverity(status: DiagnosticStatus): DiagnosticSeverity {
  if (status === 'healthy') return 'ok';
  if (status === 'permission_denied' || status === 'audio_capture_failed' || status === 'speech_service_failed') return 'err';
  if (status === 'profile_or_stt_stuck_suspected' || status === 'normal_profile_specific' || status === 'no_result') return 'warn';
  return '';
}

/** Whether the host should show its recovery guide for this status. */
export function shouldShowRecoveryGuide(status: DiagnosticStatus): boolean {
  return status === 'profile_or_stt_stuck_suspected' || status === 'normal_profile_specific';
}

/**
 * many-ai-cli's UI language to recognition language (voice.ts getLang). `null`/empty means
 * the many-ai-cli default `ja`.
 */
export function appLangToRecognitionLang(appLang: string | null | undefined): string {
  const lang = appLang || 'ja';
  if (lang === 'ja') return 'ja-JP';
  if (lang === 'vi') return 'vi-VN';
  return 'en-US';
}

// ---------------------------------------------------------------------------
// Tiny typed emitter (no EventTarget / CustomEvent dependency)
// ---------------------------------------------------------------------------

type Listener<T> = (payload: T) => void;

class Emitter<M> {
  private readonly listeners = new Map<keyof M, Set<Listener<never>>>();

  on<K extends keyof M>(type: K, fn: Listener<M[K]>): () => void {
    let set = this.listeners.get(type);
    if (!set) {
      set = new Set();
      this.listeners.set(type, set);
    }
    set.add(fn as Listener<never>);
    return () => {
      set.delete(fn as Listener<never>);
    };
  }

  // Like dispatchEvent, a throwing listener is reported and does not stop the others or
  // the engine's own state transitions.
  emit<K extends keyof M>(type: K, payload: M[K]): void {
    const set = this.listeners.get(type);
    if (!set) return;
    for (const fn of Array.from(set)) {
      try {
        (fn as Listener<M[K]>)(payload);
      } catch (err) {
        host().console?.error?.('vtype-core listener failed:', err);
      }
    }
  }

  clear(): void {
    this.listeners.clear();
  }
}

// ---------------------------------------------------------------------------
// Diagnostic log (voice.ts pushVoiceDiagEvent / setVoiceDiagStatus / createVoiceDiagReport)
// ---------------------------------------------------------------------------

export type RecognitionId = number | string | null;

export interface VoiceDiagEvent {
  readonly timestamp: string;
  readonly recognitionId: RecognitionId;
  readonly event: string;
  readonly error: string | null;
  readonly message: string | null;
  readonly hasResult: boolean;
  readonly transcriptLength: number;
}

export interface VoiceDiagEventDetail {
  error?: string | null;
  message?: string | null;
  hasResult?: boolean;
  transcriptLength?: number;
}

export interface DiagnosticSnapshot {
  readonly status: DiagnosticStatus;
  /** Extra detail (usually the browser's error message). Empty string when none. */
  readonly message: string;
  readonly events: VoiceDiagEvent[];
}

export interface VoiceDiagReport {
  readonly generatedAt: string;
  readonly appVersion: string | null;
  readonly userAgent: string | undefined;
  readonly userAgentData: {
    brands: NavigatorUADataLike['brands'];
    mobile: boolean | undefined;
    platform: string | undefined;
  } | null;
  readonly origin: string | null;
  readonly isLocalOrigin: boolean;
  readonly speechRecognitionSupported: boolean;
  readonly chromiumDetected: boolean;
  readonly status: DiagnosticStatus;
  readonly message: string;
  readonly events: VoiceDiagEvent[];
}

class DiagnosticsLog {
  events: VoiceDiagEvent[] = [];
  lastDetail = '';
  seq = 0;

  constructor(
    public status: DiagnosticStatus,
    private readonly limit: number,
    private readonly onStatus: (snapshot: DiagnosticSnapshot) => void,
  ) {}

  push(recognitionId: RecognitionId, event: string, detail: VoiceDiagEventDetail = {}): VoiceDiagEvent {
    const item: VoiceDiagEvent = {
      timestamp: new Date().toISOString(),
      recognitionId: recognitionId == null ? null : recognitionId,
      event,
      error: detail?.error || null,
      message: detail?.message || null,
      hasResult: !!detail?.hasResult,
      transcriptLength: Number.isFinite(detail?.transcriptLength) ? (detail.transcriptLength as number) : 0,
    };
    this.events.push(item);
    while (this.events.length > this.limit) this.events.shift();
    return item;
  }

  setStatus(status: DiagnosticStatus, detail: string | null = null): void {
    this.status = status;
    this.lastDetail = detail || '';
    this.onStatus({ status, message: this.lastDetail, events: this.events.slice() });
  }
}

// ---------------------------------------------------------------------------
// Main recognizer (voice.ts first IIFE, lines 15-642)
// ---------------------------------------------------------------------------

export interface SpeechRecognizerOptions extends SupportOptions {
  /**
   * Recognition language (BCP 47), or a function read right before each start. voice.ts read
   * it at construction, on every instance re-creation and right before start().
   */
  lang: string | (() => string);
  /** Defaults to VOICE_DIAG_STUCK_MS (20000). */
  stuckTimeoutMs?: number;
  /** Defaults to VOICE_DIAG_EVENT_LIMIT (80). */
  diagnosticEventLimit?: number;
  /** Supplies `appVersion` for the diagnostic report (voice.ts read `.settings-app-version`). */
  getAppVersion?: () => string | null | undefined;
}

export interface SpeechRecognizerState {
  readonly supported: boolean;
  /** Between the native `start` event and the stop (many-ai-cli `voiceActive` / `recording`). */
  readonly recording: boolean;
  /**
   * start() accepted but the native `start` event has not arrived yet. voice.ts did not track
   * this; it is here so other engines can refuse to open the microphone meanwhile.
   */
  readonly starting: boolean;
  /** Microphone audio is being captured (many-ai-cli `voiceAudioActive`). */
  readonly audioActive: boolean;
  /** `audioend` seen, waiting for the result (many-ai-cli `voice-processing`). */
  readonly processing: boolean;
  /** A diagnostic run holds its own recognition instance. */
  readonly diagnosticRunning: boolean;
  /** Id of the current recognition instance. A new id is issued after every stop. */
  readonly recognitionId: number;
}

export type RecognitionActivity =
  | 'audiostart'
  | 'soundstart'
  | 'speechstart'
  | 'speechend'
  | 'soundend'
  | 'audioend'
  | 'nomatch';

export interface SpeechRecognizerResult {
  readonly recognitionId: number;
  /**
   * False when the result came from an instance that was already replaced (for example the
   * final result that Chrome delivers after stop()). voice.ts applied those results too.
   */
  readonly isCurrent: boolean;
  /** `results[resultIndex][0].transcript`, passed through as is (may be an empty string). */
  readonly transcript: string;
  readonly isFinal: boolean;
  readonly resultIndex: number;
}

export interface SpeechRecognizerError {
  readonly recognitionId: number;
  /** Normalized code (`not-allowed`, `aborted`, `InvalidStateError`, ...). */
  readonly code: string;
  /** Raw `event.error` of an onerror event; null for a start() exception. */
  readonly error: string | null;
  readonly message: string | null;
  /** `recognition`: onerror event. `start`: start() threw. */
  readonly source: 'recognition' | 'start';
  /** voice.ts showed a toast for this error (everything except no-speech / aborted). */
  readonly notify: boolean;
}

export interface SpeechRecognizerEventMap {
  /** Any field of the state snapshot changed. */
  state: SpeechRecognizerState;
  /** Native `start` fired (voice.ts dispatched `voiceinput:started`). */
  start: { readonly recognitionId: number };
  /** Recording ended for any reason (voice.ts dispatched `voiceinput:stopped`). */
  stop: { readonly recognitionId: number };
  result: SpeechRecognizerResult;
  error: SpeechRecognizerError;
  /** audioActive changed (voice.ts dispatched `voiceinput:statechanged`). */
  audioActive: boolean;
  /** Lower-level recognition events, e.g. to drive a waveform. */
  activity: { readonly recognitionId: number; readonly kind: RecognitionActivity };
  /** Diagnostic status changed (voice.ts dispatched `voiceinput:diagnostic`). */
  diagnostic: DiagnosticSnapshot;
}

export interface StartOutcome {
  readonly started: boolean;
  /** Error code when not started: `unsupported`, `disposed`, or the normalized start() error. */
  readonly error?: string;
}

export interface VoiceDiagnostics {
  getStatus(): DiagnosticStatus;
  getLastDetail(): string;
  getEvents(): VoiceDiagEvent[];
  getReport(): VoiceDiagReport;
  /** `JSON.stringify(getReport(), null, 2)`: the text voice.ts copied to the clipboard. */
  getReportJson(): string;
  /** Run the diagnostic with its own recognition instance (voice.ts runVoiceDiagnostic). */
  run(): void;
  /** Set status `normal_profile_specific` (window.__anyAiCliVoiceDiagnostics.markNormalProfileSpecific). */
  markNormalProfileSpecific(): void;
  /** Log the user's confirmation and set `normal_profile_specific` (the diagnostic "profile specific" button). */
  confirmNormalProfileSpecific(): void;
  pushEvent(recognitionId: RecognitionId, event: string, detail?: VoiceDiagEventDetail): VoiceDiagEvent;
}

export interface SpeechRecognizer {
  readonly support: RecognitionSupport;
  readonly diagnostics: VoiceDiagnostics;
  /**
   * Start listening (the non-recording branch of the voice button). Does not check whether
   * a recording is already running, like voice.ts; use isRecording() for the toggle.
   */
  start(): StartOutcome;
  /** Confirm: native stop() then finish immediately (voice.ts confirm button). */
  stop(): void;
  /** Cancel: native abort() then finish immediately (voice.ts cancel button / toggle while recording). */
  abort(): void;
  /**
   * Native stop() only; the finish happens on the `end` event (voice.ts trigger-phrase path).
   * Exceptions from the native call are not caught, as in voice.ts.
   */
  requestStop(): void;
  /** Log a `click` diagnostic event for the current instance (voice.ts button handler). */
  recordClick(): void;
  getState(): SpeechRecognizerState;
  isRecording(): boolean;
  isAudioActive(): boolean;
  /** recording, starting or a diagnostic run holds the microphone. */
  isActive(): boolean;
  getRecognitionId(): number;
  on<K extends keyof SpeechRecognizerEventMap>(type: K, handler: Listener<SpeechRecognizerEventMap[K]>): () => void;
  /** Abort, clear timers and drop all listeners. Not present in voice.ts (it lived for the page). */
  dispose(): void;
}

function errorMessage(err: unknown): string | null {
  if (err && typeof err === 'object' && 'message' in err) {
    const m = (err as { message?: unknown }).message;
    return typeof m === 'string' && m ? m : null;
  }
  return null;
}

function resolveLang(lang: string | (() => string)): string {
  return typeof lang === 'function' ? lang() : lang;
}

export function createSpeechRecognizer(options: SpeechRecognizerOptions): SpeechRecognizer {
  const emitter = new Emitter<SpeechRecognizerEventMap>();
  const resolved = resolveSupport(options);
  const SR = resolved.ctor;
  const support: RecognitionSupport = {
    supported: resolved.supported,
    speechRecognitionSupported: resolved.speechRecognitionSupported,
    chromiumDetected: resolved.chromiumDetected,
  };
  const stuckMs = options.stuckTimeoutMs ?? VOICE_DIAG_STUCK_MS;
  const diag = new DiagnosticsLog(
    support.supported ? 'idle' : 'unsupported',
    options.diagnosticEventLimit ?? VOICE_DIAG_EVENT_LIMIT,
    (snapshot) => emitter.emit('diagnostic', snapshot),
  );
  const getLang = () => resolveLang(options.lang);

  let dbgSeq = 0;
  let recognition: SpeechRecognitionLike | null = null;
  let recognitionId = 0;
  let isRecording = false;
  let startingId: number | null = null;
  let audioActive = false;
  let processing = false;
  let diagnosticRunning = false;
  let audioendStuckTimer: TimerHandle = null;
  let disposed = false;
  let lastState: SpeechRecognizerState | null = null;

  function getState(): SpeechRecognizerState {
    return {
      supported: support.supported,
      recording: isRecording,
      starting: startingId !== null,
      audioActive,
      processing,
      diagnosticRunning,
      recognitionId,
    };
  }

  function emitState(): void {
    const next = getState();
    const prev = lastState;
    if (
      prev
      && prev.recording === next.recording
      && prev.starting === next.starting
      && prev.audioActive === next.audioActive
      && prev.processing === next.processing
      && prev.diagnosticRunning === next.diagnosticRunning
      && prev.recognitionId === next.recognitionId
    ) return;
    lastState = next;
    emitter.emit('state', next);
  }

  function clearStarting(id: number): void {
    if (startingId === id) startingId = null;
  }

  // voice.ts setVoiceAudioActive: only acts on a change.
  function setAudioActive(active: boolean): void {
    if (audioActive === active) return;
    audioActive = active;
    emitter.emit('audioActive', active);
  }

  function configureRecognition(rec: SpeechRecognitionLike): void {
    rec.interimResults = true;
    rec.continuous = false;
    rec.maxAlternatives = 1;
    rec.lang = getLang();
  }

  function createInstance(): void {
    if (!SR) return;
    const rec = new SR();
    recognitionId = ++dbgSeq;
    configureRecognition(rec);
    recognition = rec;
    attachHandlers(rec, recognitionId);
  }

  // voice.ts stopVoice. Runs on every terminal event and on manual stop/cancel. Only when a
  // recording was running does it finish the recording and replace the instance.
  function stopVoice(): void {
    diag.push(recognitionId, 'stopVoice', { message: 'manual or terminal stop' });
    clearTimer(audioendStuckTimer);
    audioendStuckTimer = null;
    if (!isRecording) return;
    const stoppedId = recognitionId;
    isRecording = false;
    setAudioActive(false);
    processing = false;
    emitter.emit('stop', { recognitionId: stoppedId });
    // 次回のためにインスタンス作り直し（Chrome stuck 対策）
    // Chrome can keep a finished/aborted instance internally "started"; starting it again
    // then captures audio but never delivers a result. Always start from a fresh instance.
    if (!disposed) createInstance();
    emitState();
  }

  function attachHandlers(rec: SpeechRecognitionLike, myId: number): void {
    const activity = (kind: RecognitionActivity) => emitter.emit('activity', { recognitionId: myId, kind });

    rec.onstart = () => {
      diag.push(myId, 'start');
      isRecording = true;
      clearStarting(myId);
      setAudioActive(true);
      emitter.emit('start', { recognitionId: myId });
      emitState();
    };

    rec.onaudiostart = () => {
      diag.push(myId, 'audiostart');
      activity('audiostart');
    };

    rec.onsoundstart = () => {
      diag.push(myId, 'soundstart');
      activity('soundstart');
    };

    rec.onspeechstart = () => {
      diag.push(myId, 'speechstart');
      activity('speechstart');
    };

    rec.onspeechend = () => {
      diag.push(myId, 'speechend');
      activity('speechend');
    };

    rec.onsoundend = () => {
      diag.push(myId, 'soundend');
      activity('soundend');
    };

    rec.onaudioend = () => {
      diag.push(myId, 'audioend');
      setAudioActive(false);
      processing = true;
      clearTimer(audioendStuckTimer);
      audioendStuckTimer = setTimer(() => {
        diag.push(myId, 'stuck-timeout', { message: 'audioend without result/end/error' });
        diag.setStatus('profile_or_stt_stuck_suspected');
      }, stuckMs);
      activity('audioend');
      emitState();
    };

    rec.onresult = (e) => {
      clearTimer(audioendStuckTimer);
      audioendStuckTimer = null;
      processing = false;
      const result = e.results[e.resultIndex];
      if (!result) {
        emitState();
        return;
      }
      // Passed through unfiltered: voice.ts applied empty transcripts too (an empty final
      // result still ended the segment). Filtering, if wanted, is the caller's decision.
      const transcript = result[0]!.transcript;
      diag.push(myId, 'result', { hasResult: true, transcriptLength: transcript.length });
      emitter.emit('result', {
        recognitionId: myId,
        isCurrent: myId === recognitionId,
        transcript,
        isFinal: result.isFinal,
        resultIndex: e.resultIndex,
      });
      emitState();
    };

    rec.onnomatch = () => {
      diag.push(myId, 'nomatch');
      activity('nomatch');
    };

    rec.onend = () => {
      clearTimer(audioendStuckTimer);
      audioendStuckTimer = null;
      diag.push(myId, 'end');
      clearStarting(myId);
      stopVoice();
      emitState();
    };

    rec.onerror = (e) => {
      clearTimer(audioendStuckTimer);
      audioendStuckTimer = null;
      diag.push(myId, 'error', { error: e.error || null, message: e.message || null });
      clearStarting(myId);
      // `aborted` (e.g. another recognition took the microphone) and `no-speech` are not shown
      // to the user; every error still ends the recording and replaces the instance.
      emitter.emit('error', {
        recognitionId: myId,
        code: normalizeVoiceErrorCode(e),
        error: e.error ?? null,
        message: e.message || null,
        source: 'recognition',
        notify: !isSilentRecognitionError(e.error),
      });
      stopVoice();
      emitState();
    };
  }

  function start(): StartOutcome {
    if (disposed) return { started: false, error: 'disposed' };
    if (!support.supported || !recognition) return { started: false, error: 'unsupported' };
    const rec = recognition;
    const id = recognitionId;
    rec.lang = getLang();
    startingId = id;
    try {
      rec.start();
    } catch (err) {
      clearStarting(id);
      const code = normalizeVoiceErrorCode(err);
      emitter.emit('error', {
        recognitionId: id,
        code,
        error: null,
        message: errorMessage(err),
        source: 'start',
        notify: true,
      });
      emitState();
      return { started: false, error: code };
    }
    emitState();
    return { started: true };
  }

  function stop(): void {
    if (!recognition) return;
    startingId = null;
    try { recognition.stop(); } catch (_) { /* ignored as in voice.ts */ }
    stopVoice();
    emitState();
  }

  function abort(): void {
    if (!recognition) return;
    startingId = null;
    try { recognition.abort(); } catch (_) { /* ignored as in voice.ts */ }
    stopVoice();
    emitState();
  }

  function requestStop(): void {
    recognition?.stop();
  }

  // voice.ts runVoiceDiagnostic.
  function runDiagnostic(): void {
    if (disposed) return;
    if (!SR || !support.supported) {
      diag.setStatus('unsupported');
      return;
    }
    if (isRecording) {
      try { recognition?.abort(); } catch (_) { /* ignored as in voice.ts */ }
      stopVoice();
    }
    const d = new SR();
    const diagId = 'diag-' + (++diag.seq);
    let settled = false;
    let sawResult = false;
    let sawError = false;
    let stuckTimer: TimerHandle = null;
    let hardTimer: TimerHandle = null;
    configureRecognition(d);
    diagnosticRunning = true;

    function clearTimers(): void {
      clearTimer(stuckTimer);
      clearTimer(hardTimer);
      stuckTimer = null;
      hardTimer = null;
    }
    function finish(status: DiagnosticStatus, detail = ''): void {
      if (settled) return;
      settled = true;
      clearTimers();
      diagnosticRunning = false;
      setAudioActive(false);
      diag.setStatus(status, detail);
      emitState();
      try { d.abort(); } catch (_) { /* ignored as in voice.ts */ }
    }
    function event(name: string, detail: VoiceDiagEventDetail = {}): void {
      diag.push(diagId, name, detail || {});
    }

    diag.setStatus('running');
    event('click');
    emitState();
    hardTimer = setTimer(() => {
      event('diagnostic-timeout', { message: 'no terminal event within hard timeout' });
      finish('profile_or_stt_stuck_suspected');
    }, stuckMs + VOICE_DIAG_HARD_TIMEOUT_EXTRA_MS);

    d.onstart = () => {
      event('start');
      setAudioActive(true);
      emitState();
    };
    d.onaudiostart = () => event('audiostart');
    d.onsoundstart = () => event('soundstart');
    d.onspeechstart = () => event('speechstart');
    d.onspeechend = () => event('speechend');
    d.onsoundend = () => event('soundend');
    d.onaudioend = () => {
      event('audioend');
      setAudioActive(false);
      emitState();
      stuckTimer = setTimer(() => {
        event('stuck-timeout', { message: 'audioend without result/end/error' });
        finish('profile_or_stt_stuck_suspected');
      }, stuckMs);
    };
    d.onresult = (e) => {
      const result = e.results[e.resultIndex];
      const text = result?.[0]?.transcript || '';
      sawResult = true;
      event('result', { hasResult: true, transcriptLength: text.length });
      finish('healthy');
    };
    d.onnomatch = () => event('nomatch');
    d.onerror = (e) => {
      sawError = true;
      const error = normalizeVoiceErrorCode(e);
      event('error', { error, message: e.message || null });
      finish(classifyVoiceError(error), e.message || '');
    };
    d.onend = () => {
      event('end');
      if (!sawResult && !sawError) finish('no_result');
    };

    try {
      d.start();
    } catch (err) {
      const error = normalizeVoiceErrorCode(err);
      const message = errorMessage(err);
      event('start-error', { error, message });
      finish(classifyVoiceError(error), message || '');
    }
  }

  function createReport(): VoiceDiagReport {
    const nav = resolved.navigator;
    const uaData = nav?.userAgentData ? {
      brands: nav.userAgentData.brands,
      mobile: nav.userAgentData.mobile,
      platform: nav.userAgentData.platform,
    } : null;
    const origin = host().location?.origin ?? null;
    return {
      generatedAt: new Date().toISOString(),
      appVersion: options.getAppVersion?.() || null,
      userAgent: nav?.userAgent,
      userAgentData: uaData,
      origin,
      isLocalOrigin: /^https?:\/\/127\.0\.0\.1(?::\d+)?$/.test(origin ?? ''),
      speechRecognitionSupported: !!SR,
      chromiumDetected: !!support.chromiumDetected,
      status: diag.status,
      message: diag.lastDetail,
      events: diag.events.slice(),
    };
  }

  const diagnostics: VoiceDiagnostics = {
    getStatus: () => diag.status,
    getLastDetail: () => diag.lastDetail,
    getEvents: () => diag.events.slice(),
    getReport: createReport,
    getReportJson: () => JSON.stringify(createReport(), null, 2),
    run: runDiagnostic,
    markNormalProfileSpecific: () => diag.setStatus('normal_profile_specific'),
    confirmNormalProfileSpecific: () => {
      diag.push(null, 'normal-profile-specific-confirmed', {
        message: 'user confirmed Incognito or a new profile works',
      });
      diag.setStatus('normal_profile_specific');
    },
    pushEvent: (id, event, detail) => diag.push(id, event, detail),
  };

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    clearTimer(audioendStuckTimer);
    audioendStuckTimer = null;
    const rec = recognition;
    if (rec) {
      try { rec.abort(); } catch (_) { /* ignore */ }
      rec.onstart = rec.onaudiostart = rec.onsoundstart = rec.onspeechstart = null;
      rec.onspeechend = rec.onsoundend = rec.onaudioend = rec.onnomatch = rec.onend = null;
      rec.onresult = null;
      rec.onerror = null;
    }
    recognition = null;
    isRecording = false;
    startingId = null;
    audioActive = false;
    processing = false;
    emitter.clear();
  }

  if (support.supported) {
    createInstance();
    diag.setStatus('idle');
  }
  lastState = getState();

  return {
    support,
    diagnostics,
    start,
    stop,
    abort,
    requestStop,
    recordClick: () => {
      diag.push(recognitionId, 'click');
    },
    getState,
    isRecording: () => isRecording,
    isAudioActive: () => audioActive,
    isActive: () => isRecording || startingId !== null || diagnosticRunning,
    getRecognitionId: () => recognitionId,
    on: (type, handler) => emitter.on(type, handler),
    dispose,
  };
}

// ---------------------------------------------------------------------------
// Hotword (wake word) listener (voice.ts second IIFE, lines 645-1021)
//
// A second recognition instance that listens for a phrase and then hands over to the main
// recognizer. Chrome cannot run two recognitions in one page at the same time; restarting
// this one while the main recognizer runs fights over the microphone (InvalidStateError,
// or "waveform moves but no result"). The exclusion is: never start while the host says voice
// input is busy, and stopForVoiceInput() before the main recognizer starts.
//
// Note: in many-ai-cli this block is currently disabled by an early `return;` (line 649),
// so the listener is ported but nothing instantiates it there today.
// ---------------------------------------------------------------------------

export interface HotwordListenerOptions extends SupportOptions {
  lang: string | (() => string);
  /**
   * The wake phrase, or '' when wake word is disabled (voice.ts getWakePhrase: '' unless the
   * setting is on, then the trimmed phrase).
   */
  getPhrase: () => string;
  /** Text normalization applied to both the phrase and transcripts (voice.ts normalizeTriggerMatchText). Defaults to identity. */
  normalize?: (text: string) => string;
  /** Host conditions for listening: enabled, armed (global or session) and hovered. */
  canListen: () => boolean;
  /** Main voice input is running or about to (voice.ts `voiceActive || _voiceIntentActive()`). */
  isVoiceBusy?: () => boolean;
  /** Phrase detected. stopForVoiceInput() has already been started (not awaited, as in voice.ts). */
  onWake?: () => void;
  /** A fatal error: the host should disarm wake word and show the error. */
  onFatalError?: (error: { code: string; error: string; message: string | null }) => void;
  /** Listening state changed (the places voice.ts called updateMicChip). */
  onChange?: (state: HotwordState) => void;
}

export interface HotwordState {
  readonly listening: boolean;
  readonly starting: boolean;
}

export interface HotwordListener {
  readonly support: RecognitionSupport;
  /** Start listening if canListen() and not already listening/starting (voice.ts startHotword). */
  start(): void;
  /** Abort listening and cancel a pending restart (voice.ts stopHotword). */
  stop(): void;
  /**
   * Release the microphone before the main recognizer starts (voice.ts stopHotwordForVoiceInput).
   * Resolves true if the hotword was active; always replaces the instance.
   */
  stopForVoiceInput(): Promise<boolean>;
  /** Call when the main voice input stopped: re-arms after a short delay (voice.ts `voiceinput:stopped` listener). */
  notifyVoiceStopped(): void;
  canListen(): boolean;
  getState(): HotwordState;
  /** listening or starting: the hotword holds (or is about to hold) the microphone. */
  isActive(): boolean;
  /** Not present in voice.ts. */
  dispose(): void;
}

type HotwordEventMap = {
  start: { readonly currentTarget?: unknown };
  end: { readonly currentTarget?: unknown };
  result: SpeechRecognitionEventLike;
  error: SpeechRecognitionErrorEventLike;
};

export function createHotwordListener(options: HotwordListenerOptions): HotwordListener {
  const resolved = resolveSupport(options);
  const support: RecognitionSupport = {
    supported: resolved.supported,
    speechRecognitionSupported: resolved.speechRecognitionSupported,
    chromiumDetected: resolved.chromiumDetected,
  };
  const SR = resolved.ctor;
  if (!SR || !support.supported) {
    return {
      support,
      start: () => {},
      stop: () => {},
      stopForVoiceInput: () => Promise.resolve(false),
      notifyVoiceStopped: () => {},
      canListen: () => false,
      getState: () => ({ listening: false, starting: false }),
      isActive: () => false,
      dispose: () => {},
    };
  }
  const Ctor: SpeechRecognitionConstructor = SR;
  const normalize = options.normalize ?? ((text: string) => text);

  let isListening = false;
  let isStarting = false;
  let restartTimer: TimerHandle = null;
  let disposed = false;

  // hw も recognition と同じく 'aborted' 後に内部 state が "started" のまま固着し、
  // abort() / stop() が無視される stuck 状態に陥ることがある。差し替え可能にし、stuck を疑う経路で破棄する。
  let hw: SpeechRecognitionLike;
  const hotwordListeners: Array<[string, (ev: never) => void]> = [];
  function onHotword<K extends keyof HotwordEventMap>(eventName: K, handler: (e: HotwordEventMap[K]) => void): void {
    hotwordListeners.push([eventName, handler as (ev: never) => void]);
    hw.addEventListener(eventName, handler as (ev: never) => void);
  }
  function configureHotword(rec: SpeechRecognitionLike): void {
    rec.interimResults = true;
    rec.continuous = false;
    rec.maxAlternatives = 1;
  }
  function recreateHotword(): void {
    const oldHotword = hw;
    hw = new Ctor();
    configureHotword(hw);
    for (const [name, fn] of hotwordListeners) {
      hw.addEventListener(name, fn);
    }
    try { oldHotword.abort(); } catch (_) { /* ignored as in voice.ts */ }
  }
  function isCurrentHotwordEvent(e: { readonly currentTarget?: unknown } | null | undefined): boolean {
    return !e || !e.currentTarget || e.currentTarget === hw;
  }
  hw = new Ctor();
  configureHotword(hw);

  function canListen(): boolean {
    const voiceBusy = options.isVoiceBusy?.() ?? false;
    return !disposed && !!options.canListen() && !voiceBusy;
  }

  function notifyChange(): void {
    options.onChange?.({ listening: isListening, starting: isStarting });
  }

  function startHotword(): void {
    if (!canListen() || isListening || isStarting) return;
    isStarting = true;
    hw.lang = resolveLang(options.lang);
    try {
      hw.start();
    } catch (err) {
      isStarting = false;
      host().console?.warn?.('Wake word recognition start failed:', err);
    }
  }

  function stopHotword(): void {
    clearTimer(restartTimer);
    restartTimer = null;
    try { hw.abort(); } catch (_) { /* ignored as in voice.ts */ }
  }

  // hw が active だった場合は hw.end ＋短い余裕（マイクキャプチャ解放待ち）を経てから resolve する。
  // hw.abort() は非同期で、直後に同期で本体の start() を呼ぶと Chrome のマイクが半分掴まれた状態で start し、
  // audiostart は発火するが result が届かない（「波形は出るがテキストが入らない」）。
  function stopHotwordForVoiceInput(): Promise<boolean> {
    const wasActive = isListening || isStarting;
    if (!wasActive) {
      stopHotword();
      isListening = false;
      isStarting = false;
      // 早期 return 経路でも hw を必ず作り直す（'aborted' 直後でも内部 state が "started" のまま固着しうる）。
      recreateHotword();
      notifyChange();
      return Promise.resolve(false);
    }
    return new Promise((resolve) => {
      let settled = false;
      const finish = () => {
        if (settled) return;
        settled = true;
        hw.removeEventListener('end', onEnd);
        isListening = false;
        isStarting = false;
        // end が来たケースでも作り直す。end = mic 解放完了とは限らない。
        recreateHotword();
        notifyChange();
        setTimer(() => resolve(true), HOTWORD_MIC_RELEASE_WAIT_MS);
      };
      const onEnd = () => finish();
      hw.addEventListener('end', onEnd);
      stopHotword();
      // セーフティ: end が来なくても強制 resolve（壊れた状態でブロックしない）。
      setTimer(finish, HOTWORD_STOP_SAFETY_MS);
    });
  }

  onHotword('start', (e) => {
    if (!isCurrentHotwordEvent(e)) return;
    isStarting = false;
    isListening = true;
    notifyChange();
  });

  onHotword('result', (e) => {
    if (!isCurrentHotwordEvent(e)) return;
    const phrase = normalize(options.getPhrase());
    if (!phrase) return;
    for (let i = e.resultIndex; i < e.results.length; i++) {
      const raw = e.results[i]![0]!.transcript;
      if (normalize(raw).includes(phrase)) {
        void stopHotwordForVoiceInput();
        options.onWake?.();
        return;
      }
    }
  });

  onHotword('end', (e) => {
    if (!isCurrentHotwordEvent(e)) return;
    isListening = false;
    isStarting = false;
    notifyChange();
    if (!canListen()) return;
    clearTimer(restartTimer);
    restartTimer = setTimer(() => {
      restartTimer = null;
      if (canListen()) startHotword();
    }, HOTWORD_RESTART_DELAY_MS);
  });

  onHotword('error', (e) => {
    if (!isCurrentHotwordEvent(e)) return;
    isStarting = false;
    // 'aborted' は Chrome 側で SpeechRecognition が stuck になる代表的な起点。
    // abort() / stop() 後の追い掛けや、本体の start() が mic を奪った結果として発生し、
    // この後 'end' が届かないケースがある。stuck な hw が mic を掴んだままだと
    // 直後の本体 start() で「波形は出るが result が届かない」ため、即座にインスタンスを捨てる。
    if (e.error === 'aborted') {
      isListening = false;
      recreateHotword();
      notifyChange();
      return;
    }
    if (e.error && HOTWORD_FATAL_ERRORS.includes(e.error)) {
      options.onFatalError?.({
        code: normalizeVoiceErrorCode(e),
        error: e.error,
        message: e.message || null,
      });
    }
  });

  function notifyVoiceStopped(): void {
    notifyChange();
    if (!canListen() || isListening || isStarting) return;
    clearTimer(restartTimer);
    restartTimer = setTimer(() => {
      restartTimer = null;
      if (canListen()) startHotword();
    }, HOTWORD_REARM_DELAY_MS);
  }

  function dispose(): void {
    if (disposed) return;
    stopHotword();
    disposed = true;
    isListening = false;
    isStarting = false;
  }

  return {
    support,
    start: startHotword,
    stop: stopHotword,
    stopForVoiceInput: stopHotwordForVoiceInput,
    notifyVoiceStopped,
    canListen,
    getState: () => ({ listening: isListening, starting: isStarting }),
    isActive: () => isListening || isStarting,
    dispose,
  };
}
