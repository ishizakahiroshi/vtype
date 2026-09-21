# vtype privacy policy

Last updated: 2026-09-21

vtype does not collect, store, sell or share personal information. There is no account to create.

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
recognition service.** So what you say reaches Chrome and that service. This is the browser's
own behaviour rather than a connection vtype makes; vtype neither adds a destination to that
path nor keeps a copy of anything on it.

The handling of that audio is covered by the browser's own privacy policy. If you do not want
what you say to reach an external recognition service, do not use vtype.

## Network

vtype itself contacts no server at all. Nothing in the package is loaded from somewhere else;
all of its code ships inside the extension.

## What is stored in your browser

Settings are kept in `chrome.storage.sync`, which Chrome itself synchronises between the
browsers you are signed in to.

- `trigger`: what starts a recording (press the mic, or rest the mouse on it)
- `micDisplay`: which fields carry a mic (every visible field, or only the one you are using)
- `micOffsets`: where you dragged the mic to, one entry per site origin, at most 50
- `excludedSites`: the sites vtype stays off on, at most 100

`excludedSites` is also written to `chrome.storage.local`. Some enterprise policies refuse sync
outright, and "the mic came back on the site I switched it off on" is not an acceptable outcome
of that.

`micOffsets` and `excludedSites` contain origins of sites you chose yourself (in the form
`https://example.com`). They stay in your browser and are never sent to the developer.

## Transcripts

Nothing is kept. Recognised text goes into the field you are typing in and that is the end of
it: there is no history, no search and no export.

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

## Changes

Changes to this policy are announced by updating this document.

## Contact

https://github.com/ishizakahiroshi/vtype/issues
