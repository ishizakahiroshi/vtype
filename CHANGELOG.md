# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). The version in
`packages/extension/manifest.json` is the single source of the number; everything else derives
from it. Releasing turns `## [Unreleased]` into `## [X.Y.Z] - YYYY-MM-DD` and opens a fresh
`## [Unreleased]` above it.

## [Unreleased]

### Added

- Input modes: normal, English (recognition in en-US) and katakana (recognised text turned into
  full-width katakana with a dictionary shipped in the package). The mode stays until changed,
  also across restarts. A replacement table ("write B for A") applies after recognition. The
  conversion lives in `vtype-core`, so the extension and the desktop app share it.
- vtype desktop (`packages/native`, 0.1.0, not published yet): one Rust program for Windows,
  macOS and Linux for voice input into any app, without the extension. It starts Google Chrome
  (which must be installed) in a profile of its own, off screen, on a speech page it serves on
  127.0.0.1, and types what Chrome recognises into the app in front. Before the first recording it
  asks for consent in a small Chrome window, then allows the microphone in that profile itself.
  Its own settings page (`vtype settings`) edits the shortcut, the input mode, the replacement
  table and how text is typed. A global shortcut
  (Ctrl+Alt+Space; Control+Option+V on macOS), a tray or menu bar icon, a floating mic icon, the
  in-progress text above it, commands (`vtype toggle` / `start` / `stop` / `mode` / `status`) for
  launchers and AutoHotkey, no typing into password fields, and an experimental mic beside the
  text field in use (Windows and macOS, off by default). On Linux: X11 through XTest, Wayland
  through the remote desktop portal, then `ydotool`, then the clipboard.
- Templates in the desktop app: the button at the top left of the floating mic lists them, and
  choosing one puts it into the app in front. Each row has ✏ and ✕ at its right end: ✏ opens that
  template on the settings page, ✕ deletes it at once and leaves "Undo" in its row, with the list
  still open.
- Words said in the desktop app while the system says no text field has the focus are not typed
  into whatever is in front: they wait in a bubble above the floating mic, with "Copy", "Insert"
  and ✕, held in memory only (never written to disk) until one of them is pressed. When the system
  cannot tell, when the floating mic is hidden, and on Wayland, words are typed as before.
- Starting the desktop app at sign-in is a choice: the first-run screen has "Start vtype when you
  sign in", ticked by default, and the same checkbox at the top of the desktop settings on the
  settings page switches it later. On the Microsoft Store build, once Windows or a policy has
  switched its startup task, the checkbox is locked and points to Windows Settings > Apps >
  Startup. The `.deb` still starts vtype for every user; each user can switch it off (a
  `Hidden=true` entry in their own `~/.config/autostart`). `vtype install` / `vtype uninstall`
  still do the same from the command line.
- Each time the desktop app starts, a sign-in entry that starts another copy of vtype (moved, or
  installed again elsewhere) is pointed at the copy that is running: the Run key on Windows, the
  LaunchAgent on macOS, `~/.config/autostart/vtype.desktop` on Linux. It adds no entry, and the
  Store build is left alone (its startup task always starts its own copy).
- The settings page has one line and a link about the desktop app, which is a separate app: the
  extension does not talk to it and asks for no new permission.
- "Report a problem" on the settings page and in the desktop app's menu: opens GitHub's issue
  form with the version, OS and browser filled in, and sends nothing itself. A bug report issue
  template goes with it. "Copy diagnostic info" in the desktop app's menu never includes audio or
  transcripts.
- "About vtype" at the end of the settings page, and in the desktop app's tray menu: the version,
  where the voice goes (the words shown before the first recording) with a link to the privacy
  policy, the developer's website, the source, the MIT license and the licenses of the open-source
  software inside. The desktop app opens these links in the usual browser, not in its settings
  window's own Chrome profile.
- Release tooling for the desktop app (nothing is published by it): a workflow that builds all
  three systems on a `native-v*` tag into a draft GitHub Release (zip, universal macOS tar.gz,
  `.deb`, unsigned MSIX, third-party notices, checksums, Homebrew formula), and the npm package
  layout `@ishizakahiroshi/vtype`.

### Changed

- The privacy policy and the store texts describe the desktop app (its own Chrome profile, the
  consent before the first recording) and the new settings. The privacy policy also says what else
  the desktop app keeps or writes: words waiting in a bubble (in memory only), the sign-in entry it
  switches, and the app names in its log. The store listing itself is updated
  only with v0.2.0.
- The packaging check accepts `fetch(chrome.runtime.getURL(...))`, which can only read files
  inside the package (the katakana dictionary); every other network call still fails it.

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
