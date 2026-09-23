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

Get it from the [Chrome Web Store](https://chromewebstore.google.com/detail/vtype/nngfilimeplngdjdmgkddlhbdjpmikgn)
and press **Add to Chrome**. A tab then asks for the microphone once; allow it there, and no site
will ask you again. Chrome keeps it up to date.

Chrome 116 or newer. Firefox is not supported yet: it ships with `SpeechRecognition` disabled and
has no offscreen documents, so it needs a recognition engine of its own.

### From source

To try a change of your own, or a fix that has not reached the store yet, build it and load it
unpacked:

```sh
pnpm install
pnpm -F vtype-core build
pnpm -F vtype-extension build
```

Then open `chrome://extensions`, turn on **Developer mode**, press **Load unpacked** and choose
`packages/extension/dist`. Turn the store version off first, so that only one vtype is running.
Step-by-step instructions, and what to check once it is loaded, are in
[`packages/extension/README.md`](packages/extension/README.md).

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

## Desktop

vtype desktop (Windows, macOS, Linux) takes the same recognition out of the browser: press a
shortcut in any app (a text editor, a chat client, a terminal) and what you say is typed there.
It does not need the extension. It starts Google Chrome in the background, in a Chrome profile of
its own (never your usual one), and uses Chrome's speech recognition; the text is typed into the
app in front. **Google Chrome must be installed.** The extension and the desktop app are separate
programs, and each works without the other.

- A shortcut from anywhere: Ctrl+Alt+Space on Windows and Linux, Control+Option+V on macOS
  (change it with `vtype settings`)
- A small mic icon near the bottom right, and a tray / menu bar icon with the same controls
- Three input modes that stay until you change them: normal, English, katakana
- Nothing is typed into password fields (on Linux, wherever the desktop's accessibility service
  can tell that it is one)
- Experimental, off by default (Windows and macOS): a mic beside the text field you are in

It is not published yet. When it is, it comes from the Microsoft Store on Windows, a Homebrew tap
on macOS (`brew install ishizakahiroshi/tap/vtype`), a `.deb` on GitHub Releases for Linux, and
npm everywhere (`npm i -g @ishizakahiroshi/vtype`). How the packages are made is in
[`packages/native/README.md`](packages/native/README.md).

The macOS and Linux versions are built and tested automatically (CI), but the author has not run
them on a real Mac or Linux machine. If something does not work there, please
[open an issue](https://github.com/ishizakahiroshi/vtype/issues).

### The first time

1. Start vtype and press the shortcut (or the mic icon). A small Chrome window opens once: read
   what vtype sends where, press **Agree and start**, and allow the microphone. The window then
   closes itself, and from then on Chrome runs off screen.
2. The same window has **Start vtype when you sign in**, ticked to begin with. Leave it ticked and
   vtype starts with the OS from then on; clear it and it does not (the Store and `.deb` packages
   start with the OS already, and clearing the box turns that off). To change it later, use the
   same checkbox at the top of the desktop settings on the settings page.
   From the command line, `vtype install` turns it on and, on GNOME, also adds the shortcut (the
   window and the settings page do not add it). `vtype uninstall` turns it off, takes that
   shortcut out, and quits vtype.

If you move vtype, or install it again in another place, start it once from there. Each time vtype
starts, a sign-in entry that starts another copy of vtype is pointed at the one running; it never
adds an entry that is not there. The Store version always starts its own copy.

`vtype settings` (or **Open settings** in the tray menu) opens vtype's own settings page in that
Chrome: the shortcut, the input mode, the replacement table, and how text is typed.

### Commands

```sh
vtype toggle          # start dictation, or stop it if it is running
vtype start           # start
vtype stop            # stop
vtype mode kana       # normal | en | kana
vtype status          # is the speech page connected, is dictation running
vtype settings        # open the settings page
vtype diag            # diagnostic information (never any words you said)
```

The commands talk to the running app, so any launcher can bind them to a key. For example, in
AutoHotkey v2 on Windows:

```ahk
^!k::Run "vtype mode kana"
```

On GNOME, `vtype install` adds Ctrl+Alt+Space as a custom shortcut (Wayland lets no app grab keys
itself); a shortcut of your own under **Settings > Keyboard > Custom Shortcuts** that runs
`vtype toggle` works the same.

### Per system

- macOS: allow vtype in **System Settings > Privacy & Security > Accessibility**, or it cannot type
  into other apps. The binary is not signed, so after an update you may need to allow it again.
- Linux on Wayland: apps may not send keystrokes, so vtype asks through the desktop's remote
  desktop portal (the desktop asks you to allow it), then tries `ydotool` (English letters only),
  and otherwise puts the
  text on the clipboard and tells you to paste it. There is no floating mic icon on Wayland; use
  the top bar icon.

### Reporting a problem

**Report a problem** in the tray menu (and on the extension's settings page) opens a new GitHub
issue in your browser with the OS and version filled in. Nothing is sent until you submit it
yourself. **Copy diagnostic info** puts the OS, version, settings and recent errors on the clipboard
for you to paste; it never includes audio or what you said.

## Privacy

vtype hands your voice to the browser's own speech recognition. **In Chrome, that recognition
works by the browser sending the audio to Google's speech recognition service**, so what you say
reaches Google. That is the thing to know before you speak, and the microphone permission page
says it above the button that grants the permission.

Nothing else leaves your browser. vtype sends nothing to its developer, and has no server to send
it to. It collects no personal information, no browsing history, no page content and no
analytics, and it keeps no transcript history. The desktop app sends nothing over the network
either: it asks for your consent before the first recording, then uses Chrome's recognition in
the same way (so what you say reaches Google through Chrome), and it talks only to that Chrome on
your own computer. The full text is in [PRIVACY.md](PRIVACY.md).

**About vtype**, at the end of the extension's settings page and in the desktop app's tray menu,
says this again, with the version, the developer, the source and the licenses.

## Repository layout

| Path | What it is |
|---|---|
| `packages/core` | `vtype-core`: the recognition engine. Renders nothing; the caller owns every pixel |
| `packages/extension` | The Chrome MV3 extension. Builds to `dist/`, which is what you load |
| `packages/extension/_locales` | Every user-facing string, one file per language ([adding one](packages/extension/README.md#adding-a-language)) |
| `packages/native` | vtype desktop: one Rust program for Windows, macOS and Linux, and its packaging ([details](packages/native/README.md)) |
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
