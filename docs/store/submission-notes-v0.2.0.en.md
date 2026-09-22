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
- A link to the desktop app through the optional `nativeMessaging` permission (**off by
  default**). The permission is requested only when the user presses "Turn on the desktop link"
  on the settings page; if it is refused, nothing changes. The only counterpart is the vtype
  desktop app the user installed on the same computer (Native Messaging host
  `com.ishizakahiroshi.vtype`): the extension hands it the recognised text, and the desktop app
  types it into apps outside the browser. The desktop app sends nothing over the network either
- Reporting a problem: "Open the report form" on the settings page opens GitHub's new-issue form
  in a new tab with the version, OS and browser name filled in. Nothing is sent automatically

`nativeMessaging` is optional rather than required so that the update does not disable the
extension for existing users while it waits for them to accept a new permission.

## Single purpose

Entering text into text fields on web pages by voice. Only when the user has installed the desktop
app and turned the link on, the same dictation result is also handed to text fields in apps
outside the browser.

## Permissions

The text for `offscreen`, `storage` and the content script's `http://*/*` and `https://*/*` is the
same as in v0.1.0. Added:

- `nativeMessaging` (`optional_permissions`): hands the recognised text to the vtype desktop app
  that the user installed on their own computer. It is optional, not held by default, and granted
  only when the user presses the button on the settings page and allows it. It connects to one
  host only, `com.ishizakahiroshi.vtype`, and to no other program. The desktop app sends nothing
  over the network and keeps none of the text it receives
- `storage` now also keeps the input mode (`inputMode`), the replacement table (`replacements`, in
  `chrome.storage.local`) and whether the link is on (`desktopBridge`). The link's status is kept
  in `chrome.storage.session` and is gone when the browser closes

## Data handling

In addition to v0.1.0:

- Handing text to the desktop app: only with the link turned on, the recognised text goes to the
  desktop app on the same computer over Native Messaging. It does not leave the computer. The
  desktop app keeps nothing once the text is typed into the app in front
- The replacement table: word pairs the user entered. Stored only in the browser
  (`chrome.storage.local`)
- Reporting a problem: only opens GitHub's form in the browser. The user submits it on that page;
  the extension sends nothing

## What to test

In addition to 1–7 of v0.1.0:

1. Set the input mode to "Katakana" on the settings page and speak: the field receives full-width
   katakana. The mode stays until changed, also after restarting the browser
2. Add a row to the replacement table: it applies from the next recognised text
3. Permissions: right after installation the extension does not hold `nativeMessaging` (see its
   details in chrome://extensions). Pressing "Turn on the desktop link" on the settings page is the
   only time Chrome asks for it. Refusing leaves the link off and everything else unchanged
4. Turning the link on without the desktop app installed only shows that it was not found;
   recording and inserting into fields keep working

## Privacy policy URL

https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md
