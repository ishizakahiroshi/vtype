# Chrome Web Store submission notes (v0.1.0, English)

The text for the dashboard's "single purpose", "permission justification" and "data usage"
fields, plus notes for the reviewer. One file per version; never overwrite the previous one.

## Reviewer notes

First release. The extension puts spoken words into the text fields of web pages, using the
browser's built-in Web Speech API for the recognition. In Chrome the browser recognises speech by
sending the audio to Google's speech recognition service, so what the user says reaches Google;
this is stated before the user grants the microphone, in the store listing, the privacy policy and
the permission page that opens right after installation. There is no other connection and no
developer server.

In this version:

- A faint mic next to every text field on screen (a setting narrows it to the field in use)
- Press and speak: interim words are inserted at the caret and corrected in place while you talk
- A panel to clear the field or submit the page
- A waveform while recording
- The mic can be dragged aside; its position is remembered per site
- The toolbar icon switches the extension off for the current site; the settings page lists those sites
- English and Japanese (`_locales/en` and `_locales/ja`, with `en` as `default_locale`). The
  manifest's `name`, `description` and `action.default_title` are `__MSG_*`, so what you see
  follows your own browser language
- A diagnostic log, **off by default**. Switched on from the settings page, it records the time,
  how often recognition restarted, the kind of event, **how many characters** a result had and
  why a recording ended, in `chrome.storage.local`, at most 300 entries. **The recognised text is
  not recorded.** The settings page shows, copies and empties it; nothing is sent anywhere

## Single purpose

Entering text into web page text fields by voice. That is the only thing it does.

## Permissions

- `offscreen`: needed to run the speech recognition in the extension's own offscreen document.
  That is what makes the microphone a single, one-time permission for the extension; opening the
  microphone from a content script would ask for permission again on every origin the user visits
- `storage`: needed for the settings (what starts a recording, which fields show a mic, dragged
  mic positions, the list of sites the extension is switched off on). They live in
  `chrome.storage.sync`; the switched-off list is additionally written to `chrome.storage.local`,
  because some enterprise policies refuse sync and the mic must not come back on a site the user
  turned it off on
- The content script's `http://*/*` and `https://*/*` with `all_frames`: text fields exist on
  every kind of site, so this cannot be narrowed to a list of hosts, and fields inside iframes
  need the script in that frame. **No `host_permissions` and no `tabs` permission are requested**,
  so the extension's background never learns which site a tab is on; the per-site decision is made
  by the content script from its own origin
- Reading page content: used only to find text fields and insert at the caret. No page content is
  collected or transmitted
- Remote code: none. Everything the extension runs ships inside the package

## Data handling

- Personally identifiable information: not collected
- Browsing history or page content: not collected
- What the user types: not collected
- Sent to the developer: nothing (there is no developer server)
- Analytics: none
- Where the audio goes: vtype does not perform recognition; it hands the audio to the browser's
  Web Speech API. In Chrome, the browser performs the recognition by sending the audio to
  Google's speech recognition service. That is the browser's behaviour, not a connection vtype
  makes, and vtype adds no destination to it. The privacy policy states the same
- Stored settings: `trigger`, `micDisplay`, `micOffsets` (per origin, at most 50),
  `excludedSites` (at most 100) and `diagnostics` (the log's on/off, off by default), kept in the
  browser (`chrome.storage.sync`, plus `chrome.storage.local` for the excluded list)
- Diagnostic log: off by default. When on, at most 300 entries in `chrome.storage.local`, holding
  the time, the recognition attempt number, the kind of event, a result's **character count** and
  why the recording ended. **It contains no recognised text**, the user can empty it at any time,
  and it is never sent to the developer
- Transcripts: not stored. Once the text is in the field, nothing is kept
- Password fields: excluded. The input types it works on are an allowlist that does not include
  password, and fields marked `autocomplete="current-password"` / `"new-password"` are excluded too

## What to test

1. On any site, a faint mic appears just outside the right edge of a search box
2. Pressing it starts recording, and the spoken words appear in that field at the caret
3. Opening three sites of different origins produces no microphone permission dialog at all
   (the microphone is allowed once, on the page that opens right after installation)
4. Password fields get no mic
5. Pressing the toolbar icon stops the mic appearing on that site, and the settings page
   (chrome://extensions, Details, Extension options) lists it
6. Switching the settings page to "only the field I am using" takes effect on an already open
   page, without a reload
7. "Keep a diagnostic log" on the settings page is off by default. With it on, a recording fills
   the log on that same page, and no line of it contains anything that was said (only counts)

## Privacy policy URL

https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md
