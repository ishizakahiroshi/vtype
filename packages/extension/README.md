# vtype extension (Chrome, Manifest V3)

The browser extension. When a text field on a web page gets focus, a small mic appears just
outside its right edge; resting the mouse on the mic opens a panel with three buttons,
× / send / mic, in the same order as many-ai-cli's input bar. Pressing that small mic starts
the recording and opens the panel in one go (C7e); pressing it again stops. The words appear in
the field itself, at the caret, and are refined while you talk. The panel's mic does the same
as the small one, and the text is already where it belongs when recognition ends. While the
recording runs the panel does not close by itself, so the waveform and the stop button stay
within reach. The settings page offers a second way to start: with "rest the mouse on the mic",
the panel opening is itself the start, so you can speak without pressing anything. Send stops
recognition and submits the page. × empties the field and is shown only while the field has
text. The panel's text line is a fallback: it shows text that could not be written into the
field (the field was removed, or an IME composition was in the way).

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
`offscreen.html` + `offscreen.js` (runs recognition), `permission.html` + `permission.js`
(the one-time microphone page) and `options.html` + `options.js` (the settings page, registered
as `options_ui`). Every script is one self-contained classic bundle; the build
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
   a third of a second: the panel opens below the mic, and **nothing starts recording** (that
   is the default `click` setting). Move from the mic into the panel: it stays open. Leave
   both: it closes about half a second later.
5. Pressing on the mic or the panel does not move focus out of the field (the caret stays).
6. Click the password field: no mic at all. If the form has a "show password" button,
   press it and focus the revealed field: still no mic (fields marked
   `autocomplete="current-password"` / `"new-password"` are excluded).
7. Read-only and disabled fields get no mic.
8. Click empty page space: the mic disappears. Switch to another window and back: the
   mic is still next to the field.
9. On a touch device (or Chrome DevTools device emulation with touch), tap the mic: the
   panel opens immediately and the recording starts; tap again: it stops. The panel stays open
   (that is where the recording is stopped from) and goes away when the field loses focus.
10. A field near the right edge of the window: the mic is tucked inside the field's right
    edge instead of being pushed off screen, and the panel grows to the left.
11. The panel shows × / send / mic from left to right. × is invisible while the field is empty
    and appears as soon as you type; pressing it empties the field (the page reacts as if you
    had deleted the text yourself) and × disappears again. Send submits the page (C8): it
    interrupts a running recording, and when vtype cannot tell how the page is sent it says so
    and sends nothing.

### Speaking into a field

On three sites of different origins (for example a search box, a webmail body and a contact
form), count every microphone permission dialog you see. The expected count is zero.

12. Put the caret in the middle of existing text and press the small mic next to the field:
    the panel opens, its mic turns red and pulses, and a row of blue bars appears and moves
    while you speak. The caret does not move out of the field when you press. No permission
    dialog appears on the site.
13. Speak a sentence. The words appear in the field as you speak and are corrected in place
    (they do not pile up), with the text before and after the caret intact. On a site built
    with React or a rich editor, check the page reacts as if you had typed (character
    counters, send buttons becoming active).
14. Pause and keep speaking: recording continues until you press the mic again (after a long
    silence, three of Chrome's no-speech timeouts in a row, it stops by itself; what was
    already written stays in the field).
15. Press the small mic again (or the panel's mic): recognition stops, nothing is added a
    second time, and the text in the field stays exactly as it was. The panel stays open;
    pressing the mic once more starts a new recording.
16. Start recording, then click another field and speak: the words go into the field where you
    started, not the one you clicked.
17. Start recording in one tab, then start it in another tab: the first tab stops and says so,
    and what it had already written stays in its field.
18. With a Japanese IME, start composing in the field and keep speaking: the field is not
    rewritten until you finish (or cancel) the composition, and the spoken text lands after it.
19. Press × while recording: the field empties and the next words start from the beginning of
    the empty field.
20. Press send while speaking: recognition stops and the page is submitted with what is in the
    field. If the text could not be written (see the panel's message), nothing is submitted.
21. To see the permission path: in `chrome://settings/content/microphone`, block the
    extension's origin, then press the mic: the panel says the microphone is not allowed and
    offers "Allow it", which opens the permission page. Pressing the mic again tries again
    (a press always tries).

### The settings page

Open it from `chrome://extensions` (Details -> Extension options), or right-click the
extension and choose Options. It has one setting, "what starts recording", and one action,
"put the mic back on every site" (item 32):

22. "Press the mic" (the default) is what items 4 and 12 describe.
23. Switch to "Rest the mouse on the mic" and go back to a page **without reloading it**:
    resting on the mic now opens the panel *and* starts recording at once; a quick pass-over
    still does nothing. Pressing the mic still starts and stops. While recording, moving the
    pointer away leaves the panel open; it closes about half a second after the recording ends.
    Stopping with the panel's mic does not restart it while the panel stays open; let the panel
    close and open it again for the next recording.
24. Switch back to "Press the mic", again without reloading: resting on the mic only opens the
    panel.
25. With the "Rest the mouse on the mic" setting and the microphone blocked (see item 21):
    the first hover says the microphone is not allowed, and hovering again does not repeat the
    attempt; pressing the mic still tries. After a start succeeds, hovering starts recordings
    again.

### Moving the mic out of the way (C7f)

On some sites the mic lands on a button of the site's own (a × inside the search field). It
can be dragged aside, and where it was put is remembered for that site.

26. Press the mic and move the mouse a few centimetres before letting go: the mic (and the
    panel, if it is open) follows the pointer, and letting go does **not** start a recording.
    Press it again without moving: that still starts one.
27. Do the same while a recording runs: the mic moves and the recording keeps going.
28. Scroll the page and resize the window: the mic keeps the distance you gave it, next to the
    field.
29. Drag it far off the edge of the window: it stops at the edge instead of disappearing.
30. Reload the page, and open a second page on the same site: the mic is where you put it.
    Open a different site: the mic is back at its normal place there.
31. On a touch device, drag the mic with one finger: it moves and the page does not scroll
    under it. A tap without moving still starts and stops recording.
32. In the settings page press "Put the mic back on every site": it says how many sites were
    put back, and a page you left open goes back to the normal place **without being
    reloaded**. Pressing it again says the mic has not been moved on any site.

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
- Settings, in `chrome.storage.sync`: `trigger` (`"click"` or `"hover"`: what starts
  recording) and `micOffsets` (where the mic was dragged to, one entry per origin, at most 50,
  oldest dropped first). Every open page follows a change through `chrome.storage.onChanged`,
  with no reload. Anything missing, unreadable or unexpected means the default: start on a
  press, mic at its normal place.
- Not covered: closed shadow roots (the page hides them from extensions), documents in
  `designMode`, and pages the browser does not let extensions script (`chrome://`, the Chrome
  Web Store).
- The field's DOM is never written, except by × (which empties it the way a user would). The
  mic and panel live in a closed shadow root on a separate `<vtype-root>` element appended to
  `<html>`; their stylesheet exists only inside that shadow root.
