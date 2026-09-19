// Engine exclusivity: browser SpeechRecognition and getUserMedia (Whisper) never run together.
// Every browser API is a synthetic fake injected through options; no real browser is used.
import { describe, expect, it, vi } from 'vitest';
import {
  ENGINE_BUSY,
  buildTranscribeUrl,
  createVoiceInput,
  createWhisperRecorder,
  type AudioContextLike,
  type FetchLike,
  type GetUserMediaLike,
  type SpeechRecognitionConstructor,
  type SpeechRecognitionLike,
} from '../src/index.js';

const ENDPOINT = 'https://example.com/transcribe';
const TOKEN = 'test-token';

// --- fake SpeechRecognition ------------------------------------------------

type Handler = (ev: unknown) => void;

function makeFakeSpeechRecognition() {
  const instances: FakeSR[] = [];
  let startCalls = 0;
  class FakeSR implements SpeechRecognitionLike {
    lang = '';
    continuous = false;
    interimResults = false;
    maxAlternatives = 1;
    onstart: Handler | null = null;
    onaudiostart: Handler | null = null;
    onsoundstart: Handler | null = null;
    onspeechstart: Handler | null = null;
    onspeechend: Handler | null = null;
    onsoundend: Handler | null = null;
    onaudioend: Handler | null = null;
    onresult: SpeechRecognitionLike['onresult'] = null;
    onnomatch: Handler | null = null;
    onerror: SpeechRecognitionLike['onerror'] = null;
    onend: Handler | null = null;
    started = false;
    private listeners = new Map<string, Set<Handler>>();
    constructor() {
      instances.push(this);
    }
    start(): void {
      startCalls++;
      this.started = true;
    }
    stop(): void {}
    abort(): void {}
    addEventListener(type: string, listener: (ev: never) => void): void {
      if (!this.listeners.has(type)) this.listeners.set(type, new Set());
      this.listeners.get(type)!.add(listener as Handler);
    }
    removeEventListener(type: string, listener: (ev: never) => void): void {
      this.listeners.get(type)?.delete(listener as Handler);
    }
    /** Fire a native event: on<type> property first, then addEventListener listeners. */
    fire(type: string, ev: Record<string, unknown> = {}): void {
      const payload = { currentTarget: this, ...ev };
      const prop = (this as unknown as Record<string, unknown>)['on' + type];
      if (typeof prop === 'function') (prop as Handler)(payload);
      for (const l of Array.from(this.listeners.get(type) ?? [])) l(payload);
    }
  }
  return {
    ctor: FakeSR as unknown as SpeechRecognitionConstructor,
    instances,
    startCalls: () => startCalls,
    lastStarted: () => [...instances].reverse().find((i) => i.started)!,
  };
}

// --- fake WebAudio / getUserMedia / fetch ----------------------------------

function node() {
  return { connect: vi.fn(), disconnect: vi.fn() };
}

function makeFakeAudio() {
  const contexts: FakeAudioContext[] = [];
  class FakeAudioContext implements AudioContextLike {
    readonly sampleRate: number;
    state = 'running';
    readonly destination = node();
    scriptNode: ReturnType<AudioContextLike['createScriptProcessor']> | null = null;
    constructor(opts: { sampleRate: number }) {
      this.sampleRate = opts.sampleRate;
      contexts.push(this);
    }
    createMediaStreamSource() { return node(); }
    createAnalyser() { return { ...node(), fftSize: 0, getByteTimeDomainData: vi.fn() }; }
    createGain() { return { ...node(), gain: { value: 1 } }; }
    createScriptProcessor() {
      this.scriptNode = { ...node(), onaudioprocess: null };
      return this.scriptNode;
    }
    resume() { return Promise.resolve(); }
    close() { this.state = 'closed'; return Promise.resolve(); }
    /** Deliver one mono buffer through the ScriptProcessor path. */
    feed(samples: Float32Array): void {
      this.scriptNode!.onaudioprocess!({
        inputBuffer: { numberOfChannels: 1, length: samples.length, getChannelData: () => samples },
      });
    }
  }
  const trackStop = vi.fn();
  const getUserMedia = vi.fn<GetUserMediaLike>(async () => ({ getTracks: () => [{ stop: trackStop }] }));
  return { AudioContext: FakeAudioContext, contexts, getUserMedia, trackStop };
}

function okFetch(text: string) {
  return vi.fn<FetchLike>(async () => ({ ok: true, json: async () => ({ text }) }));
}

function setup(opts: { fetch?: FetchLike; hotword?: boolean } = {}) {
  const sr = makeFakeSpeechRecognition();
  const audio = makeFakeAudio();
  let clock = 0;
  const fetch = opts.fetch ?? okFetch('  hello world  ');
  const voice = createVoiceInput({
    recognition: { lang: 'en-US', SpeechRecognition: sr.ctor, isChromium: true },
    whisper: {
      endpoint: ENDPOINT,
      token: TOKEN,
      getUserMedia: audio.getUserMedia,
      AudioContext: audio.AudioContext,
      AudioWorkletNode: null,
      fetch,
      now: () => clock,
    },
    ...(opts.hotword
      ? { hotword: { lang: 'en-US', SpeechRecognition: sr.ctor, isChromium: true, getPhrase: () => 'wake', canListen: () => true } }
      : {}),
  });
  return { voice, sr, audio, fetch, setClock: (ms: number) => { clock = ms; } };
}

const speech = () => new Float32Array(4096).fill(0.1);
const silence = () => new Float32Array(4096);

// --- tests -----------------------------------------------------------------

describe('engine exclusivity', () => {
  it('refuses whisper while browser recognition is recording, without calling getUserMedia', async () => {
    const { voice, sr, audio } = setup();
    expect(voice.recognizer!.start()).toEqual({ started: true });
    sr.lastStarted().fire('start');
    expect(voice.recognizer!.isRecording()).toBe(true);

    expect(await voice.whisper!.start()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(await voice.whisper!.toggle()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(audio.getUserMedia).toHaveBeenCalledTimes(0);
    expect(voice.getActiveEngine()).toBe('browser');
  });

  it('refuses whisper while browser start() is pending (before the native start event)', async () => {
    const { voice, sr, audio } = setup();
    voice.recognizer!.start();
    expect(sr.startCalls()).toBe(1);
    expect(voice.recognizer!.getState().starting).toBe(true);

    expect(await voice.whisper!.start()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(audio.getUserMedia).toHaveBeenCalledTimes(0);
  });

  it('refuses browser recognition while whisper is recording, without calling SpeechRecognition.start', async () => {
    const { voice, sr, audio } = setup();
    expect(await voice.whisper!.start()).toEqual({ started: true });
    expect(audio.getUserMedia).toHaveBeenCalledTimes(1);

    expect(voice.recognizer!.start()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(sr.startCalls()).toBe(0);
    expect(voice.getActiveEngine()).toBe('whisper');
  });

  it('refuses browser recognition while whisper is still waiting for getUserMedia', async () => {
    const { voice, sr, audio } = setup();
    let grant!: () => void;
    audio.getUserMedia.mockImplementationOnce(
      () => new Promise((resolve) => { grant = () => resolve({ getTracks: () => [] }); }),
    );
    const pending = voice.whisper!.start();
    expect(voice.whisper!.getState().starting).toBe(true);

    expect(voice.recognizer!.start()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(sr.startCalls()).toBe(0);

    grant();
    expect(await pending).toEqual({ started: true });
  });

  it('refuses a diagnostic run while whisper is active, creating no recognition instance', async () => {
    const { voice, sr } = setup();
    await voice.whisper!.start();
    const before = sr.instances.length;

    voice.recognizer!.diagnostics.run();
    expect(sr.instances.length).toBe(before);
    expect(sr.startCalls()).toBe(0);
    expect(voice.recognizer!.diagnostics.getStatus()).toBe('idle');
    const last = voice.recognizer!.diagnostics.getEvents().at(-1)!;
    expect(last.event).toBe('diagnostic-refused');
    expect(last.error).toBe(ENGINE_BUSY);
  });

  it('refuses whisper while the hotword listener holds the microphone', async () => {
    const { voice, sr, audio } = setup({ hotword: true });
    voice.hotword!.start();
    expect(sr.startCalls()).toBe(1);
    sr.lastStarted().fire('start');
    expect(voice.hotword!.getState().listening).toBe(true);

    expect(await voice.whisper!.start()).toEqual({ started: false, error: ENGINE_BUSY });
    expect(audio.getUserMedia).toHaveBeenCalledTimes(0);
  });

  it('does not start the hotword while whisper is active', async () => {
    const { voice, sr } = setup({ hotword: true });
    await voice.whisper!.start();
    voice.hotword!.start();
    expect(sr.startCalls()).toBe(0);
  });

  it('lets each engine start once the other has fully stopped', async () => {
    const { voice, sr, audio } = setup();
    voice.recognizer!.start();
    const first = sr.lastStarted();
    first.fire('start');
    first.fire('end');
    expect(voice.recognizer!.isActive()).toBe(false);

    expect(await voice.whisper!.start()).toEqual({ started: true });
    expect(audio.getUserMedia).toHaveBeenCalledTimes(1);
    voice.whisper!.cancel();
    expect(voice.whisper!.isActive()).toBe(false);

    expect(voice.recognizer!.start()).toEqual({ started: true });
    expect(sr.startCalls()).toBe(2);
  });
});

describe('whisper transcription', () => {
  it('posts WAV to the injected endpoint with the token and delivers the trimmed text', async () => {
    const { voice, audio, fetch, setClock } = setup();
    const results: string[] = [];
    const stops: string[] = [];
    let browserStartFromStop: unknown = null;
    voice.whisper!.on('result', (r) => results.push(r.text));
    voice.whisper!.on('stop', (s) => {
      stops.push(s.reason);
      // A stop listener may start the other engine immediately.
      browserStartFromStop = voice.recognizer!.start();
    });

    await voice.whisper!.start();
    audio.contexts[0]!.feed(speech());
    setClock(1000);
    expect(await voice.whisper!.finish()).toEqual({ finished: true, text: 'hello world' });

    expect(fetch).toHaveBeenCalledTimes(1);
    const [url, init] = fetch.mock.calls[0]!;
    expect(url).toBe(`${ENDPOINT}?token=${TOKEN}`);
    expect(init.method).toBe('POST');
    expect(init.headers).toEqual({ 'Content-Type': 'audio/wav' });
    expect(init.body).toBeInstanceOf(Blob);
    expect((init.body as Blob).size).toBe(44 + 4096 * 2);
    expect(results).toEqual(['hello world']);
    expect(stops).toEqual(['result']);
    expect(browserStartFromStop).toEqual({ started: true });
    expect(audio.trackStop).toHaveBeenCalledTimes(1);
  });

  it('discards silence as no_speech without calling fetch', async () => {
    const { voice, audio, fetch, setClock } = setup();
    const errors: string[] = [];
    voice.whisper!.on('error', (e) => errors.push(e.code));
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});

    await voice.whisper!.start();
    audio.contexts[0]!.feed(silence());
    setClock(1000);
    expect(await voice.whisper!.finish()).toEqual({ finished: false, reason: 'error', error: 'no_speech' });
    expect(fetch).toHaveBeenCalledTimes(0);
    expect(errors).toEqual(['no_speech']);
    warn.mockRestore();
  });

  it('reports the server error code on a non-ok response', async () => {
    const failing = vi.fn<FetchLike>(async () => ({ ok: false, json: async () => ({ error: 'whisper_unreachable' }) }));
    const { voice, audio, setClock } = setup({ fetch: failing });
    await voice.whisper!.start();
    audio.contexts[0]!.feed(speech());
    setClock(1000);
    expect(await voice.whisper!.finish()).toEqual({ finished: false, reason: 'error', error: 'whisper_unreachable' });
  });

  it('builds the URL from function endpoint/token, and sends no token parameter when none is configured', () => {
    expect(buildTranscribeUrl(ENDPOINT)).toBe(ENDPOINT);
    expect(buildTranscribeUrl(ENDPOINT, TOKEN)).toBe(`${ENDPOINT}?token=${TOKEN}`);
    expect(buildTranscribeUrl(`${ENDPOINT}?lang=ja`, 'a b')).toBe(`${ENDPOINT}?lang=ja&token=a%20b`);
    expect(buildTranscribeUrl(ENDPOINT, null)).toBe(`${ENDPOINT}?token=`);
  });

  it('reports audio_capture and never touches getUserMedia when recording is unsupported', async () => {
    const rec = createWhisperRecorder({ endpoint: ENDPOINT, getUserMedia: null, AudioContext: null, fetch: okFetch('x') });
    const errors: string[] = [];
    rec.on('error', (e) => errors.push(e.code));
    expect(rec.support.supported).toBe(false);
    expect(await rec.start()).toEqual({ started: false, error: 'audio_capture' });
    expect(errors).toEqual(['audio_capture']);
  });
});
