---
schemaVersion: 1
color: "#6366f1"
initials: "vt"
cat:
  ja: "Chrome拡張 / 音声入力"
  en: "Chrome Extension / Voice input"
tagline:
  ja: "Web のどの入力欄にも、声で書ける。回数制限も、アカウントも無い。"
  en: "Speak into any text field on the web. No quota, no account."
short:
  ja: "画面上のテキスト欄に薄いマイクを出し、話した言葉をその場でカーソル位置へ入れる Chrome 拡張。"
  en: "A Chrome extension that puts a faint mic on every text field and types what you say, at the caret."
tech: ["TypeScript", "Chrome Extension", "MV3", "Web Speech API", "pnpm"]
store: null
live: null
guide: null
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
---
## ja

Web ページのテキスト入力欄に、声で文字を入れる Chrome 拡張です。画面に見えている入力欄にはうっすらとしたマイクが並び、押して話すと、聞き取れた言葉がその欄のカーソル位置へ、話しながらその場で入っていきます。検索ボックスでも、問い合わせフォームでも、リッチテキストの本文欄でも、文字を打てる欄なら同じように使えます。

作った動機は、既存の音声入力拡張のほとんどが「1 日 N 分まで」の上限を設けて有料版へ誘導することでした。Chrome には音声認識が最初から入っているので、それを間借りすれば上限を持つ理由がありません。**上限・有料プラン・アカウント登録を持たないことが、この拡張の存在理由**です。

マイクの許可はインストール直後の 1 回だけです。認識を拡張自身のページ（offscreen document）で回すことで、訪れたサイトごとに許可ダイアログが出る問題を構造的に避けています。パスワード欄では動きません（対象の入力欄を許可リストで決めていて、パスワードはそこに入っていません）。音声は開発者のサーバーへは一切送られません。認識はブラウザに委ねており、Chrome ではブラウザ自身が音声を認識サービスへ送ります。

サイトのボタンとマイクが重なるときはドラッグしてずらせて、その位置をサイトごとに覚えます。合わないサイトはツールバーのアイコンで 1 押しで切れます。表示は日本語と英語に対応していて、言語を足すときはメッセージファイルを 1 本置くだけです。

## en

A Chrome extension that puts your voice into the text fields of web pages. Every text field you can see carries a faint little mic; press it and speak, and the words appear in that field at the caret while you are still talking. Search boxes, contact forms, rich-text message bodies — if you can type in it, this works in it.

It exists because nearly every voice-input extension meters you — so many minutes a day, then a paid plan. Chrome already ships speech recognition, so borrowing it removes the reason for any cap at all. **Having no quota, no paid tier and no account is the point of this one**, not a feature it is missing.

The microphone is allowed once, right after you install it. Recognition runs in the extension's own offscreen document, which structurally avoids the permission dialog appearing on every site you visit. Password fields are excluded (the input types it works on are an allowlist, and password is not in it). No audio is ever sent to the developer; recognition is left to the browser, and in Chrome the browser itself sends the audio to its recognition service.

Where the mic collides with a button of the site's own, drag it aside — it stays there for that site. Where the extension does not belong, the toolbar icon switches it off for that site in one press. The interface speaks English and Japanese, and adding a language is adding one message file.
