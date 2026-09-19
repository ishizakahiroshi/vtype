# vtype-core

Speech-to-text engines for voice input into web text fields. The package renders nothing: it
creates no DOM, uses no class names and shows no messages. It runs recognition and reports
state, results and errors, and the caller keeps its own UI.

Two engines:

- **Browser**: the Web Speech API (`SpeechRecognition` / `webkitSpeechRecognition`), Chromium only.
- **Whisper**: records with `getUserMedia` + WebAudio, encodes 16 kHz mono WAV and POSTs it to
  a transcription endpoint that **you** pass in (the package contains no URL of its own).

The two never run at the same time. Chrome gives the microphone to only one of them, and the
other one then receives nothing. `createVoiceInput()` enforces this: the second engine is
refused with `engine_busy`.

The package also runs inside a Chrome extension offscreen document. Every host API can be
injected (`SpeechRecognition`, `getUserMedia`, `AudioContext`, `AudioWorkletNode`, `fetch`,
clock), so it also runs in tests without a browser.

License: MIT.

## Install

```sh
pnpm add vtype-core
```

ESM only. TypeScript declarations are included.

## Quick start

```ts
import { createVoiceInput } from 'vtype-core';

const voice = createVoiceInput({
  engine: () => currentEngine,                        // 'browser' | 'whisper' | 'off'
  recognition: { lang: () => navigator.language || 'en-US' },
  whisper: {
    endpoint: 'https://example.com/transcribe',        // your transcription endpoint
    token: () => sessionToken,                         // sent as ?token=...; omit to send none
  },
});

voice.recognizer?.on('result', ({ transcript, isFinal }) => { /* show it */ });
voice.whisper?.on('result', ({ text }) => { /* show it */ });

micButton.onclick = async () => {
  const r = await voice.toggle();
  if (r.error === 'engine_busy') { /* the other engine holds the mic */ }
};
confirmButton.onclick = () => voice.confirm();
cancelButton.onclick = () => voice.cancel();
```

## Public API

### `createVoiceInput(options): VoiceInput`

The entry point that picks the engine and enforces mutual exclusion.

| option | type | meaning |
|---|---|---|
| `engine` | `'browser' \| 'whisper' \| 'off'` or a function returning one | Engine used by `toggle/confirm/cancel`. Default: `browser` if `recognition` is given, else `whisper` if `whisper` is given, else `off` |
| `recognition` | `SpeechRecognizerOptions` | Create the browser engine |
| `whisper` | `WhisperRecorderOptions` without `canStart` | Create the Whisper engine |
| `hotword` | `HotwordListenerOptions` | Create the wake-word listener (see the note on many-ai-cli below) |

| member | returns | meaning |
|---|---|---|
| `recognizer` | `SpeechRecognizer \| null` | Browser engine. `start()` and `diagnostics.run()` are refused while Whisper is active |
| `whisper` | `WhisperRecorder \| null` | Whisper engine. Starting is refused while the recognizer or the hotword is active |
| `hotword` | `HotwordListener \| null` | Its `isVoiceBusy` is combined with "browser recording or Whisper active" |
| `getEngine()` | `VoiceEngineName` | |
| `getActiveEngine()` | `'browser' \| 'whisper' \| null` | Which engine holds the microphone |
| `isActive()` | `boolean` | |
| `toggle()` | `Promise<VoiceToggleOutcome>` | Voice button: `{ engine, started, stopped, error? }`. `error` is `off`, `unavailable`, `engine_busy` or the engine's own error |
| `confirm()` | `Promise<WhisperFinishOutcome \| void>` | Confirm button |
| `cancel()` | `void` | Cancel button |
| `dispose()` | `void` | |

`ENGINE_BUSY` (`'engine_busy'`) is exported. **Exclusion only applies to engines created
through `createVoiceInput`.** If you create the engines separately, pass your own
`canStart` to the Whisper recorder and check `isActive()` yourself.

### `createSpeechRecognizer(options): SpeechRecognizer`

Browser engine (ported from many-ai-cli `voice.ts`).

Options: `lang` (string or function, **required**), `SpeechRecognition?` (constructor to inject;
`null` = missing), `navigator?`, `isChromium?`, `stuckTimeoutMs?` (20000),
`diagnosticEventLimit?` (80), `getAppVersion?` (for the diagnostic report).

| member | returns | meaning |
|---|---|---|
| `support` | `{ supported, speechRecognitionSupported, chromiumDetected }` | Unsupported when SpeechRecognition is missing or the browser is not Chromium |
| `start()` | `{ started, error? }` | `error`: `unsupported`, `disposed`, a start() exception code, or `engine_busy` via `createVoiceInput` |
| `stop()` | `void` | Confirm: native `stop()`, then finish immediately |
| `abort()` | `void` | Cancel: native `abort()`, then finish immediately |
| `requestStop()` | `void` | Native `stop()` only; the finish happens on `end` (trigger-phrase path) |
| `recordClick()` | `void` | Log a `click` diagnostic event |
| `getState()` | `SpeechRecognizerState` | `{ supported, recording, starting, audioActive, processing, diagnosticRunning, recognitionId }` |
| `isRecording() / isAudioActive() / isActive()` | `boolean` | `isActive` = recording, starting or diagnostic run |
| `getRecognitionId()` | `number` | Current instance id (a new id after every stop) |
| `on(type, handler)` | unsubscribe function | Events below |
| `diagnostics` | `VoiceDiagnostics` | `getStatus, getLastDetail, getEvents, getReport, getReportJson, run, markNormalProfileSpecific, confirmNormalProfileSpecific, pushEvent` |
| `dispose()` | `void` | |

Events: `state`, `start {recognitionId}`, `stop {recognitionId}`,
`result {recognitionId, isCurrent, transcript, isFinal, resultIndex}`,
`error {recognitionId, code, error, message, source, notify}`, `audioActive (boolean)`,
`activity {recognitionId, kind}` (`audiostart`, `soundstart`, `speechstart`, `speechend`,
`soundend`, `audioend`, `nomatch`), `diagnostic {status, message, events}`.

### `createWhisperRecorder(options): WhisperRecorder`

Whisper engine (ported from many-ai-cli `voice-whisper.ts`).

| option | default | meaning |
|---|---|---|
| `endpoint` | required | Transcription URL, or a function read per request |
| `token` | none | Token string / function. Sent as `?token=<encoded>`; omit to send no parameter. `null` / `''` send an empty parameter |
| `tokenQueryParam` | `token` | Parameter name |
| `recorderWorklet` | none | `{ url, processorName }` of an AudioWorklet that posts mono `Float32Array` chunks. Without it the ScriptProcessor path is used |
| `autoStop` | `true` | Finish after silence once speech was heard |
| `autoStopSilenceMs` | `2000` | Use `graceSecondsToSilenceMs(raw, defaultSec)` to convert a seconds setting (0 to 10 s clamp) |
| `maxRecordMs` | `120000` | |
| `canStart` | none | Return an error code to refuse opening the mic (set by `createVoiceInput`) |
| `getUserMedia`, `AudioContext`, `AudioWorkletNode`, `fetch`, `now` | host globals | Injection points. `null` = not available |

Request: `POST <endpoint>[?token=...]`, `Content-Type: audio/wav`, body = WAV Blob. Response
JSON: `{ text }` on success; `{ error: '<code>' }` with a non-2xx status on failure.

| member | returns | meaning |
|---|---|---|
| `support` | `{ supported }` | `getUserMedia` and `AudioContext` exist |
| `toggle()` | `Promise<{ started, finished?, error? }>` | Voice button: finish when recording, otherwise start |
| `start()` | same | Start only (`already_recording` when recording) |
| `finish()` | `Promise<{ finished, text?, reason?, error? }>` | Confirm: stop recording and transcribe |
| `cancel()` | `void` | Cancel recording or the pending request |
| `getState()` | `{ supported, starting, recording, processing, audioActive }` | |
| `isRecording() / isProcessing() / isActive()` | `boolean` | `isActive` = starting, recording or processing |
| `getAudioLevel()` | `number` (0..1) | Input level for a waveform |
| `on(type, handler)` | unsubscribe function | Events below |
| `dispose()` | `void` | |

Start errors: `audio_capture` (unsupported or capture failed), `permission_denied`,
`processing`, `already_recording`, `already_starting`, `disposed`, `engine_busy`.
Events: `state`, `start`, `processing`, `result {text}`,
`error {code, phase, cancelled, message}`, `notice {code: 'processing'}`,
`stop {reason: 'cancel' | 'result' | 'error'}`, `audioActive (boolean)`.
Error codes: `WHISPER_ERROR_CODES` plus `cancelled`.

### `createHotwordListener(options): HotwordListener`

Wake-word listener on a second SpeechRecognition instance. Options: `lang`, `getPhrase`,
`normalize?`, `canListen`, `isVoiceBusy?`, `onWake?`, `onFatalError?`, `onChange?`, and the
support options. Members: `start()`, `stop()`, `stopForVoiceInput(): Promise<boolean>`,
`notifyVoiceStopped()`, `canListen()`, `getState()`, `isActive()`, `dispose()`, `support`.

### Helpers

`detectSupport(options?)`, `getSpeechRecognitionConstructor()`, `isChromiumBrowser(nav?)`,
`normalizeVoiceErrorCode(err)`, `classifyVoiceError(code)` (→ diagnostic status),
`classifyVoiceErrorForDisplay(err)` (→ `permission | audio_capture | network | service | language | other`),
`isSilentRecognitionError(code)`, `diagnosticSeverity(status)`, `shouldShowRecoveryGuide(status)`,
`appLangToRecognitionLang(lang)`, `buildTranscribeUrl(endpoint, token?, param?)`,
`graceSecondsToSilenceMs(raw, defaultSec)`, `prepareBaseText(value)`, `joinTranscript(base, text)`,
`encodeWavPcm16(chunks, sourceRate, targetRate)`, plus the timing and threshold constants.

## Driving an existing UI unchanged (many-ai-cli)

The goal of the port is that many-ai-cli keeps its buttons, bar, toasts and shortcuts. Only
the engine code is replaced. The sketch below mirrors what `voice.ts` / `voice-whisper.ts`
did, using the host's own helpers (`inputEl`, `t`, `showToast`, `autoExpand`, ...). The
transcription path and token stay in many-ai-cli (`TRANSCRIBE_PATH` below is many-ai-cli's own
constant).

```ts
import { createVoiceInput, appLangToRecognitionLang, graceSecondsToSilenceMs,
  prepareBaseText, joinTranscript, classifyVoiceErrorForDisplay } from 'vtype-core';

const voice = createVoiceInput({
  engine: () => getVoiceEngine(),
  recognition: {
    lang: () => appLangToRecognitionLang(localStorage.getItem(STORAGE_LANG_KEY)),
    getAppVersion: () => readAppVersionLabel(),     // host reads its settings version label
  },
  whisper: {
    endpoint: TRANSCRIBE_PATH,
    token: () => token,
    recorderWorklet: { url: '/whisper-recorder-worklet.js', processorName: 'many-ai-cli-whisper-recorder' },
    autoStop: () => localStorage.getItem(STORAGE_VOICE_WHISPER_AUTO_STOP_KEY) !== '0',
    autoStopSilenceMs: () => graceSecondsToSilenceMs(localStorage.getItem(STORAGE_VOICE_GRACE_KEY), DEFAULT_VOICE_GRACE_SEC),
  },
  // no `hotword`: the wake-word block is disabled in voice.ts today
});
const rec = voice.recognizer!;
const wh = voice.whisper!;
let preVoiceText = '';
let interimStart = 0;

// voice button (click, Alt+V, the mobile hold button, ...)
btn.addEventListener('click', async () => {
  if (getVoiceEngine() === 'browser') {
    rec.recordClick();
    if (rec.isRecording()) { rec.abort(); return; }
    if (localStorage.getItem(STORAGE_VOICE_INPUT_DISABLED_KEY) === '1') return;
    preVoiceText = inputEl.value;
    inputEl.value = prepareBaseText(preVoiceText); updateInputClearButton();
    interimStart = inputEl.value.length;
    rec.start();                                   // errors arrive as 'error' events
  } else if (getVoiceEngine() === 'whisper') {
    if (!wh.isRecording() && !wh.isProcessing()) {
      preVoiceText = inputEl.value;
      inputEl.value = prepareBaseText(preVoiceText); updateInputClearButton();
    }
    await wh.toggle();
  }
});
confirmBtn.addEventListener('click', () => { void voice.confirm(); });
cancelBtn.addEventListener('click', () => { inputEl.value = preVoiceText; autoExpand(); voice.cancel(); });

// Host helpers (many-ai-cli's existing code, unchanged):
//   setRecordingLook(on)  toggles the voice button's `recording` class and tooltip
//   setProcessingLook(on) toggles the voice bar's `voice-processing` class
// browser engine -> existing UI
rec.on('start', () => { set_voiceActive(true); setRecordingLook(true); showVoiceBar();
  document.dispatchEvent(new CustomEvent('voiceinput:started')); });
rec.on('stop', () => { set_voiceActive(false); setRecordingLook(false); hideVoiceBar();
  setTimeout(() => inputEl.focus(), 0); document.dispatchEvent(new CustomEvent('voiceinput:stopped')); });
rec.on('audioActive', (a) => { set_voiceAudioActive(a); document.dispatchEvent(new CustomEvent('voiceinput:statechanged')); });
rec.on('state', (s) => setProcessingLook(s.processing));
rec.on('activity', ({ kind }) => driveWaveform(kind));
rec.on('result', ({ transcript, isFinal }) => {     // do NOT filter by isCurrent (see below)
  inputEl.value = inputEl.value.slice(0, interimStart) + transcript;
  if (isFinal) {
    inputEl.value += ' '; interimStart = inputEl.value.length;
    const tp = getActiveTriggerPhrase();
    if (tp && activeSessionId !== null && textEndsWithTriggerPhrase(buildSendText(), tp)) {
      rec.requestStop(); doSend(activeSessionId); return;
    }
  }
  autoExpand(); updateSlashMenu();
});
rec.on('error', (e) => { if (e.notify) showVoiceError(classifyVoiceErrorForDisplay(e.code), e.code); });
rec.on('diagnostic', ({ status, message }) => renderDiagnosticStatus(status, message));
diagRunBtn.addEventListener('click', () => rec.diagnostics.run());
diagCopyBtn.addEventListener('click', () => navigator.clipboard.writeText(rec.diagnostics.getReportJson()));
window.__anyAiCliVoiceDiagnostics = { ...rec.diagnostics, copy: () => diagCopyBtn.click() };

// whisper engine -> existing UI
wh.on('start', () => { set_voiceActive(true); setRecordingLook(true); setProcessingLook(false); showVoiceBar();
  document.dispatchEvent(new CustomEvent('voiceinput:started')); });
wh.on('processing', () => { setProcessingLook(true); set_voiceActive(true); setRecordingLook(true); });
wh.on('audioActive', (a) => { set_voiceAudioActive(a); document.dispatchEvent(new CustomEvent('voiceinput:statechanged')); });
wh.on('notice', () => showToast(t('voice_whisper_processing'), btn, 3000));
wh.on('result', ({ text }) => { inputEl.value = joinTranscript(preVoiceText, text); autoExpand();
  updateInputClearButton(); updateSlashMenu(); maybeAutoSubmit(); });
wh.on('error', (e) => {
  if (e.cancelled) showToast(t('voice_whisper_cancelled'), btn, 2000); else showWhisperError(e.code);
  inputEl.value = preVoiceText; autoExpand();       // both start and finish failures restored the field
});
wh.on('stop', () => { setProcessingLook(false); hideVoiceBar(); set_voiceActive(false);
  setRecordingLook(false); document.dispatchEvent(new CustomEvent('voiceinput:stopped')); });
```

## Contract notes (read before wiring a host)

1. **Late results after confirm.** After `stop()` (confirm) the instance is replaced, but
   Chrome may still deliver the final result of the previous instance. It arrives with
   `isCurrent: false`. many-ai-cli wrote such late results into the input, so a host that wants
   identical behaviour must **not** drop results by `isCurrent`.
2. **Late end/error of an old instance.** A late `end` or `error` from an old instance runs
   the same finish path, and can stop a recording that was started again in the meantime.
   This is inherited from `voice.ts` and has not been verified in a real browser. Hosts can
   tell which instance an event came from by `recognitionId`.
3. **Hotword is disabled in many-ai-cli.** `voice.ts` returns early (`return;` at line 649)
   before the wake-word block. To keep today's behaviour, many-ai-cli should not create a
   hotword listener (do not pass `hotword` / do not call `createHotwordListener`).
4. **`appLangToRecognitionLang`** mirrors many-ai-cli's app-language mapping (`ja` → `ja-JP`,
   `vi` → `vi-VN`, else `en-US`, empty → `ja`). Other hosts should pass their own `lang`.
5. **Version.** `package.json` says `0.0.1`, which is already taken on npm by the name
   reservation. It must be bumped before any publish.
6. **Empty transcripts are passed through.** The browser engine emits results exactly as Chrome
   delivers them, including an empty final transcript during silence. Filter them in the host
   if you do not want them.
7. **Whisper cancel during transcription reports twice.** `cancel()` emits `stop {reason:'cancel'}`
   at once. The aborted request then emits `error {cancelled:true}` and a second
   `stop {reason:'cancel'}`, as `voice-whisper.ts` did (it dispatched `voiceinput:stopped` twice).
8. **Whisper worklet.** The recorder worklet module is not shipped here. Pass
   `recorderWorklet` to use the AudioWorklet path; otherwise the ScriptProcessor fallback of
   the source is used.
