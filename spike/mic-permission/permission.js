// vtype spike: spike/mic-permission/permission.js
//
// Runs on the extension's own chrome-extension:// origin (permission.html,
// opened in a normal tab by background.js when the toolbar icon is
// clicked). Its only job is to make the browser show the microphone
// permission prompt once for THIS origin, on the theory that the offscreen
// document -- which shares the same chrome-extension:// origin -- can then
// reuse that grant without prompting again. This script never talks to
// background.js or offscreen.js; it is a one-shot, standalone permission
// bootstrap.

const statusEl = document.getElementById('status');
const button = document.getElementById('grant');

button.addEventListener('click', async () => {
  statusEl.textContent = 'requesting...';
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    stream.getTracks().forEach((track) => track.stop());
    statusEl.textContent = 'granted. You can close this tab.';
  } catch (err) {
    statusEl.textContent = `denied or failed: ${
      err && err.name ? err.name : err
    }`;
  }
});
