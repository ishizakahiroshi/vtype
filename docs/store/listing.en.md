# Chrome Web Store listing (English)

Written to be pasted straight into the dashboard fields. The table at the end says which heading
goes in which field.

## Extension name

vtype

## Short description

Speak into any text field on the web. It uses the browser's own speech recognition: no account, no quota, no minute limit.

## Detailed description

vtype puts your voice into the text fields of web pages. Every text field you can see carries a
faint little mic; press it and speak, and the words appear in that field at the caret while you
are still talking. A search box, a contact form, a rich-text message body: if you can type in it,
this works in it.

The recognition is the browser's own. There is no API key, no account and no cap on how many
minutes you may use per day. The absence of that cap is the point of the extension rather than a
missing feature.

The microphone is allowed once, on a page that opens right after you install it. Recognition runs
in the extension's own page, so no site you visit ever shows you a permission dialog.

What it does:

- A faint mic next to every text field on screen (a setting narrows this to the field you are using)
- Press it and speak: the words land in the field as you talk, at the caret, leaving the text
  before and after it intact
- Resting the mouse on the mic opens a small panel: clear the field, or submit the page
- A blue waveform moves while recording, so you can see that you are being heard
- Writing waits while a Japanese IME composition is in progress
- If the mic lands on a button of the site's own, drag it aside; where you put it is remembered
  for that site
- The toolbar icon switches vtype off for the site you are on; the settings page lists what you
  switched off
- No mic on password fields, and none on read-only or disabled fields

Privacy:

- Nothing is sent to the developer. vtype itself contacts no server at all
- No personal information, browsing history, page content or typed input is collected
- No transcript history is kept. The words go into the field and that is the end of it
- The recognition is the browser's. In Chrome the browser performs it by sending the audio to
  Google's speech recognition service; that is the browser's behaviour, not a connection vtype makes
- Settings (what starts recording, which fields show a mic, dragged positions, excluded sites)
  are kept inside your browser

Permissions:

- offscreen: runs the speech recognition in the extension's own page, which is what makes the
  microphone a one-time permission for the extension instead of a dialog on every site
- storage: keeps the settings inside your browser
- Access to all sites: text fields exist on every kind of site, so the mic has to be able to
  appear on any of them. No host permissions and no `tabs` permission are requested

## Category

Productivity

## Languages

English / Japanese

The extension itself ships both (`packages/extension/_locales/` is the source; the manifest's name
and description follow the reader's language through `__MSG_*`). The dashboard takes one listing
per language: paste this file for English and `listing.ja.md` for Japanese. A new language gets its
own listing file here.

## Screenshot plan

Four images at 1280x800. **No other company's product may appear in any of them** (a certain
problem in review). Shoot them on a purpose-made demo page.

1. A single-line field (a search box) with the faint mic beside it
2. The panel open under the mic (clear / send / mic)
3. Recording: the mic red, the blue waveform moving, words appearing in the field
4. The settings page (what starts recording, which fields show a mic, the excluded-site list)

## Dashboard field mapping

| Dashboard field | What to paste |
|---|---|
| Name | "Extension name" |
| Summary (132 chars) | "Short description" |
| Description | "Detailed description" |
| Category | "Category" |
| Language | "Languages" |
| Screenshots | the four from "Screenshot plan" |
| Privacy policy URL | https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md |
| Single purpose | "Single purpose" in `submission-notes-vX.Y.Z.en.md` |
| Permission justification | "Permissions" in `submission-notes-vX.Y.Z.en.md` |
| Data usage disclosure | "Data handling" in `submission-notes-vX.Y.Z.en.md` |
