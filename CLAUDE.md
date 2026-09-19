<!-- このファイルはプロジェクト固有ルールのみを書く。個人/グローバル AI ルール
（言語・確認スタイル・出力フォーマット等）は各 AI ツールのグローバル設定へ。
fresh public clone でも有効な内容に保つこと。 -->

# vtype 開発ガイド

> **このファイルは索引であって本文ではない。** 全 AI セッションで全文がロードされるので、
> ルールの本文はここへ書かず、破る人が必ず開く場所（コード・検査スクリプト・skill・guide・台帳）へ置き、
> ここには索引の 1 行だけを残す。新しいルールを足す前に既存の CLAUDE.md・skill・guide・台帳を検索し、
> 正本が既にあれば参照だけにする。詳細は下記「設計原則の索引」。

## プロジェクト概要

Web 上の**任意のテキスト入力欄**へ音声入力するブラウザ拡張。入力欄にフォーカスするとマイクボタンが現れ、
話した内容がその欄へ差し込まれる。Gmail でも ChatGPT でも社内システムでも、テキスト欄であれば動く。

**認識エンジンはブラウザに内蔵されているものを間借りする。** Chrome では Web Speech API
（`SpeechRecognition` / `webkitSpeechRecognition`）を直接呼ぶため、API キーもアカウントも不要で、
利用回数の上限も無い。既存の商用拡張が 1 日あたりの分数制限を設けて有料版へ誘導しているのに対し、
vtype はその制限自体を持たない。

対象は Chrome と Firefox。ただし **Firefox は `SpeechRecognition` を既定で無効にしており**
（`media.webspeech.recognition.enable` は about:config にあるが Mozilla は出荷していない）、
**offscreen document API も持たない**。したがって Firefox 版は認識エンジンを別途持ち込む必要があり、
Chrome 版とは配管が異なる。共有コア + ブラウザ別アダプタの構成を取るのはこのため。

## やらないこと（スコープ外）

- **有料プラン・利用量の上限・アカウント登録を作らない。** これを持たないことが本拡張の存在理由
- **音声データを自前のサーバーへ集めない。** 送信先はブラウザの認識エンジン、またはユーザー自身が指定したローカルエンジンのみ
- **password 欄では動かさない。** 音声入力の対象から構造的に除外する
- 文字起こしの保存・履歴・検索といったノート機能を持たない（入力欄へ差し込んだら役目は終わり）
- 音声コマンドによるブラウザ操作（タブを開く、スクロールする等）は扱わない。入力欄への文字入力だけ

## 技術スタック

| 種別 | 採用 | 備考 |
|---|---|---|
| 拡張形式 | Manifest V3 | Chrome / Firefox 共通 |
| 言語 | TypeScript | <!-- TODO: ビルド構成が決まったら追記 --> |
| 認識（Chrome） | Web Speech API | ブラウザ内蔵。API キー不要・無制限 |
| 認識（Firefox） | <!-- TODO: 未決定 --> | `SpeechRecognition` が既定無効のため別途必要 |

## ディレクトリ構成

<!-- TODO: 共有コア + ブラウザ別アダプタの実体が決まったら列挙する。 -->

## 主要コマンド

<!-- TODO: ビルド構成が決まったら追記する。 -->

## 設計原則の索引（本文は正本にある）

事故から生まれた設計ルールを追記する表。**本文はここに書かず、破る人が必ず開く場所に置く。**
機械検査があるものは、それが最終的な歯止め（`scripts/check-claude-md.mjs` を CI/hook で検査）。

<!-- TODO: ルールが増えたら以下の表に 1 行足す。空のうちは表ごと削ってよい。
| ルール | 正本（本文はここ） | 機械検査 |
|---|---|---|
| <1 行で> | `<path/to/file>` | `<script or test name>` or なし |
-->

**新しいルールを足す前に、まずこの表に 1 行足せる形にできないかを考える。** できないもの
（機械検査も、決まったファイルも無いもの）だけが本文を持ってよい。

## AI 作業共通ルール

ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等の AI 作業共通ルールは、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` および `~/.claude/guides/`）。

このリポジトリ固有のルール:

- **マイク許可の取得経路を content script に直書きしない。** オリジンごとに許可ダイアログが出るため、
  拡張自身のページ（Chrome は offscreen document）で認識を回し、結果だけを content script へ渡す
- **入力欄への差し込みは `value` 直代入で済ませない。** React 等の制御コンポーネントは
  ネイティブ setter + `input` イベントでないと state が更新されない。contenteditable はまた別経路
- **日本語 IME の変換中に差し込まない。** `compositionstart` / `compositionend` を見て待つ

## Obsidian artifacts

If `docs/obsidian/README.md` exists, use it as an index for related knowledge artifacts.
Use the repository-relative `docs/obsidian` entry. Do not write to a central absolute
path and do not silently fall back to `docs/local` when the entry is missing.

## secrets-scan（このリポジトリの配線）

書く瞬間の責務（固有名詞の一般化・fixture は合成データ等）は上記「AI 作業共通ルール」の参照先に従う。このリポジトリ固有の配線は以下:

- scanner: `scripts/secrets-scan.mjs`（手動実行: `node scripts/secrets-scan.mjs --staged --block`）
- layer 2: `.githooks/pre-commit`（`git config core.hooksPath .githooks` で有効化済み。clone した第三者は `scripts/install-hooks.sh` or `.ps1` を 1 回実行する）
- layer 3: `.github/workflows/secrets-scan.yml` / layer 4: release ゲート
- env (full coverage に必要・未設定なら構造 regex のみで継続): `KB_ROOT` / `FAMILY_ROOT`。設定詳細は `scripts/secrets-scan.mjs` の冒頭コメント
- 参照実装・設計詳細: `worklog-bridge` リポの `docs/local/secrets-scan-design/`（gitignored・公開しない）

## 関連ドキュメント

| 項目 | パス |
|---|---|
| ユーザー向け README | `README.md` |
| Codex/他 AI 用入口 | `AGENTS.md` |
| ローカル作業ノート（非公開） | `docs/local/`（存在する場合） |
| Obsidian knowledge artifacts | `docs/obsidian/`（存在する場合。作業キューではない） |
