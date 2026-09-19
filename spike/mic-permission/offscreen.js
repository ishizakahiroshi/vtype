// vtype spike: spike/mic-permission/offscreen.js
//
// Runs inside the offscreen document (offscreen.html). This is the one
// place in the spike that is allowed to touch SpeechRecognition.
//
// Whether webkitSpeechRecognition even runs inside an offscreen document at
// all, and whether it needs a fresh permission prompt here, is exactly the
// unknown this spike exists to answer. Nothing is assumed: every failure
// mode (constructor missing, start() throwing, onerror firing) is reported
// back to background.js instead of being swallowed, so the tester can see
// it in the content-script overlay / devtools console.
//
// This file only supports one active recognition session at a time (single
// offscreen document, single `session`/`recognition` variable). If two
// different tabs both press Start before the first one stops, the second
// request will silently take over the session. That is a known limitation
// of this throwaway spike, not something worth solving here.

let recognition = null;
let session = null; // { tabId, requestId }

function send(type, extra) {
  if (!session) return;
  chrome.runtime.sendMessage(
    Object.assign(
      {
        type,
        tabId: session.tabId,
        requestId: session.requestId
      },
      extra || {}
    )
  );
}

// Diagnostic trail: every recognition event is reported, so the content
// panel can show the full sequence instead of only the last state.
function diag(text) {
  send('vtype-spike/offscreen-to-bg-diag', { text });
}

async function reportMicPermission() {
  try {
    const status = await navigator.permissions.query({ name: 'microphone' });
    diag(`mic permission (extension origin): ${status.state}`);
  } catch (err) {
    diag(`mic permission query failed: ${err && err.message ? err.message : err}`);
  }
}

function startRecognition() {
  const SpeechRecognitionCtor =
    window.SpeechRecognition || window.webkitSpeechRecognition;
  diag(
    `ctor: SpeechRecognition=${!!window.SpeechRecognition} ` +
      `webkitSpeechRecognition=${!!window.webkitSpeechRecognition} ` +
      `lang=${navigator.language}`
  );
  reportMicPermission();

  if (!SpeechRecognitionCtor) {
    send('vtype-spike/offscreen-to-bg-error', {
      error:
        'not-available: neither window.SpeechRecognition nor ' +
        'window.webkitSpeechRecognition exists in this offscreen document'
    });
    return;
  }

  try {
    recognition = new SpeechRecognitionCtor();
    recognition.continuous = true;
    recognition.interimResults = true;
    // Follow the browser UI language so a tester speaking their own language
    // gets readable transcripts (e.g. ja-JP on a Japanese Chrome).
    recognition.lang = navigator.language || 'en-US';

    recognition.onresult = (event) => {
      // Diagnostic: dump the raw shape of every changed result so an empty
      // transcript can be told apart from a missing one.
      for (let i = event.resultIndex; i < event.results.length; i++) {
        const r = event.results[i];
        const alt = r && r[0];
        diag(
          `result[${i}/${event.results.length}] final=${!!(r && r.isFinal)} ` +
            `alts=${r ? r.length : 'none'} ` +
            `transcript=${alt ? JSON.stringify(alt.transcript) : 'none'} ` +
            `conf=${alt ? alt.confidence : 'none'}`
        );
      }
      const result = event.results[event.results.length - 1];
      const transcript = result && result[0] ? result[0].transcript : '';
      send('vtype-spike/offscreen-to-bg-result', {
        transcript,
        isFinal: !!(result && result.isFinal)
      });
    };

    recognition.onerror = (event) => {
      send('vtype-spike/offscreen-to-bg-error', {
        error: `onerror: ${event.error}${event.message ? ` (${event.message})` : ''}`
      });
    };

    for (const name of [
      'start',
      'audiostart',
      'soundstart',
      'speechstart',
      'speechend',
      'soundend',
      'audioend',
      'nomatch'
    ]) {
      recognition.addEventListener(name, () => diag(`event: ${name}`));
    }

    recognition.onend = () => {
      send('vtype-spike/offscreen-to-bg-end', {});
    };

    recognition.start();
    diag('start() called');
  } catch (err) {
    send('vtype-spike/offscreen-to-bg-error', {
      error: `exception on start(): ${err && err.message ? err.message : err}`
    });
  }
}

function stopRecognition() {
  if (!recognition) return;
  try {
    recognition.stop();
  } catch (err) {
    send('vtype-spike/offscreen-to-bg-error', {
      error: `exception on stop(): ${err && err.message ? err.message : err}`
    });
  }
}

chrome.runtime.onMessage.addListener((message) => {
  if (!message || typeof message.type !== 'string') return;

  if (message.type === 'vtype-spike/bg-to-offscreen-start') {
    session = { tabId: message.tabId, requestId: message.requestId };
    startRecognition();
    return;
  }

  if (message.type === 'vtype-spike/bg-to-offscreen-stop') {
    stopRecognition();
    return;
  }
  // Any other message type is not addressed to the offscreen document
  // (see the messaging-quirk note in content.js) and is ignored.
});
