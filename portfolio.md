---
schemaVersion: 1
color: "#6366f1"
initials: "vt"
cat:
  ja: "Chrome拡張 / デスクトップアプリ / 音声入力"
  en: "Chrome Extension / Desktop app / Voice input"
tagline:
  ja: "Web でも、パソコンのどのアプリでも、声で書ける。回数制限も、アカウントも無い。"
  en: "Speak into any text field, on the web or in any app. No quota, no account."
short:
  ja: "画面上のテキスト欄に薄いマイクを出し、話した言葉をその場でカーソル位置へ入れる Chrome 拡張と、どのアプリにも声で入力できるデスクトップ版（Windows・macOS・Linux）。"
  en: "A Chrome extension that puts a faint mic on every text field and types what you say at the caret, and a desktop app (Windows, macOS, Linux) that types it into any app."
tech: ["TypeScript", "Chrome Extension", "MV3", "Web Speech API", "pnpm", "Rust"]
store: "https://chromewebstore.google.com/detail/vtype/nngfilimeplngdjdmgkddlhbdjpmikgn"
live: null
guide: null
privacy: "https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md"
featured: false
features:
  - icon: "◎"
    title: { ja: "上限を持たない", en: "No cap, by design" }
    desc:  { ja: "認識はブラウザ内蔵のものを間借りする。API キーも登録も要らず、1 日何分までの制限も無い。", en: "It borrows the browser's own recognition: no API key, no sign-up, and no daily minute limit." }
  - icon: "▭"
    title: { ja: "どの欄でも同じように", en: "Every kind of field" }
    desc:  { ja: "検索ボックスも、リッチエディタの本文も、iframe の中も。話しながら文字がその場で入る。", en: "Search boxes, rich-text bodies, fields inside iframes — the words land as you speak." }
  - icon: "⚑"
    title: { ja: "許可は一度きり", en: "Asked once, never again" }
    desc:  { ja: "認識を拡張自身のページで回すので、訪れたサイトごとにマイクの確認が出ない。", en: "Recognition runs in the extension's own page, so no site ever shows a microphone dialog." }
  - icon: "⌨"
    title: { ja: "ブラウザの外でも", en: "Outside the browser too" }
    desc:  { ja: "デスクトップ版は、どのアプリでもショートカットを押して話すと、そこへ文字が入る。GitHub Releases・npm・Homebrew で配布。", en: "The desktop app types what you say into any app at a shortcut. On GitHub Releases, npm and Homebrew." }
shots:
  - path: portfolio/mic.png
    caption: { ja: "見えている入力欄のすぐ右に、薄いマイクが並ぶ。欄の見た目は変えない。", en: "A faint mic just right of every visible field; the fields themselves look the same." }
  - path: portfolio/recording.png
    caption: { ja: "押して話すと、話しながらその欄のカーソル位置へ文字が入る。", en: "Press and speak: the words land at the caret while you are still talking." }
  - path: portfolio/settings.png
    caption: { ja: "録音の始め方とマイクを出す場所を選べる。合わないサイトはここで切れる。", en: "Choose how recording starts and which fields get a mic, and switch sites off." }
---
## ja

Web ページのテキスト入力欄に、声で文字を入れる Chrome 拡張です。画面に見えている入力欄にはうっすらとしたマイクが並び、押して話すと、聞き取れた言葉がその欄のカーソル位置へ、話しながらその場で入っていきます。検索ボックスでも、問い合わせフォームでも、リッチテキストの本文欄でも、文字を打てる欄なら同じように使えます。

作った動機は、既存の音声入力拡張のほとんどが「1 日 N 分まで」の上限を設けて有料版へ誘導することでした。Chrome には音声認識が最初から入っているので、それを間借りすれば上限を持つ理由がありません。**上限・有料プラン・アカウント登録を持たないことが、この拡張の存在理由**です。

マイクの許可はインストール直後の 1 回だけです。認識を拡張自身のページ（offscreen document）で回すことで、訪れたサイトごとに許可ダイアログが出る問題を構造的に避けています。パスワード欄では動きません（対象の入力欄を許可リストで決めていて、パスワードはそこに入っていません）。音声は開発者のサーバーへは一切送られません。認識はブラウザに委ねており、Chrome ではブラウザ自身が音声を認識サービスへ送ります。

サイトのボタンとマイクが重なるときはドラッグしてずらせて、その位置をサイトごとに覚えます。合わないサイトはツールバーのアイコンで 1 押しで切れます。表示は日本語と英語に対応していて、言語を足すときはメッセージファイルを 1 本置くだけです。

デスクトップ版（Windows・macOS・Linux）もあります。拡張とは別のプログラムで、ブラウザの外のアプリ（テキストエディタ、チャット、ターミナル）でも、ショートカットを押して話すとその場へ文字が入ります。認識は同じく Chrome の音声認識で、Chrome を専用のプロフィールで裏で動かします（Google Chrome が必要）。0.1.0 を GitHub Releases、npm（`npm i -g @ishizakahiroshi/vtype`）、Homebrew（`brew install ishizakahiroshi/tap/vtype`）で配っていて、Microsoft Store 版は準備中です。macOS 版と Linux 版は、作者はまだ実機で動かしていません。

## en

A Chrome extension that puts your voice into the text fields of web pages. Every text field you can see carries a faint little mic; press it and speak, and the words appear in that field at the caret while you are still talking. Search boxes, contact forms, rich-text message bodies — if you can type in it, this works in it.

It exists because nearly every voice-input extension meters you — so many minutes a day, then a paid plan. Chrome already ships speech recognition, so borrowing it removes the reason for any cap at all. **Having no quota, no paid tier and no account is the point of this one**, not a feature it is missing.

The microphone is allowed once, right after you install it. Recognition runs in the extension's own offscreen document, which structurally avoids the permission dialog appearing on every site you visit. Password fields are excluded (the input types it works on are an allowlist, and password is not in it). No audio is ever sent to the developer; recognition is left to the browser, and in Chrome the browser itself sends the audio to its recognition service.

Where the mic collides with a button of the site's own, drag it aside — it stays there for that site. Where the extension does not belong, the toolbar icon switches it off for that site in one press. The interface speaks English and Japanese, and adding a language is adding one message file.

There is also a desktop app for Windows, macOS and Linux. It is a separate program: in any app outside the browser (a text editor, a chat client, a terminal), press a shortcut and speak, and the words are typed there. It uses the same Chrome speech recognition, running Chrome in the background in a profile of its own (Google Chrome must be installed). Version 0.1.0 is on GitHub Releases, npm (`npm i -g @ishizakahiroshi/vtype`) and Homebrew (`brew install ishizakahiroshi/tap/vtype`); a Microsoft Store version is coming. The author has not yet run the macOS and Linux versions on real machines.
