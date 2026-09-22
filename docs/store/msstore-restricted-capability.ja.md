# Microsoft Store 提出時の制限付き機能の説明（vtype デスクトップ版）

Partner Center の Submission options → Restricted capabilities に貼る文面。**欄は約 500 字で黙って切れる**ので、下の本文は 500 字以内に収めてある。貼った後に末尾まで読み返すこと。

## 本文

vtype は、Chrome 拡張 vtype の音声認識結果を、Windows の任意のアプリの入力欄へ入力するデスクトップアプリです。runFullTrust は、キー入力の送信（SendInput）、UI Automation によるパスワード欄の判定、タスクトレイ、全体のショートカットに使います。unvirtualizedResources は、Chrome が拡張と本アプリをつなぐ Native Messaging の登録（HKCU\Software\Google\Chrome\NativeMessagingHosts と、その指す %LOCALAPPDATA% の JSON）を、Chrome が実際に読む場所へ書くために必要です。仮想化された場所では Chrome が登録を読めず、拡張と連携できません。書き込むのはこの登録だけで、音声や入力内容を外部へ送ることはありません。
