# hidden-chrome-speech（使い捨て・出荷しない）

このディレクトリは出荷しない実験用です。vtype の拡張にもデスクトップ版にも入れません。
`packages/` は変更しません。npm の依存も足しません。

目的は 1 点だけです。Chrome を `--app` で起動し、窓を画面の外に出す、または最小化したとき、
Web Speech API の認識結果が 10 分間届き続けるかを、人が声を出さずに測ります。
マイクの代わりは Windows の読み上げで作った WAV で、Chrome のテスト用起動オプション
（`--use-fake-device-for-media-stream` と `--use-file-for-fake-audio-capture`）で流します。
このオプションは実験だけに使い、製品の起動引数にはしません。
偽入力は getUserMedia にしか届かず、10 分計測では WAV を再生してマイクに聞かせた。

普段使っている Chrome のプロフィールは開きません。`--user-data-dir` は一時フォルダで、
スクリプト終了時に消します。`--remote-debugging-port` は使いません。

## ファイル

| ファイル | 役割 |
|---|---|
| `make-audio.ps1` | 日本語と英語の読み上げを 16kHz モノラル 16bit の WAV にする |
| `page.html` | `webkitSpeechRecognition`（`ja-JP` / `continuous` / `interimResults`）。結果は WebSocket へ送る |
| `server.mjs` | `127.0.0.1` でページを配り、受け取った JSON を時刻つきで JSONL に書く。Node 標準モジュールだけ |
| `run.ps1` | 一時プロフィールで Chrome または Edge を `offscreen` / `minimized` / `normal` で起動する |

## 実行

```powershell
pwsh -File .\make-audio.ps1
pwsh -File .\run.ps1 -Mode normal -Browser chrome
```

`-Mode` は `offscreen` / `minimized` / `normal`、`-Browser` は `chrome` / `edge` です。
既定では 600 秒つけっぱなしにします。計測結果の `results-*.jsonl`、WAV、撮影画像は git に入れません。

`-PlayToSpeakers` は `ja.wav` を既定の再生デバイスへループ再生します。スクリプトの親セッションが
先に切れると再生が残るので、終わったら pwsh・コマンドラインに `vtype-spike-hidden-chrome` を含む
Chrome・`server.mjs` の node が残っていないことを確かめてください。

## 結果（2026-09-22）

Chrome の画面外（`-Mode offscreen`）で 600 秒、認識結果が届き続けました。最小化・通常の窓・Edge は
10 分計測をしていません。詳細はローカルの作業ノートにあります。
