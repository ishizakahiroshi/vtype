# Microsoft Store listing and Partner Center answers (vtype desktop 0.1.0, English, draft)

Text for the "Description" field in Partner Center, and drafts of the answers given at
submission. **Not submitted yet** (submission waits for the owner's approval). The Japanese version
is [`msstore-listing.ja.md`](msstore-listing.ja.md).

## Description

Requires Google Chrome (vtype uses Google Chrome for speech recognition).

vtype is a resident app that puts what you say into the text field of any app. Press the shortcut
(Ctrl+Alt+Space) or the mic in the corner of the screen and speak: the text goes where the cursor
is in the app in front, whether that is Notepad, a chat app or a terminal.

What it does:

- A shortcut that works from any app, and a tray icon
- Input modes (normal / English / katakana) that stay until you change them
- A replacement table: "when it hears A, write B"
- Nothing is typed into password fields
- No account, no time limit, no paid plan

Where your voice goes:

- vtype starts Google Chrome in a profile of its own (a folder separate from your usual Chrome)
  and uses its speech recognition. **Chrome sends what you say to Google's speech recognition
  service.**
- Before the first recording, vtype shows a page that explains this and asks for your consent.
  Nothing is recorded until you agree.
- vtype itself sends nothing over the network, and the developer runs no server. Neither what
  you say nor the text is kept.

Privacy policy: https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md

## Answers in Partner Center

### Does the product access personal information

Yes. What the user says is handed to Google Chrome's speech recognition (the Web Speech API);
Chrome sends it to Google's speech recognition service, which turns it into text. vtype itself
keeps neither the audio nor the text, and sends neither to the developer. The privacy policy URL
is above (Store Policies 10.5.1: Win32 products must always have a privacy policy).

### Sharing with third parties

Yes, with Google (Chrome's speech recognition service), **only after the app has explained it on
screen and the user has agreed, before the first recording** (the opt-in consent of Store Policies
10.5.2). Without consent nothing is recorded.

### Age rating (the IARC questionnaire)

To the question about sharing the user's personal information with third parties, answer "yes
(audio is sent to Google's speech recognition)". There is no interaction between users, no
purchase and no sharing of location.

### Restricted capabilities

`runFullTrust` only. Reason: "A desktop app that types text into the app in front and provides a
tray icon and a global shortcut." `microphone` is not declared: the Google Chrome that vtype starts
opens the microphone; vtype itself does not.

### Dependency on other software

Google Chrome (Store Policies 10.2.4; disclosed at the beginning of the description, in the first
line of "Description" above).

## If the Store review rejects it

Drop the Microsoft Store and make npm and the GitHub Releases zip the way to install on Windows
(the parent plan's distribution decision). **Nothing is removed here.** The steps:

1. In `.github/workflows/native-release.yml`, remove the `MSIX` step of the windows job (which
   runs `scripts/release/build-msix.ps1`) and `out/*.msix` from the artifact list (and "an
   unsigned MSIX" from the comment at the top)
2. Remove `scripts/release/build-msix.ps1` and `packages/native/packaging/msix/`, or say under
   Releasing in `packages/native/README.md` that they are not used
3. In `README.md` (`## Desktop`), take the Microsoft Store out of the ways to install; on Windows,
   npm (`npm i -g @ishizakahiroshi/vtype`) and the zip
4. Remove the `vtype-<ver>.msix` row from the table in `packages/native/RELEASE_NOTES.md`
5. Mark this file and `msstore-listing.ja.md` "not submitted (rejected in review)" at the top, with
   the reason
