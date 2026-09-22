# Chrome Web Store submission notes (v0.2.0, English, draft)

Text for the dashboard's "Single purpose", "Permission justification" and "Data usage" fields,
and a note for the reviewer. A draft for the v0.2.0 submission: **neither the listing nor this
text is submitted until the v0.1.0 review is over.** Fields that do not change (the first half
of the single purpose, the existing permissions) reuse the v0.1.0 text.

## Reviewer notes

Changes since v0.1.0:

- Input modes (normal / English / katakana) and a replacement table. Recognised text is turned
  into katakana with a dictionary shipped inside the package, and the "write B for A" pairs the
  user entered are applied. Both happen inside the browser only
- Reporting a problem: "Open the report form" on the settings page opens GitHub's new-issue form
  in a new tab with the version, OS and browser name filled in. Nothing is sent automatically

No permission is added since v0.1.0 (and there are no `optional_permissions`). The settings page
has one line and a link about the desktop app, which is a separate app; the extension does not
talk to it.

## Single purpose

Entering text into text fields on web pages by voice.

## Permissions

The text for `offscreen`, `storage` and the content script's `http://*/*` and `https://*/*` is the
same as in v0.1.0. No permission is added; only the `storage` text gains:

- `storage` now also keeps the input mode (`inputMode`) and the replacement table
  (`replacements`, in `chrome.storage.local`)

## Data handling

In addition to v0.1.0:

- The replacement table: word pairs the user entered. Stored only in the browser
  (`chrome.storage.local`)
- Reporting a problem: only opens GitHub's form in the browser. The user submits it on that page;
  the extension sends nothing

## What to test

In addition to 1–7 of v0.1.0:

1. Set the input mode to "Katakana" on the settings page and speak: the field receives full-width
   katakana. The mode stays until changed, also after restarting the browser
2. Add a row to the replacement table: it applies from the next recognised text
3. Permissions: in chrome://extensions the details show the same permissions as v0.1.0 (and no
   optional ones)

## Privacy policy URL

https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md
