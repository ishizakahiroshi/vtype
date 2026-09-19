# vtype extension (Chrome, Manifest V3)

The browser extension. When a text field on a web page gets focus, a small mic appears just
outside its right edge; resting the mouse on the mic opens a panel with three buttons,
× / send / mic, in the same order as many-ai-cli's input bar. Press the panel's mic, speak,
and press it again: what you said is inserted at the caret of the field. While you speak the
text is shown only in the panel, never in the field. × empties the field and is shown only
while the field has text. Send comes in a later step (it renders but does nothing yet).

Recognition is the browser's own (Web Speech API), run in the extension's offscreen document,
so the microphone is allowed once for the extension (a page opens for that on install) and no
site ever asks. There is no keyboard shortcut in v1.

Not published to npm. The extension is loaded from `dist/`.

## Build

From the repository root:

```sh
pnpm install
pnpm -F vtype-core build           # the extension bundles vtype-core from its dist/
pnpm -F vtype-extension build      # writes packages/extension/dist/
pnpm -F vtype-extension typecheck
pnpm -F vtype-extension test
```

`dist/` contains `manifest.json`, `content.js`, `background.js` (the service worker),
`offscreen.html` + `offscreen.js` (runs recognition) and `permission.html` + `permission.js`
(the one-time microphone page). Every script is one self-contained classic bundle; the build
fails if module syntax is left in an output or a referenced file is missing.

## Load it in Chrome

1. Open `chrome://extensions`.
2. Turn on **Developer mode** (top right).
3. Click **Load unpacked** and choose the `packages/extension/dist` folder
   (not `packages/extension` itself: the manifest there points at the built `content.js`).
4. A tab "vtype needs the microphone" opens (first install only). Press "Allow the
   microphone" and allow it in Chrome's dialog. This is the only microphone dialog vtype should
   ever cause. If you skipped it, the panel offers "Allow it" after the first failed attempt.
5. After rebuilding, press the reload icon on the vtype card, then reload the page you test on.
   Pages that were open before the extension was loaded do not get the content script until
   they are reloaded.

## What to check by hand

Try pages of different kinds, each on a different site:

- a search box (single-line `input type="search"` or `text`)
- a webmail or chat message body (usually a `contenteditable` element, not a textarea)
- a contact or feedback form with a multi-line `textarea`
- a login form (its password field is the negative case)

Check on each:

In the list below, "the mic" is the small, faint mic icon that appears next to the field.

1. Focus the field: the mic appears just outside the field's right edge, centered on
   the first line. Nothing is drawn inside the field and the field's own look does not change.
2. Scroll the page, and scroll any inner scroll area that contains the field: the mic stays
   next to the field. When the field scrolls out of view (including out of an inner scroll
   area) the mic disappears, and it comes back when the field does.
3. Resize the window, and make the layout move the field (open a sidebar, expand text above
   it): the mic follows.
4. Move the mouse quickly across the mic: the panel does **not** open. Rest on it for about
   a third of a second: the panel opens below the mic. Move from the mic into the panel:
   it stays open. Leave both: it closes about half a second later.
5. Pressing on the mic or the panel does not move focus out of the field (the caret stays).
6. Click the password field: no mic at all. If the form has a "show password" button,
   press it and focus the revealed field: still no mic (fields marked
   `autocomplete="current-password"` / `"new-password"` are excluded).
7. Read-only and disabled fields get no mic.
8. Click empty page space: the mic disappears. Switch to another window and back: the
   mic is still next to the field.
9. On a touch device (or Chrome DevTools device emulation with touch), tap the mic: the
   panel opens immediately; tap again: it closes.
10. A field near the right edge of the window: the mic is tucked inside the field's right
    edge instead of being pushed off screen, and the panel grows to the left.
11. The panel shows × / send / mic from left to right. × is invisible while the field is empty
    and appears as soon as you type; pressing it empties the field (the page reacts as if you
    had deleted the text yourself) and × disappears again. Send does nothing yet.

### Speaking into a field

On three sites of different origins (for example a search box, a webmail body and a contact
form), count every microphone permission dialog you see. The expected count is zero.

12. Put the caret in the middle of existing text, open the panel and press its mic: the mic
    turns red and pulses. No permission dialog appears on the site.
13. Speak a sentence. The words appear in the panel (the not-yet-final part in grey); the field
    does not change while you speak.
14. Pause and keep speaking: recording continues until you press the mic again (after a long
    silence, three of Chrome's no-speech timeouts in a row, it stops by itself and keeps the
    text in the panel; pressing the mic again continues after that text).
15. Press the mic again: the panel's text is inserted once at the caret, the text before and
    after the caret is intact, and the panel empties. On a site built with React or a rich
    editor, check the page reacts as if you had typed (character counters, send buttons
    becoming active).
16. Start recording, then click another field and stop: the text goes into the field where you
    started, not the one you clicked.
17. Start recording in one tab, then start it in another tab: the first tab stops and says so,
    keeping its text in the panel.
18. With a Japanese IME, start composing in the field, then stop recording: the text is
    inserted after you finish (or cancel) the composition, not in the middle of it.
19. To see the permission path: in `chrome://settings/content/microphone`, block the
    extension's origin, press the mic: the panel says the microphone is not allowed and offers
    "Allow it", which opens the permission page.

## Hostile-CSS testbed (plan C7)

`testbed/hostile.html` is a synthetic page whose CSS tries to break injected UI: the three
rules from the plan (`* { box-sizing: content-box !important }`, `* { font-size: 24px
!important }`, `div { display: flex !important }`) plus rules aimed at vtype's own class names
and host element. Content scripts do not run on `file://`, so serve it over http:

```sh
node packages/extension/testbed/serve.mjs     # http://127.0.0.1:8787/hostile.html (loopback only)
```

With the extension loaded (see above), open `http://127.0.0.1:8787/hostile.html` and check:

1. The page itself looks broken (big text, flex rows): the hostile CSS is active.
2. Focus each field under "Single-line fields" and "Multi-line": the mic has the same size and
   shape as on a normal page, and the panel (hover the mic) shows × / send / mic at normal size,
   in one row, with small icons.
3. Untick "hostile CSS on" at the top and compare: the mic and panel look the same either way.
4. The fields under "Must show no mic" get no mic.
5. The lookalike page elements with vtype's class names (`panel`, `btn send`, `mic recording`)
   look like plain page elements: not styled or animated like vtype.
6. The scroll-container field, the right-edge field (panel opens leftwards) and the field near
   the bottom (panel opens upwards) behave as described in the list above.
7. With the OS setting "reduce motion" on, press the panel's mic: it turns red but does not
   pulse.

Stop the server with Ctrl+C.

## Scope of this build

- Targets: `input` of type `text` / `search` / `email` / `url` / `tel` (a missing or unknown
  type counts as `text`), `textarea`, and `contenteditable` elements, including inside open
  shadow roots. Password inputs are excluded by an allowlist of input types, not a denylist.
  Read-only and disabled fields are excluded.
- Frames: the content script runs in every frame (`all_frames`), so fields inside iframes get
  their own mic inside that frame.
- Not covered: closed shadow roots (the page hides them from extensions), documents in
  `designMode`, and pages the browser does not let extensions script (`chrome://`, the Chrome
  Web Store).
- The field's DOM is never written, except by × (which empties it the way a user would). The
  mic and panel live in a closed shadow root on a separate `<vtype-root>` element appended to
  `<html>`; their stylesheet exists only inside that shadow root.
