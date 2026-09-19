// vtype spike: spike/mic-permission/content.js
//
// This content script must NEVER call SpeechRecognition or getUserMedia
// itself. It only (1) shows a small on-page overlay so a human tester can
// drive the spike without opening devtools, and (2) relays start/stop
// intent to background.js, displaying whatever comes back.
//
// Messages reach this content script only via chrome.tabs.sendMessage from
// background.js (chrome.runtime.sendMessage sent by extension pages is
// delivered to extension pages, not to content scripts). The type-prefix and
// requestId filter below is defensive: it drops replies that belong to an
// earlier Start in this same tab.

(() => {
  const NS = 'vtype-spike';
  let activeRequestId = null;

  const panel = document.createElement('div');
  panel.style.cssText = [
    'position:fixed',
    'bottom:16px',
    'right:16px',
    'z-index:2147483647',
    'background:#111',
    'color:#fff',
    'font:12px/1.4 monospace',
    'padding:10px 12px',
    'border-radius:8px',
    'max-width:320px',
    'box-shadow:0 2px 8px rgba(0,0,0,.4)'
  ].join(';');
  panel.innerHTML =
    '<div style="margin-bottom:6px;font-weight:bold;">vtype mic-permission spike</div>' +
    `<button id="${NS}-start" type="button" style="margin-right:6px;">Start</button>` +
    `<button id="${NS}-stop" type="button" disabled>Stop</button>` +
    `<div id="${NS}-status" style="margin-top:6px;white-space:pre-wrap;max-height:240px;overflow:auto;">idle (origin: ${location.origin})</div>`;

  function mount() {
    document.documentElement.appendChild(panel);
  }
  if (document.documentElement) {
    mount();
  } else {
    document.addEventListener('DOMContentLoaded', mount, { once: true });
  }

  const statusEl = panel.querySelector(`#${NS}-status`);
  const startBtn = panel.querySelector(`#${NS}-start`);
  const stopBtn = panel.querySelector(`#${NS}-stop`);

  // Append-only trail (not overwrite): an error followed by 'recognition
  // ended' must both stay visible. Each line carries ms since Start.
  let startedAt = Date.now();
  const lines = [];
  function setStatus(text) {
    lines.push(`+${Date.now() - startedAt}ms ${text}`);
    if (lines.length > 30) lines.shift();
    statusEl.textContent = lines.join('\n');
    statusEl.scrollTop = statusEl.scrollHeight;
    // Also surfaced in the console, in case the overlay is obscured.
    console.log('[vtype-spike]', text);
  }

  startBtn.addEventListener('click', () => {
    startedAt = Date.now();
    lines.length = 0;
    activeRequestId = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    startBtn.disabled = true;
    stopBtn.disabled = false;
    setStatus('requesting offscreen recognition...');
    chrome.runtime.sendMessage({
      type: 'vtype-spike/content-to-bg-start',
      requestId: activeRequestId
    });
  });

  stopBtn.addEventListener('click', () => {
    chrome.runtime.sendMessage({
      type: 'vtype-spike/content-to-bg-stop',
      requestId: activeRequestId
    });
    startBtn.disabled = false;
    stopBtn.disabled = true;
  });

  chrome.runtime.onMessage.addListener((message) => {
    if (!message || typeof message.type !== 'string') return;
    if (!message.type.startsWith('vtype-spike/bg-to-content-')) return;
    if (message.requestId !== activeRequestId) return;

    if (message.type === 'vtype-spike/bg-to-content-result') {
      setStatus(
        `${message.isFinal ? '[final]' : '[interim]'} ${message.transcript}`
      );
    } else if (message.type === 'vtype-spike/bg-to-content-diag') {
      setStatus(message.text);
    } else if (message.type === 'vtype-spike/bg-to-content-error') {
      setStatus(`ERROR: ${message.error}`);
      startBtn.disabled = false;
      stopBtn.disabled = true;
    } else if (message.type === 'vtype-spike/bg-to-content-end') {
      setStatus('recognition ended');
      startBtn.disabled = false;
      stopBtn.disabled = true;
    }
  });
})();
