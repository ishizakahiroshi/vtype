// vtype spike: spike/mic-permission/background.js
//
// Message flow implemented here (see README.md for the full picture):
//   content.js  --[chrome.runtime.sendMessage]-->  background.js (this file)
//   background.js --[chrome.runtime.sendMessage]--> offscreen.js
//   offscreen.js --[chrome.runtime.sendMessage]--> background.js (this file)
//   background.js --[chrome.tabs.sendMessage(tabId, ...)]--> content.js
//
// background.js never touches SpeechRecognition or getUserMedia itself. Its
// only jobs are: (1) create the offscreen document on demand, and (2) relay
// messages between a specific tab's content script and the offscreen
// document, keeping track of which tab a recognition session belongs to
// (chrome.tabs.sendMessage needs an explicit tabId; chrome.runtime.sendMessage
// has no notion of "the tab that asked").

const OFFSCREEN_URL = chrome.runtime.getURL('offscreen.html');
let creatingOffscreen = null;

// Returns a short label for the diagnostic trail: 'reused' / 'created' / 'awaited'.
async function ensureOffscreenDocument() {
  const has = await chrome.offscreen.hasDocument();
  if (has) return 'reused';

  if (creatingOffscreen) {
    await creatingOffscreen;
    return 'awaited';
  }

  creatingOffscreen = chrome.offscreen.createDocument({
    url: OFFSCREEN_URL,
    reasons: ['USER_MEDIA'],
    justification:
      'Run Web Speech API (webkitSpeechRecognition) recognition for the ' +
      'vtype mic-permission spike.'
  });

  try {
    await creatingOffscreen;
  } finally {
    creatingOffscreen = null;
  }
  return 'created';
}

// Clicking the toolbar icon opens the one-time, extension-origin page that
// requests getUserMedia so the browser's mic-permission prompt is shown for
// the extension's own chrome-extension:// origin. See permission.js.
chrome.action.onClicked.addListener(() => {
  chrome.tabs.create({ url: chrome.runtime.getURL('permission.html') });
});

chrome.runtime.onMessage.addListener((message, sender) => {
  if (!message || typeof message.type !== 'string') return;

  if (message.type === 'vtype-spike/content-to-bg-start') {
    const tabId = sender.tab && sender.tab.id;
    if (tabId == null) return;

    ensureOffscreenDocument()
      .then((how) => {
        chrome.tabs.sendMessage(tabId, {
          type: 'vtype-spike/bg-to-content-diag',
          requestId: message.requestId,
          text: `offscreen document: ${how}`
        });
        chrome.runtime.sendMessage({
          type: 'vtype-spike/bg-to-offscreen-start',
          requestId: message.requestId,
          tabId
        });
      })
      .catch((err) => {
        chrome.tabs.sendMessage(tabId, {
          type: 'vtype-spike/bg-to-content-error',
          requestId: message.requestId,
          error: `offscreen document could not be created: ${
            err && err.message ? err.message : err
          }`
        });
      });
    return;
  }

  if (message.type === 'vtype-spike/content-to-bg-stop') {
    chrome.runtime.sendMessage({
      type: 'vtype-spike/bg-to-offscreen-stop',
      requestId: message.requestId
    });
    return;
  }

  if (message.type === 'vtype-spike/offscreen-to-bg-result') {
    chrome.tabs.sendMessage(message.tabId, {
      type: 'vtype-spike/bg-to-content-result',
      requestId: message.requestId,
      transcript: message.transcript,
      isFinal: message.isFinal
    });
    return;
  }

  if (message.type === 'vtype-spike/offscreen-to-bg-error') {
    chrome.tabs.sendMessage(message.tabId, {
      type: 'vtype-spike/bg-to-content-error',
      requestId: message.requestId,
      error: message.error
    });
    return;
  }

  if (message.type === 'vtype-spike/offscreen-to-bg-diag') {
    chrome.tabs.sendMessage(message.tabId, {
      type: 'vtype-spike/bg-to-content-diag',
      requestId: message.requestId,
      text: message.text
    });
    return;
  }

  if (message.type === 'vtype-spike/offscreen-to-bg-end') {
    chrome.tabs.sendMessage(message.tabId, {
      type: 'vtype-spike/bg-to-content-end',
      requestId: message.requestId
    });
    return;
  }
  // Anything else (e.g. this same broadcast bouncing back) is ignored.
});
