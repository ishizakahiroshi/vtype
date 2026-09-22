# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). The version in
`packages/extension/manifest.json` is the single source of the number; everything else derives
from it. Releasing turns `## [Unreleased]` into `## [X.Y.Z] - YYYY-MM-DD` and opens a fresh
`## [Unreleased]` above it.

## [Unreleased]

### Added

-

### Changed

-

### Fixed

-

## [0.1.0] - 2026-09-22

The first Chrome Web Store release. Everything below is new, so nothing is listed as changed or
fixed against a published version.

### Added

- Voice input into any text field on a web page. Every visible text field carries a faint mic
  just outside its right edge; a setting narrows that to the field being used.
- Words appear in the field itself, at the caret, while you speak, and are corrected in place.
  Text before and after the caret is left alone, and pages built with React or a rich editor
  react as they would to typing.
- A panel under the mic with three buttons: clear the field, submit the page, start or stop
  recording. A blue waveform moves while recording.
- Recognition runs in the extension's offscreen document with the browser's built-in Web Speech
  API, so the microphone is allowed once for the extension and no site ever asks. In Chrome that
  API sends the audio to Google's speech recognition service; the microphone permission page says
  so above the button that grants the permission.
- The recording is held by the field it started in: switching fields or tabs does not move it,
  and starting one in a second tab stops the first.
- Japanese IME support: the field is not written to while a composition is in progress.
- The mic can be dragged aside when it lands on a button of the site's own, and where it was put
  is remembered per site (at most 50 origins).
- Per-site off switch on the toolbar icon, with the list editable on the settings page (at most
  100 entries, kept in sync and local storage).
- Settings page: what starts a recording (press or hover), which fields carry a mic (all visible
  or only the one in use), and a button that puts every dragged mic back.
- Password, read-only and disabled fields are excluded structurally, by an allowlist of input
  types rather than a denylist.
- English and Japanese throughout, following the browser's language: the store name and summary
  (through the manifest's `__MSG_*` fields), the settings page, the microphone page and every
  message in the panel. Adding a language is adding one `_locales/<code>/messages.json`; the
  build, the tests and the packaging check refuse a locale whose keys do not match.
- An opt-in diagnostic log, off by default, for the one question that cannot be answered
  otherwise: why did a recording stop where it did. It records timings, how often recognition
  restarted, how many characters each result carried and why a recording ended — never what was
  said — in this browser only, at most 300 entries, shown, copied and emptied from the settings
  page.
- `vtype-core`, the recognition engine as a workspace package that renders nothing, so the same
  engine can be used outside the extension.

### Changed

- Nothing: there is no earlier published version to change.

### Fixed

- Nothing: there is no earlier published version to fix.
