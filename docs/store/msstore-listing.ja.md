# Microsoft Store 掲載文と Partner Center の申告（vtype desktop 0.1.0・日本語・下書き）

Partner Center の「説明」欄へ貼る文面と、提出時に答える項目の下書き。**まだ提出しない**（提出は依頼者の承認の後）。
英語版は [`msstore-listing.en.md`](msstore-listing.en.md)。

## 説明文

Google Chrome が必要です（音声認識に Google Chrome を使います）。

vtype は、どのアプリの入力欄にも声で文字を入れる常駐アプリです。ショートカット（Ctrl+Alt+Space）か画面の隅のマイクを押して話すと、前面のアプリのカーソルの位置に文字が入ります。メモ帳でも、チャットでも、ターミナルでも使えます。

主な機能:

- どのアプリからでも使えるショートカットと、タスクトレイのアイコン
- 入力モード（通常 / 英語 / カタカナ）。切り替えたまま残ります
- 「A と聞こえたら B にする」置き換え表
- パスワード欄には入力しません
- アカウント登録も、利用時間の上限も、有料プランもありません

音声の行き先:

- vtype は、vtype 専用のプロフィール（普段使いの Chrome とは別のフォルダ）で Google Chrome を起動し、その音声認識を使います。**話した音声は Chrome によって Google の音声認識サービスへ送られます。**
- 最初の録音の前に、このことを説明する画面を出し、同意を求めます。同意するまで録音しません。
- vtype 自身はネットワークへ何も送りません。開発者のサーバーもありません。話した内容も文字も保存しません。

プライバシーポリシー: https://github.com/ishizakahiroshi/vtype/blob/main/PRIVACY.md

## Partner Center で答える項目

### 個人情報を扱うか

はい。利用者が話した音声を、Google Chrome の音声認識（Web Speech API）へ渡します。Chrome がそれを Google の音声認識サービスへ送り、文字にします。vtype 自身は音声も文字も保存せず、開発者へも送りません。プライバシーポリシーの URL は上のとおり（Store Policies 10.5.1。Win32 の製品はプライバシーポリシーが必須）。

### 第三者への共有

あり。共有先は Google（Chrome の音声認識サービス）。**最初の録音の前に、アプリの画面で説明して同意を得てから**送ります（Store Policies 10.5.2 のオプトインの同意）。同意しなければ録音しません。

### 年齢区分（IARC の質問票）

「利用者の個人情報を第三者と共有するか」に当たる質問には「はい（音声を Google の音声認識へ送る）」と答える。利用者同士のやり取り・課金・位置情報の共有は無い。

### 制限付きの機能

`runFullTrust` だけ。理由: 「前面のアプリへ文字を入力し、タスクトレイのアイコンとグローバルなショートカットを提供するデスクトップアプリのため」。`microphone` は宣言しない（マイクを開くのは vtype が起動する Google Chrome で、vtype 自身は開かない）。

### 依存するソフトウェア

Google Chrome（Store Policies 10.2.4。説明文の冒頭で開示する。上の「説明文」の 1 行目）。

## 審査で落ちたとき

Microsoft Store をやめ、npm と GitHub Releases の zip を Windows の正式な入れ方にする（親 plan の「配布経路の判断」）。**ここでは外さない。** 手順:

1. `.github/workflows/native-release.yml` の windows の job から、`MSIX` の step（`scripts/release/build-msix.ps1` を呼ぶ）と、成果物の一覧の `out/*.msix` を消す（冒頭のコメントの「an unsigned MSIX」も）
2. `scripts/release/build-msix.ps1` と `packages/native/packaging/msix/` を消すか、使わない旨を `packages/native/README.md` の Releasing に書く
3. `README.md` の `## Desktop` の入れ方から Microsoft Store を外し、Windows は npm（`npm i -g @ishizakahiroshi/vtype`）と zip にする
4. `packages/native/RELEASE_NOTES.md` の表から `vtype-<ver>.msix` の行を消す
5. 本書と `msstore-listing.en.md` の先頭に「提出しない（審査で不承認）」と書き、不承認の理由を残す
