# vtype privacy policy

Last updated: 2026-09-22

**What you say is sent, for speech recognition, by the browser's speech recognition to Google's
speech recognition service, which turns it into text (this is how speech recognition works in
Chrome; see "Where your voice goes" below).**

Apart from that, the developer of vtype receives no information about you. Personal information is
never sold, and never passed to a third party for anything other than the speech recognition.
There is no account to create.

## What is not collected

The developer receives none of the following.

- Your name, email address or any other personally identifying information
- Your browsing history or the URLs of the pages you open
- The contents of pages
- What you type into text fields
- What you say, or the transcript of it
- Cookies or credentials
- Usage analytics

vtype contains no analytics code, and the developer runs no server for it.

## Where your voice goes

vtype does not do the recognition itself. What you say is handed to the browser's built-in speech
recognition (the Web Speech API).

**In Chrome, that recognition works by the browser sending the audio to Google's speech
recognition service.** When you start a recording in vtype, Chrome sends what you say to that
service and the recognised text comes back to vtype. The browser does the sending; vtype adds no
other destination to that path and keeps no copy of the audio or the text.

The handling of the audio sent to Google is covered by Google's (Chrome's) privacy policy. If you
do not want what you say to reach Google's recognition service, do not use vtype. The microphone
permission page that opens right after installation says this too, above the button that grants
the permission.

## Network

Apart from the speech recognition above, vtype sends no data to any server, the developer's
included; the developer runs no server. Nothing in the package is loaded from somewhere else;
all of its code ships inside the extension.

## What is stored in your browser

Settings are kept in `chrome.storage.sync`, which Chrome itself synchronises between the
browsers you are signed in to.

- `trigger`: what starts a recording (press the mic, or rest the mouse on it)
- `micDisplay`: which fields carry a mic (every visible field, or only the one you are using)
- `micOffsets`: where you dragged the mic to, one entry per site origin, at most 50
- `excludedSites`: the sites vtype stays off on, at most 100
- `diagnostics`: whether the diagnostic log is kept (off by default)
- `inputMode`: the input mode (normal / English / katakana)
- `desktopBridge`: whether the link to the desktop app is on (off by default)

The replacement table (`replacements`: pairs of "when A is recognised, write B" that you entered
yourself) is kept in `chrome.storage.local`. Whether the desktop app is connected, and a copy of
its settings (`desktopBridgeStatus` / `nativeConfig`), are kept in `chrome.storage.session` and
are gone when the browser closes.

`excludedSites` is also written to `chrome.storage.local`. Some enterprise policies refuse sync
outright, and "the mic came back on the site I switched it off on" is not an acceptable outcome
of that.

`micOffsets` and `excludedSites` contain origins of sites you chose yourself (in the form
`https://example.com`). They stay in your browser and are never sent to the developer.

## The diagnostic log

**It is off unless you switch it on** on the settings page. It exists for one question — why did
a recording stop where it did — and it records:

- the time
- the number of the recognition attempt (Chrome restarts recognition between utterances)
- the kind of event (started / speech detected / a result arrived / ended)
- **how many characters** a result contained
- why the recording ended (you stopped it / silence / an error and its code)

**Neither what you said nor the recognised text is recorded** — only how many characters there
were. The log lives in `chrome.storage.local` (inside this browser), holds at most 300 entries
with the oldest dropped first, and the settings page shows, copies and empties it at any time.
**It is never sent to the developer.**

## Transcripts

Nothing is kept. Recognised text goes into the field you are typing in and that is the end of
it: there is no history, no search and no export. While the diagnostic log is on, what it keeps
is the number of characters, never the characters.

## Password fields

vtype does not work in password fields and shows no mic on them. The input types it works on are
an allowlist, and password is not in it. Read-only and disabled fields are excluded too.

## Permissions

- `offscreen`: runs the speech recognition in the extension's own page. That is what makes the
  microphone something you allow once, for the extension, instead of a permission dialog on
  every site
- `storage`: keeps the settings listed above
- Access to all sites (the content script's `http://*/*` and `https://*/*`): text fields exist on
  every kind of site, so the mic has to be able to appear on any of them. vtype requests neither
  host permissions nor the `tabs` permission, so the extension's background never learns which
  site a tab is on
- `nativeMessaging` (optional; not held at first): granted only when you press "Turn on the
  desktop link" on the settings page and allow it. It is used only to hand recognised text to the
  desktop app on the same computer, and talks to no other program

## The desktop app (Windows / macOS / Linux)

The desktop app is a separate program that types what the extension recognised into apps outside
the browser. Installing it is optional; the extension works the same without it.

- Chrome still does the speech recognition (see "Where your voice goes" above). The desktop app
  does not use the microphone and never handles audio
- The desktop app sends nothing over the network. The only things it talks to are Chrome on the
  same computer (Native Messaging) and its own commands run on the same computer (`vtype toggle`
  and the like)
- Recognised text goes into the app in front and that is the end of it; nothing is kept. If you
  choose the "paste" method in the settings, or on Linux under Wayland when direct typing is not
  possible, the text is put on the clipboard (the paste method puts the clipboard's previous
  contents back afterwards)
- It types nothing into password fields (on Windows and macOS it asks the system what kind of
  field it is; on Linux it leaves out the ones the system's accessibility service can identify)
- On your computer it keeps a settings file (`config.json`: the shortcut, the input mode, whether
  the icon is shown, and so on) and a log (`vtype.log`: at most 1 MB, two generations; times,
  kinds of events and errors only, never what you said or the text). Neither is sent to the
  developer

## Reporting a problem

"Report a problem", on the extension's settings page and in the desktop app's menu, only opens
GitHub's new-issue form in your browser with the version, the system and the browser's name filled
in. Nothing is sent automatically: you look at the form and decide whether to submit it. "Copy
diagnostic info" in the desktop app's menu copies the system, version, settings and recent errors
to the clipboard, and never includes audio or transcripts.

## Changes

Changes to this policy are announced by updating this document.

## Contact

https://github.com/ishizakahiroshi/vtype/issues
