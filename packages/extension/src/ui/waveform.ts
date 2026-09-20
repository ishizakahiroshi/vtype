// The bars that move while recording (plan C7c).
//
// Ported from many-ai-cli web/src/app/voice.ts (same author, MIT): drawBars / animLoop /
// startWaveform and the `activity` + `result` handlers that drive them.
//
// The bars are NOT driven by microphone volume. voice.ts has the note at showVoiceBar: calling
// getUserMedia({audio:true}) next to SpeechRecognition makes the two fight over the microphone,
// and the waveform then animates while no result ever arrives. The movement comes from the
// recognition events instead: each event sets an intensity target, the drawn intensity follows
// it smoothly, and a short "kick" is added when speech starts or the transcript grows.
//
// Values below are the ones in voice.ts today (checked in the source, not copied from the
// plan). Where the plan and the code disagreed, the code won; see the report.

export type WaveformActivity =
  | "audiostart"
  | "soundstart"
  | "speechstart"
  | "speechend"
  | "soundend"
  | "audioend"
  | "nomatch";

/** voice.ts startWaveform(): the target the bars begin at. */
export const START_TARGET = 0.05;
/** voice.ts rec.on('activity'): only these four kinds move the target. */
export const ACTIVITY_TARGETS: Readonly<Partial<Record<WaveformActivity, number>>> = {
  soundstart: 0.55,
  speechstart: 0.9,
  speechend: 0.25,
  audioend: 0.03,
};
/** Kinds that also restart the kick (voice.ts sets lastKickAt for these two). */
const KICKING_ACTIVITIES: ReadonlySet<WaveformActivity> = new Set(["soundstart", "speechstart"]);
/** voice.ts rec.on('result'): a transcript that grew lifts the target to at least this. */
export const TRANSCRIPT_TARGET = 0.85;
/** Per frame the intensity moves this fraction of the way to the target. */
export const SMOOTHING = 0.18;
/** The kick fades to zero over 1/3 s (1 - seconds * 3). */
export const KICK_DECAY_PER_SEC = 3;
export const KICK_WEIGHT = 0.6;
export const BAR_COUNT = 48;

const BASE_AMPLITUDE = 0.08;
const DYNAMIC_AMPLITUDE = 0.92;
const BAR_COLOR = "59,130,246"; // rgba(59,130,246,...) as in voice.ts

export interface WaveformOptions {
  doc?: Document;
  /** Clock for the kick decay (injected in tests). */
  now?: () => number;
  /** Whether the viewer asked for reduced motion; then the bars are drawn once, not animated. */
  reducedMotion?: () => boolean;
}

export interface Waveform {
  readonly element: HTMLCanvasElement;
  readonly running: boolean;
  /** Current target and the intensity following it (for tests and diagnostics). */
  readonly target: number;
  readonly intensity: number;
  start(): void;
  stop(): void;
  setActivity(kind: WaveformActivity): void;
  /** A recognition result arrived: a transcript that grew kicks the bars. */
  noteTranscript(transcript: string, isFinal: boolean): void;
  /** Advance one frame without waiting for requestAnimationFrame (tests). */
  step(): void;
}

function defaultReducedMotion(doc: Document): () => boolean {
  return () => {
    const view = doc.defaultView;
    if (view === null || typeof view.matchMedia !== "function") return false;
    return view.matchMedia("(prefers-reduced-motion: reduce)").matches;
  };
}

export function createWaveform(options: WaveformOptions = {}): Waveform {
  const doc = options.doc ?? document;
  const now = options.now ?? (() => Date.now());
  const reducedMotion = options.reducedMotion ?? defaultReducedMotion(doc);

  const element = doc.createElement("canvas");
  element.className = "waveform";
  element.setAttribute("aria-hidden", "true");

  let running = false;
  let frame: number | null = null;
  let wavePhase = 0;
  let intensity = 0;
  let target = 0;
  let lastKickAt = 0;
  let lastInterimLength = 0;

  function kick(): void {
    lastKickAt = now();
  }

  function activeLevel(): number {
    const sinceKick = (now() - lastKickAt) / 1000;
    const kickLevel = Math.max(0, 1 - sinceKick * KICK_DECAY_PER_SEC);
    return Math.min(1, intensity + kickLevel * KICK_WEIGHT);
  }

  function resize(): void {
    const view = doc.defaultView;
    const ratio = view?.devicePixelRatio ?? 1;
    const rect = element.getBoundingClientRect();
    const width = Math.round(rect.width * ratio);
    const height = Math.round(rect.height * ratio);
    if (width > 0 && height > 0 && (element.width !== width || element.height !== height)) {
      element.width = width;
      element.height = height;
    }
  }

  function draw(): void {
    const ctx = typeof element.getContext === "function" ? element.getContext("2d") : null;
    if (ctx === null) return; // no canvas support (also the case in the DOM used by tests)
    const W = element.width;
    const H = element.height;
    if (W === 0 || H === 0) return;
    ctx.clearRect(0, 0, W, H);
    const barW = Math.max(2, Math.floor(W / (BAR_COUNT * 1.8)));
    const gap = (W - BAR_COUNT * barW) / (BAR_COUNT + 1);
    const active = activeLevel();

    for (let i = 0; i < BAR_COUNT; i++) {
      // several sine waves plus pseudo noise, so the row looks like a waveform
      const phase = wavePhase + i * 0.42;
      const lo = Math.sin(phase) * 0.5 + 0.5;
      const hi = Math.sin(phase * 2.7 + i * 0.13) * 0.5 + 0.5;
      const rnd = (Math.sin(phase * 7.3 + i) + 1) * 0.5;
      const wave = lo * 0.4 + hi * 0.4 + rnd * 0.2;
      const v = BASE_AMPLITUDE + wave * DYNAMIC_AMPLITUDE * active;
      const barH = Math.max(barW, v * H * 0.92);
      const x = gap + i * (barW + gap);
      const y = (H - barH) / 2;
      ctx.fillStyle = `rgba(${BAR_COLOR},${Math.min(1, 0.35 + v * 0.85)})`;
      if (typeof ctx.roundRect === "function") {
        ctx.beginPath();
        ctx.roundRect(x, y, barW, barH, barW / 2);
        ctx.fill();
      } else {
        ctx.fillRect(x, y, barW, barH);
      }
    }
  }

  /** One frame: follow the target, draw, move the phase on (faster when louder). */
  function step(): void {
    intensity += (target - intensity) * SMOOTHING;
    draw();
    wavePhase += 0.18 + intensity * 0.35;
  }

  function loop(): void {
    frame = null;
    if (!running) return;
    step();
    schedule();
  }

  function schedule(): void {
    if (frame === null) frame = requestAnimationFrame(loop);
  }

  function start(): void {
    wavePhase = 0;
    intensity = 0;
    target = START_TARGET;
    lastKickAt = 0;
    lastInterimLength = 0;
    running = true;
    element.hidden = false;
    resize();
    if (reducedMotion()) {
      // One static frame: the bars are there, nothing moves.
      draw();
      return;
    }
    schedule();
  }

  function stop(): void {
    running = false;
    element.hidden = true;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
    intensity = 0;
    target = 0;
  }

  function redrawIfStill(): void {
    if (running && reducedMotion()) draw();
  }

  return {
    element,
    get running() {
      return running;
    },
    get target() {
      return target;
    },
    get intensity() {
      return intensity;
    },
    start,
    stop,
    setActivity(kind: WaveformActivity): void {
      if (!running) return;
      const next = ACTIVITY_TARGETS[kind];
      if (next === undefined) return; // audiostart / soundend / nomatch do not move the bars
      target = next;
      if (KICKING_ACTIVITIES.has(kind)) kick();
      redrawIfStill();
    },
    noteTranscript(transcript: string, isFinal: boolean): void {
      if (!running) return;
      if (transcript.length > lastInterimLength) {
        kick();
        target = Math.max(target, TRANSCRIPT_TARGET);
      }
      lastInterimLength = isFinal ? 0 : transcript.length;
      redrawIfStill();
    },
    step,
  };
}
