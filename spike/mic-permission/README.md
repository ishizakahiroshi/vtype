# vtype mic-permission spike（使い捨て）

このディレクトリは vtype 本体とは無関係の**使い捨て拡張**です。`packages/` には一切影響しません。
目的はただ 1 つ、「オフスクリーンドキュメント方式なら、別オリジンのページを渡り歩いてもマイク許可
ダイアログが 1 回で済むか」を実機で確かめることです。この README の手順だけで、このリポジトリを
初めて触る人でも再現できるように書いています。

**この spike のコードを書いた AI（Claude）は、実際に Chrome へ読み込んで動かす確認をしていません。**
以下の「動作確認手順」は、これを読んだ人（あなた）が実機で行ってください。

## ファイル構成

| ファイル | 役割 |
|---|---|
| `manifest.json` | MV3 マニフェスト。`offscreen` 権限とコンテンツスクリプトを宣言 |
| `content.js` | 各ページに挿入される。画面右下に Start/Stop パネルを出すだけ。`SpeechRecognition` も `getUserMedia` も直接呼ばない |
| `background.js` | サービスワーカー。オフスクリーンドキュメントの作成と、content ⇔ offscreen 間のメッセージ中継だけを行う |
| `offscreen.html` / `offscreen.js` | 実際に `webkitSpeechRecognition` を動かす場所。ここが本番の唯一の未知数 |
| `permission.html` / `permission.js` | plan の変更予定ファイル一覧には無いが追加した拡張ページ（理由は下記「plan からの逸脱」参照）。拡張の `chrome-extension://` オリジンで 1 回だけ `getUserMedia` を呼び、マイク許可を先に取っておくための使い捨てページ |

## メッセージフロー

```
content.js  --chrome.runtime.sendMessage-->  background.js
background.js --chrome.runtime.sendMessage--> offscreen.js
offscreen.js --chrome.runtime.sendMessage--> background.js
background.js --chrome.tabs.sendMessage(tabId,...)--> content.js
```

`content.js` は `chrome.runtime.sendMessage` で開始/停止の意思だけを `background.js` に伝え、
`background.js` がオフスクリーンドキュメントを（未作成なら）作ってから `offscreen.js` へ転送します。
認識結果・エラー・終了通知は逆向きに `offscreen.js` → `background.js` → （該当タブへ）`content.js`
の順で戻ります。

**メッセージの届き先について**: 拡張ページ（background / offscreen）が `chrome.runtime.sendMessage` で
送ったメッセージは拡張ページにだけ届き、content script には届きません。content script へ返すには
`chrome.tabs.sendMessage(tabId, ...)` が必要なので、background.js が開始を依頼したタブの ID を覚えて
中継しています。`content.js` 側の `vtype-spike/bg-to-content-*` 接頭辞と `requestId` の照合は、
同じタブで前回押した Start の返事を取り違えないための防御です。

## 事前準備: 読み込み手順

1. Chrome で `chrome://extensions` を開く
2. 右上の「デベロッパーモード」をオンにする
3. 「パッケージ化されていない拡張機能を読み込む」を押し、このフォルダ（`spike/mic-permission/`）を選ぶ
4. 拡張一覧に「vtype mic-permission spike (throwaway, do not publish)」が現れ、エラーが出ていないことを確認する
5. 拡張のツールバーアイコン（パズルピースのメニューから固定すると見つけやすい）をクリックできる状態にしておく

## 事前準備: マイク許可を 1 回だけ取る

1. 拡張のツールバーアイコンをクリックする → `permission.html` が新しいタブで開く
2. 「Grant microphone access」ボタンを押す
3. ブラウザがマイク許可ダイアログを出すので「許可」を選ぶ **← これがダイアログ 1 回目（拡張オリジン分）**
4. ページの表示が `granted. You can close this tab.` に変わったらこのタブは閉じてよい

このダイアログは拡張の `chrome-extension://<拡張ID>/` オリジンに対して出るものです。以降のテストで
別途カウントする「サイトごとのダイアログ」とは別枠として数えてください。

この事前準備を飛ばして各ページで Start を押すと、`ERROR: onerror: not-allowed` になる見込みです（未確認）。
その場合は事前準備をやり直してください。この表示自体も「offscreen では許可を出せない」ことの観察結果として
記録に残す価値があります。

## 動作確認手順（ここから先はブラウザでの手作業です）

用意するもの: **異なるオリジンの Web ページを 3 つ**。オリジン（スキーム+ホスト+ポート）が異なれば
何でも構いません。例えば検索エンジンのトップページ、ニュースサイトのトップページ、動画サイトの
トップページ、のように普段よく使うサイトを 3 つ選ぶのが手っ取り早いです（ここでは具体的な URL は
指定しません。あなたの環境で開けるサイトを使ってください)。

各ページについて:

1. ページを開く（拡張の content script が動くには `http://` か `https://` のページである必要があります。
   `chrome://` や拡張の内部ページでは動きません）
2. 画面右下に「vtype mic-permission spike」パネルが出ることを確認する（出ない場合はページの
   読み込みタイミングの問題の可能性があるので、リロードしてみる）
3. パネルの「Start」ボタンを押す
4. **この瞬間、新たにマイク許可ダイアログが出るかどうかを記録する** ← 数える対象そのもの
   - 出ない場合: そのまま数秒間何か話しかけ、パネルの表示が `[interim] ...` や `[final] ...` に
     変わるか、`ERROR: ...` になるかを確認する
   - 出た場合: そのオリジンでも許可が必要だったということなので、その事実を記録して次に進む
     （許可してよいし、拒否してエラー内容を見てもよい）
5. パネルの「Stop」ボタンを押す。表示が `recognition ended` になることを確認する
6. devtools の Console（F12 → Console）も開いておき、`[vtype-spike] ...` のログも一緒に記録する
   （パネルの表示と同じ内容が出ます。パネルが画面外に隠れる場合の保険）

これを 3 ページとも行います。

## 記録してほしいこと

各ページごとに次を記録してください。

- ページのオリジン（例のような書き方で構いません。実際のホスト名を記録に残すのは構いませんが、
  この README 自体は書き換えないでください）
- Start 押下時にマイク許可ダイアログが新たに出たか（出た/出なかった）
- パネル・console に出た最終的な表示内容（`[final] ...` のテキスト、または `ERROR: ...` の内容）
- `ERROR:` が出た場合はそのエラー文字列そのもの（`onerror: network` や
  `not-available: neither window.SpeechRecognition nor window.webkitSpeechRecognition exists ...` など）

3 ページ合計でのダイアログ回数（許可取得の 1 回を除く）が、この spike の結論に直結します。

- **合計 0 回** → オフスクリーンドキュメント方式で許可を使い回せる見込みが立つ
- **1 回以上** → 少なくともこの実装のままではオリジンごとに許可が要る、ということが分かる
- **そもそも `ERROR: not-available: ...` が出る** → offscreen document 内で
  `webkitSpeechRecognition` 自体が使えない、ということが分かる（この場合、ダイアログの回数以前の
  問題として報告してください）

いずれの結果であっても、**「動かした・動かなかった」をそのまま記録するのがこの spike の目的**です。
無理に別の実装で通そうとする必要はありません。

## 既知の制限

- 一度に有効な認識セッションは 1 つだけです（`offscreen.js` は `session` 変数を 1 つしか持たない）。
  2 つ以上のタブでほぼ同時に Start を押すと、後から押した方がセッションを奪います。テストは 1 ページ
  ずつ順番に行ってください
- `content_scripts` の `matches` は `*://*/*`（http/https 全部）にしています。読み込み時に
  「すべてのウェブサイトのデータの読み取りと変更」という広い権限の警告が出ます。気になる場合は
  `manifest.json` の `matches` を、実際にテストする 3 つの URL のパターンだけに絞ってから読み込んで
  ください
- Firefox では動きません（`chrome.offscreen` も既定で有効な `SpeechRecognition` も無いため）。この
  spike は Chrome 専用です

## 後片付け

- `chrome://extensions` からこの拡張を「削除」する
- 与えたマイク許可を取り消したい場合は、削除前に拡張の詳細ページ（`chrome://extensions` →
  この拡張の「詳細」）から「サイトの権限」を確認するか、`chrome://settings/content/microphone` で
  この拡張のオリジン（`chrome-extension://<拡張ID>`）への許可を削除する

## plan からの逸脱・不確実な点（実装者からの報告）

- **`permission.html` / `permission.js` は plan の「変更予定ファイル」に無いファイルです。** 追加した
  理由: 委託プロンプトの技術メモにある通り、オフスクリーンドキュメントはフォーカスされない・
  ユーザー操作を受けられないため、ブラウザの許可ダイアログ自体を表示できない見込みです（未確認）。そのため、拡張の
  `chrome-extension://` オリジンで許可を先に取っておく通常のタブ（`permission.html`）を用意しました。
  これは Chrome 公式のオフスクリーンドキュメントのサンプル群でも使われている定石のパターンだと
  記憶していますが、**このリポジトリの環境から Chrome の公式ドキュメントやサンプルリポジトリを
  実際に閲覧して裏取りはしていません**（web 検索ツールを使っていません）。plan の技術メモ自身も
  「UNVERIFIED hints」と明記されているので、この設計は「有力だが未検証」として扱ってください
- **`webkitSpeechRecognition` が offscreen document 内で実際に動くかどうかは、この spike が
  答えを出すために存在する問いそのものです。** 実装側では「動く」とも「動かない」とも断定していません
- `chrome.offscreen.createDocument` の `reasons` に `'USER_MEDIA'` を指定しています。これは
  Chrome の `chrome.offscreen.Reason` 列挙値の一つとして知っている値ですが、これも実機で
  `chrome.runtime.lastError` が出ないかまでは確認できていません
- content script の許可警告（`matches: ["*://*/*"]` による「すべてのサイト」権限）は、実運用の
  vtype 本体では避けるべき設計です。今回は 3 つの任意オリジンで素早く試すための使い捨て設定なので、
  本番の参考にはしないでください（plan にも「本番コードにしない」と明記されています）

## 静的チェックの実施内容（実装 AI による報告）

- `node --check` を `background.js` / `content.js` / `offscreen.js` / `permission.js` の 4 ファイルへ実行し、
  4 件とも構文エラー無しでした
- `manifest.json` は Node の `JSON.parse` で読み込み、パースエラー無しでした
- 実際に Chrome へ読み込んでの動作確認（拡張の読み込み・マイク許可ダイアログの実地カウント）は
  **行っていません**。上記「動作確認手順」を人手で実行してください
