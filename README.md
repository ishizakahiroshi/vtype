# vtype

Speak into any text field on the web.

vtype is a browser extension. Every text field you can see on a page carries a faint little mic
just outside its right edge; press it and talk, and the words appear in that field at the caret
while you are still speaking. A search box, a contact form, a rich-text message body: if you can
type in it, this works in it.

The recognition is the browser's own. Chrome has speech recognition built in (the Web Speech
API), and vtype borrows it, so there is no API key, no account, and no cap on how many minutes
you may dictate. Extensions that meter your minutes and sell you more do exist; not being one of
them is the point of this one.

## Install

Not on the Chrome Web Store yet. Until it is, load it unpacked:

```sh
pnpm install
pnpm -F vtype-core build
pnpm -F vtype-extension build
```

Then open `chrome://extensions`, turn on **Developer mode**, press **Load unpacked** and choose
`packages/extension/dist`. A tab asks for the microphone once; allow it there, and no site will
ask you again. Step-by-step instructions, and what to check once it is loaded, are in
[`packages/extension/README.md`](packages/extension/README.md).

Chrome 116 or newer. Firefox is not supported yet: it ships with `SpeechRecognition` disabled and
has no offscreen documents, so it needs a recognition engine of its own.

## What it does

- A faint mic on every visible text field, or only on the field you are using — your choice
- Words land in the field as you speak, at the caret, leaving what is around them intact
- A panel with three buttons: empty the field, submit the page, start and stop recording
- A waveform while recording, so you can see that you are being heard
- Writing waits while a Japanese IME composition is in progress
- Drag the mic aside where it collides with a button of the site's own; it stays where you put it
- Switch vtype off per site from the toolbar icon, and manage that list on the settings page
- No mic on password fields, nor on read-only or disabled ones
- An opt-in diagnostic log for when a recording behaves oddly: timings and counts, never words
- English and Japanese, following the browser's language. Another language is one file:
  see [adding a language](packages/extension/README.md#adding-a-language)

## Privacy

vtype hands your voice to the browser's own speech recognition. **In Chrome, that recognition
works by the browser sending the audio to Google's speech recognition service**, so what you say
reaches Google. That is the thing to know before you speak, and the microphone permission page
says it above the button that grants the permission.

Nothing else leaves your browser. vtype sends nothing to its developer, and has no server to send
it to. It collects no personal information, no browsing history, no page content and no
analytics, and it keeps no transcript history. The full text is in [PRIVACY.md](PRIVACY.md).

## Repository layout

| Path | What it is |
|---|---|
| `packages/core` | `vtype-core`: the recognition engine. Renders nothing; the caller owns every pixel |
| `packages/extension` | The Chrome MV3 extension. Builds to `dist/`, which is what you load |
| `packages/extension/_locales` | Every user-facing string, one file per language ([adding one](packages/extension/README.md#adding-a-language)) |
| `scripts/` | Validation, store packaging and the secrets-scan gate |
| `docs/store/` | The Chrome Web Store listing text, privacy policy and per-version submission notes |
| `spike/mic-permission` | A throwaway spike that established how the microphone permission works. Not shipped |

Development commands, from the repository root:

```sh
pnpm -r build        # core first, then the extension (the extension needs core's dist/)
pnpm -r typecheck
pnpm -r test
```

## License

[MIT](LICENSE).
