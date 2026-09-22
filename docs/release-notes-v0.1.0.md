# vtype v0.1.0

The first Chrome Web Store release: voice input into any text field on a web page, with the
browser's built-in speech recognition, no API key, no account and no usage limit.

## Added

- A faint mic beside every visible text field (or only the one in use, as a setting). Words appear
  in the field itself, at the caret, while you speak, and pages built with React or a rich editor
  react as they would to typing.
- A panel under the mic: clear the field, submit the page, start or stop recording, with a waveform
  while recording.
- Recognition in the extension's offscreen document, so the microphone is allowed once for the
  extension and no site ever asks. In Chrome the browser sends the audio to Google's speech
  recognition service, and the permission page says so before the microphone is granted. Nothing
  is sent to the developer.
- Japanese IME support, per-site off switch on the toolbar icon, a draggable mic whose position is
  remembered per site, and a settings page.
- Password, read-only and disabled fields are excluded structurally.
- English and Japanese throughout, following the browser's language.
- An opt-in diagnostic log (off by default) that records timings and why a recording ended, never
  what was said.

See [CHANGELOG.md](../CHANGELOG.md) for the full list.

## Changed

- Nothing: there is no earlier published version.

## Fixed

- Nothing: there is no earlier published version.

## Package

- Chrome Web Store package: `vtype-v0.1.0-webstore.zip`
- SHA256: `6ff0ea6fbf6ad3c55e2cf3fd6d07a2cd072e7e726ef665e141ff791f204e4a6e`

## Integrity check

```powershell
Get-FileHash .\vtype-v0.1.0-webstore.zip -Algorithm SHA256
```
