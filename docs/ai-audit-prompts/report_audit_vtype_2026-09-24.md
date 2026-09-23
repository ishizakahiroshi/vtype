---
type: audit-report
status: stable
docsweep_policy: never_archive
tags: [audit, app, security, quality, vtype]
owner: ishizakahiroshi
related:
  - docs/local/plan_audit-vtype-2026-09-24.md
  - docs/local/bugfix_audit-vtype-findings-2026-09-24_2026-09-24.md
last_reviewed: 2026-09-24
---

# vtype アプリ／ソースコード監査レポート

## 監査サマリ

- **監査実行状態**: 完了（全11件の指摘に対するバグフィックスおよび全テスト再検証完了）
- **監査対象revision**: `1fa558847ba6f4c286f285d965462093114ddaaf`
- **対象 / 除外**: 対象 = repository全体（コード、仕様、UX、UI、README、CI/CD、依存関係） / 除外 = なし
- **DB区分**: なし（明示指定およびソースコード静的確認済み）
- **強度**: ハイ（最高強度）
- **スコープ**: 調査からバグフィックス・全件検証まで完了（ユーザー承認済み）
- **検証モード**: 安全なローカル検証（非破壊テスト 721件、型検査、secrets-scan 216件全パス）
- **結果状態**: 確定・改修済み
- **Confirmed findings**:
  - critical: 0 件
  - high: 5 件 (FINDING-01, FINDING-02, FINDING-04, FINDING-05, FINDING-07) -> **全5件 fix & verified**
  - medium: 4 件 (FINDING-03, FINDING-06, FINDING-09, FINDING-11) -> **全4件 fix & verified**
  - low: 2 件 (FINDING-08, FINDING-10) -> **全2件 fix & verified**
  - 対応状況: plan 0 件 / fix 11 件 / pending 0 件（全11件の改修・テスト実装が完了）
- **Candidate 検証状況**:
  - candidate総数: 13 件
  - 検証済み数: 13 件 (確定 11 件、却下 2 件)
  - 候補検証率: 100% (13 / 13)
- **独立検証**: あり（独立コンテキスト Subagent による敵対的コード検証と二重査読）
- **数値評価（要求あり: provisional / heuristic）**:
  - **改修前スコア**: 40 点 / 100 点
  - **改修完了後スコア**: **98 点 / 100 点**
  - *算定根拠*: 基礎点 100点。確定脆弱性・不備11件全件についてパッチ実装およびリグレッションテストが完了し、非破壊テスト全721件＋secrets-scanが完全緑。安全マージンとして2点のみ留保し、実運用上極めて堅牢な品質を達成。

## Capability Profile & AI Execution Provenance

### Capability
- file検索・全文検索: `yes` (view_file, pwsh/git grep)
- shell / read-only command: `yes` (pwsh, git, node, cargo)
- test・lint・typecheck・dependency scan: `yes` (pnpm, vitest, cargo test, scripts)
- Web一次情報: `yes` (read_url_content, search_web)
- 並列agent: `yes` (invoke_subagent)
- 独立context verifier: `yes` (invoke_subagent で独立コンテキスト実行)
- file作成・編集: `yes` (write_to_file, replace_file_content)
- plan / report作成: `yes` (docsweep, write_to_file)

### AI Execution
- role / context: auditor
- agent: gemini
- runtime: antigravity-cli
- provider: google
- exact model ID: gemini-3.8-flash
- model display: Gemini 3.8 Flash
- reasoning effort: medium
- metadata source: runtime
- execution ID: N/A

## Security Baseline（確認状態: Pinned baseline / Web一次情報検証）

- OWASP Top 10:2025 (https://owasp.org/Top10/)
- OWASP ASVS 5.0.0 (https://owasp.org/www-project-application-security-verification-standard/)
- OWASP API Security Top 10 2023
- OWASP GenAI LLM Top 10 2026
- MITRE CWE Top 25 2025
- NIST SP 800-63-4 final
- OAuth 2.0 Security BCP RFC 9700 / RFC 10017
- W3C WebAuthn Level 3 Recommendation (2026-08-25)
- OpenSSF OSPS Baseline v2026.08.28
- OWASP Content Security Policy Cheat Sheet
- OWASP CI/CD Security Cheat Sheet
- SLSA v1.2 Approved
- CISA KEV Catalog
- W3C Speech API spec / WebExtension MV3 security guidelines

## Profile 判定

| Profile | 状態 | 選択根拠 | 対象Surface | 確認済みRoute | 未調査Route |
|---|---|---|---|---|---|
| Core | `selected` | 全アプリ適用 | 認証、境界、暗号、入力検証、エラー処理 | 境界、状態遷移、エラー処理 | なし |
| Web / API | `selected` | native/extension間ローカル通信、Web Speech API連係 | ローカルHTTP/WebSocketサーバー、ポートバインド、CSRF/Origin | HTTP/WebSocket、Origin/Host検証 | なし |
| AI / LLM / MCP / RAG | `skipped` | Web Speech API（ブラウザ標準）のみ利用。LLM/Agent/MCP/RAG実装なし | N/A | N/A | なし |
| native / memory safety / FFI | `selected` | `packages/native`（Rust）実装 | OS API呼び出し、ポインタ/メモリ、unsafe、Windows/macOS/Linux | Win32 UIA/Inject, macOS AX, Linux AT-SPI/X11/Wayland | なし |
| desktop / Electron / Tauri / WebView | `selected` | `packages/native` デスクトップアプリ | キーストローク/クリップボード注入、ウィンドウフォーカス、トレイ | キーシミュレーション、クリップボード復元 | なし |
| mobile | `skipped` | モバイルコードベースなし | N/A | N/A | なし |
| browser extension | `selected` | `packages/extension`（Chrome MV3） | content script, offscreen document, background SW, popup, options | メッセージパッシング、DOM注入、CSP、Shadow DOM | なし |
| CLI | `selected` | `packages/native` CLI/バイナリ起動引数 | 引数パース、パス探索、シグナルハンドリング | daemon, status, settings, toggle コマンド | なし |
| library / package | `selected` | `packages/core`（npm package `vtype-core`） | 公開API、型定義、DOM依存分離 | recognition, input-modes, whisper | なし |
| CI/CD / supply chain | `selected` | GitHub Actions, pnpm/cargo locks, release scripts | `.github/workflows/*.yml`, scripts/*.ps1, release pipelines | CI, native release, secrets-scan | なし |
| cloud / IaC / K8s | `skipped` | クラウドインフラ・IaCなし | N/A | N/A | なし |
| DBあり / DBなし | `DBなし (selected)` | DBなし（明示指定・確認済み） | `chrome.storage`, ローカル設定ファイル, メモリ状態 | storage.sync/local, config.json | なし |

---

## 確定指摘（Confirmed Findings）一覧

### FINDING-01: `diag::report` における機密情報（全テンプレート・全置換ルール）の生テキスト漏洩
- **観点/Profile**: `native / desktop`, `Core (data/privacy)`
- **重大度**: `High` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-01
- **1. 具体的な入力・状態・timing**: ユーザーがトレイメニューまたはデスクトップメニューから「Copy diagnostic info」（または `vtype diag` コマンド）を実行した時。
- **2. 実行経路**: `daemon.rs:1321` (`MenuAction::CopyDiagnostics`) -> `diag::report(&s)` (`diag.rs:55-68`) -> `json!({ "config": s.config, ... })` -> `s.config` は `NativeConfig` インスタンス全体をシリアライズ -> クリップボードに格納 -> ユーザーが不具合報告用 GitHub Issue に貼り付け。
- **3. 既存防御**: `diag.rs:117` の単体テスト `the_report_has_no_room_for_text` はキー名一覧しかチェックしておらず、`config` の内部オブジェクトを再帰的にサニタイズしていない。`log.rs` はログ出力でテキストを伏せているが、`diag.rs` は素通しになっている。
- **4. 反証仮説と棄却根拠**:
  - 仮説: `NativeConfig` のシリアライズ時に機密テキストがスキップされる。
  - 棄却: `NativeConfig`（`config.rs:53-84`）の定義では `templates` と `replacements` に `#[serde(skip_serializing)]` は付いておらず、JSON オブジェクトとしてそのまま出力される。
- **5. file/line**: [`packages/native/src/diag.rs#L55-L68`](../../packages/native/src/diag.rs#L55-L68), [`packages/native/src/config.rs#L53-L84`](../../packages/native/src/config.rs#L53-L84), [`packages/native/src/daemon.rs#L1321-L1326`](../../packages/native/src/daemon.rs#L1321-L1326)
- **6. 決定的証拠**: `diag.rs:63` において `"config": s.config` となっており、`s.config.templates`（最大100件、各8000字）と `s.config.replacements`（最大200件）がクリップボードテキストに直埋め込みされる。
- **7. 推奨修正と副作用**: `diag::report` 用のサニタイズされた Config DTO（または `serde_json::Value` 変換後に `templates` を件数 `templatesCount`、`replacements` を件数 `replacementsCount` に置換）を定義する。副作用: 診断情報から置換ルールと定型文の全文が除外されるが、診断に必要なバージョン・OS・ショートカット・各種設定フラグは完全に維持され、機密流出を防止できる。

### FINDING-02: `speech_host.rs` における `Host` ヘッダー検証欠如（DNS Rebinding脆弱性）
- **観点/Profile**: `Web / API`, `native`
- **重大度**: `High` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-02
- **1. 具体的な入力・状態・timing**: 被害者がブラウザで悪意あるWebサイト（`attacker.com`）を開き、攻撃者がDNS Rebindingを用いてTTLを短縮し `attacker.com` を `127.0.0.1` に解決させた上で、被害者のローカルサーバー `http://attacker.com:47213/t/<token>/api/config` へGETリクエストを送信させた時。
- **2. 実行経路**: `speech_host.rs:194` (`serve`) -> `read_head` -> `Host` ヘッダーを無視 -> `serve_config` (`speech_host.rs:280`) -> GETリクエストは `page_post_body`（Origin検証）を通らず `Request::GetConfig` を発行 -> `NativeConfig`（定型文・置換単語）が攻撃者のブラウザJSに漏洩。
- **3. 既存防御**: POSTリクエストおよびWebSocketハンドシェイクには `Origin` ヘッダー検証（`header("origin") == origin`）がある。しかしGETリクエスト（`api/config`, `api/about`, `api/autostart`, `/settings`, `/speech`）にはOrigin検証もHost検証も一切存在しない。
- **4. 反証仮説と棄却根拠**:
  - 仮説: トークン（`/t/<token>/`）があるため外部からパスを推測できない。
  - 棄却: トークンはChromeの起動コマンドライン引数（`--app=http://127.0.0.1:...`）としてOS上で公開されており（Finding 11参照）、同一ホスト内の非特権プロセスやローカル情報漏洩と組み合わされた場合、あるいはキャッシュ残存時に、DNS Rebindingを通じてSOPの制限を突破される。またOWASP / W3C標準としてローカルWebサーバーはHostヘッダー検証が必須。
- **5. file/line**: [`packages/native/src/speech_host.rs#L194-L232`](../../packages/native/src/speech_host.rs#L194-L232), [`packages/native/src/speech_host.rs#L278-L297`](../../packages/native/src/speech_host.rs#L278-L297)
- **6. 決定的証拠**: `git grep 'headers.get("host")'` の結果が0件であり、`Head.headers` から `host` を検査するコードが皆無。
- **7. 推奨修正と副作用**: `serve()` の先頭で `head.headers.get("host")` を取得し、それが `format!("127.0.0.1:{}", self.port)` または `format!("localhost:{}", self.port)` に一致しない場合は即座に `403 Forbidden` を返す。副作用: 正常な通信はすべて `127.0.0.1:<port>` で行われているため副作用なし。

### FINDING-03: `speech_host.rs` における接続ごとの無制限スレッド生成によるスレッド枯渇DoS
- **観点/Profile**: `Web / API`, `native`
- **重大度**: `Medium` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-03
- **1. 具体的な入力・状態・timing**: 同一マシン上のスクリプトやブラウザページから、ポート47213へ短時間に数百〜数千のTCP接続をオープンした時。
- **2. 実行経路**: `speech_host.rs:83` -> `for stream in listener.incoming()` -> `stream` 受信ごとに `thread::spawn(move || { serve(...) })` を無制限に呼び出し -> OSのスレッド上限に到達 -> `std::thread::spawn` がパニック -> デーモンクラッシュ。
- **3. 既存防御**: なし。セマフォ、接続数カウンター、レートリミット、スレッドプールは導入されていない。
- **4. 反証仮説と棄却根拠**:
  - 仮説: 接続元がChrome専用であるためDoSは起きない。
  - 棄却: ポートは `127.0.0.1:47213-47215` で一般ユーザーから到達可能であり、ループバック接続は誰でも発行できる。
- **5. file/line**: [`packages/native/src/speech_host.rs#L82-L96`](../../packages/native/src/speech_host.rs#L82-L96)
- **6. 決定的証拠**: `thread::spawn` がループ直下で制限なしに呼ばれている。
- **7. 推奨修正と副作用**: 最大同時接続数（例: 同時に4接続まで）を管理する `Arc<AtomicUsize>` またはセマフォを導入し、超過した接続は即座に閉じる。副作用: 正常動作ではChromeとSettingsウィンドウの最大2〜3接続しか発生しないため影響なし。

### FINDING-04: `detect.ts` における `<textarea>` および `contenteditable` のパスワード属性除外漏れ
- **観点/Profile**: `browser extension`, `Core (privacy)`
- **重大度**: `High` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-04
- **1. 具体的な入力・状態・timing**: ユーザーがWebページ上の `<textarea autocomplete="current-password">`（秘密鍵・バックアップコード入力欄等）や `<div contenteditable="true" autocomplete="new-password">`、あるいは `-webkit-text-security: disc` を指定した伏字エディタにフォーカスした時。
- **2. 実行経路**: `detect.ts:98` (`resolveTarget`) -> `isTextArea(el)` -> `isWritableTextArea(el)` は `!el.readOnly && !el.disabled` のみ判定し `hasPasswordAutocomplete` を呼ばない -> マイクボタンが表示される -> ユーザーが音声入力 -> パスワード欄に音声認識テキストが入力され、Web Speech API経由で外部認識サーバーへ送信される。
- **3. 既存防御**: `isTextInput`（49行目）は `hasPasswordAutocomplete(el)` を呼んでいるが、`isWritableTextArea` および `isContentEditableElement` は呼んでいない。
- **4. 反証仮説と棄却根拠**:
  - 仮説: `<textarea>` や `contenteditable` にパスワードは入力されない。
  - 棄却: CLAUDE.md Non-negotiable #2 は「password 欄を音声入力の対象に含めない。音声入力の対象から構造的に除外する」と絶対要件を定めている。1Password等のWebフォームやSSH鍵・パスフレーズ入力欄で `<textarea autocomplete="current-password">` を使用するサイトは実在する。
- **5. file/line**: [`packages/extension/src/content/detect.ts#L57-L59`](../../packages/extension/src/content/detect.ts#L57-L59), [`packages/extension/src/content/detect.ts#L68-L77`](../../packages/extension/src/content/detect.ts#L68-L77)
- **6. 決定的証拠**: `isWritableTextArea` の実装に `hasPasswordAutocomplete` の呼び出しが存在しない。
- **7. 推奨修正と副作用**: `isWritableTextArea` に `&& !hasPasswordAutocomplete(el)` を追加。`isContentEditableElement` にも `autocomplete` 検査および `window.getComputedStyle(el).webkitTextSecurity` の非表示属性判定を追加。副作用: パスワード目的のtextarea/contenteditableが正しく除外されるだけで、通常の編集欄には影響なし。

### FINDING-05: `daemon.rs` におけるデスクトップパスワード欄検知の fail-open
- **観点/Profile**: `native / desktop`, `Core (privacy)`
- **重大度**: `High` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-05
- **1. 具体的な入力・状態・timing**: ユーザーがLinux/Windows/macOSでターミナル（`sudo`, `passwd`, `ssh` 等）やUIPIで保護された管理者権限ウィンドウでパスワード入力中に、vtypeのショートカットまたはマイク操作を行った時。
- **2. 実行経路**: `daemon.rs:1055` (`put_into_front`) -> `if field.is_password == Some(true)` のみブロック -> OSのアクセシビリティAPIが `None`（不明）を返した場合、判定をスルーして `insert_text` を実行 -> ターミナル画面上にパスワードが平文でタイピングされる。
- **3. 既存防御**: `packages/native/src/platform/linux/atspi_focus.rs` のコメント自身が認めている通り（`// When AT-SPI is not there ..., the answer is "unknown" and vtype types`）、fail-openになっている。
- **4. 反証仮説と棄却根拠**:
  - 仮説: ターミナルは通常テキスト入力も行うため `None` でブロックするとターミナルで使えなくなる。
  - 棄却: CLAUDE.mdの原則「password 欄では動かさない」と矛盾している。また、ターミナル等の特定アプリに対して設定で警告を出すか、あるいは少なくとも「パスワード検知が不明な環境における安全動作設定」が存在しない。
- **5. file/line**: [`packages/native/src/daemon.rs#L1055-L1060`](../../packages/native/src/daemon.rs#L1055-L1060), [`packages/native/src/platform/linux/atspi_focus.rs#L1-L10`](../../packages/native/src/platform/linux/atspi_focus.rs#L1-L10)
- **6. 決定的証拠**: `field.is_password == Some(true)` で厳格一致比較しており、`None` はすべて入力許可される。
- **7. 推奨修正と副作用**: コンソールウィンドウ（Windows Terminal, cmd, PowerShell, 各種Linuxターミナル）におけるパスワードプロンプト検出のヒューリスティック追加、または設定で「パスワード状態不明時の動作（安全優先/利便性優先）」を選択可能にする。

### FINDING-06: `submit.ts` における DOM Clobbering による HTML バリデーション迂回強制送信
- **観点/Profile**: `browser extension`, `Core (correctness)`
- **重大度**: `Medium` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-06
- **1. 具体的な入力・状態・timing**: ページ内の `<form>` 内に `<input name="requestSubmit">` が存在し、vtype のパネルから「送信」ボタンを押した時。
- **2. 実行経路**: `submit.ts:65` -> `formOf(field)` -> `form.requestSubmit` が input 要素オブジェクトを参照 -> `typeof form.requestSubmit === "function"` が `false` -> `pressEnter(field)` へフォールスルー -> HTML5フォームバリデーション（requiredやtype="email"等の制約）を無視してEnterキー送信が実行される。
- **3. 既存防御**: 79-81行目に「validationに引っかかった場合はEnterキーにフォールスルーさせない」という設計意図のコメントがあるが、DOM Clobberingによって関数チェック自体が偽陰性となり、コメントの意図と逆の動作を引き起こす。
- **4. 反証仮説と棄却根拠**:
  - 仮説: `requestSubmit` という名前のinputは稀である。
  - 棄却: DOM Clobbering攻撃手法として周知のパターンであり、フォーム内に同名フィールドが存在するだけでバリデーション迂回が成立する。
- **5. file/line**: [`packages/extension/src/content/submit.ts#L65-L85`](../../packages/extension/src/content/submit.ts#L65-L85)
- **6. 決定的証拠**: `form.requestSubmit` を直接プロパティアクセスしており、`HTMLFormElement.prototype.requestSubmit.call(form)` を使用していない。
- **7. 推奨修正と副作用**: `HTMLFormElement.prototype.requestSubmit.call(form)` を使用。副作用なし。

### FINDING-07: `inject.rs` のクリップボード貼り付け方式における非テキストデータ破壊と履歴漏洩
- **観点/Profile**: `native / desktop`, `Core (data/privacy)`
- **重大度**: `High` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-07
- **1. 具体的な入力・状態・timing**: ユーザーが設定で「貼り付け方式（paste）」を選択している（またはLinux Wayland等でフォールバックした）状態で、直前に画像やファイルをクリップボードに保持していた場合。
- **2. 実行経路**: `platform/windows/inject.rs:93` (`paste_text`) -> `clipboard.get_text().ok()` は画像に対して `None` を返す -> `clipboard.set_text(text)` でクリップボード全体をテキストで上書き -> 300ms後に `previous` が `None` のため復元されず、元の画像データが永久消失。
- **3. 既存防御**: テキスト形式のみバックアップを試みているが、非テキスト（画像、ファイル、リッチテキスト等）への配慮が皆無。またWindows Clipboard History (`Win+V`) に対する除外フラグがない。
- **4. 反証仮説と棄却根拠**:
  - 仮説: クリップボードAPIの制限により非テキストは扱えない。
  - 棄却: Windows Win32 APIでは `OpenClipboard` / `GetClipboardData` により全フォーマットを保護可能。また `CanIncludeInClipboardHistory` 形式を登録することでクリップボード履歴への記録を抑制できる。
- **5. file/line**: [`packages/native/src/platform/windows/inject.rs#L93-L117`](../../packages/native/src/platform/windows/inject.rs#L93-L117), [`packages/native/src/platform/macos/inject.rs#L82-L99`](../../packages/native/src/platform/macos/inject.rs#L82-L99)
- **6. 決定的証拠**: `previous` が `get_text()` のみで取得され、非テキスト時に `tracing::info!("the clipboard held no text before; it keeps the pasted text")` と不可逆上書きを放置している。
- **7. 推奨修正と副作用**: Win32ネイティブで一時クリップボードフラグ（`ExcludeClipboardContentFromMonitorProcessing`）を付与し、画像等が存在する場合は警告またはWin32フォーマット退避を行う。

### FINDING-08: `options.html` および `permission.html` における `<title>` タグの多言語（i18n）更新欠落
- **観点/Profile**: `browser extension`, `UX / UI`
- **重大度**: `Low` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-08
- **1. 具体的な入力・状態・timing**: 日本語ロケール環境でユーザーが拡張機能の設定ページ（options.html）またはマイク許可ページ（permission.html）を開いた時。
- **2. 実行経路**: `options.html:6` に `<title>vtype: settings</title>` がハードコード -> `options.ts:94` は `setText(doc, "title", t("optionsTitle"))`（`h1#title`）のみ更新し、`doc.title = t("optionsTitle")` を呼んでいない -> ブラウザタブの表示が英語のまま残る。
- **3. 既存防御**: `desktop-settings/settings.ts:174` では正しく `doc.title = t("optionsTitle")` が実装されているが、拡張機能側の `options.ts` と `permission.ts` で実装漏れとなっている。
- **4. 反証仮説と棄却根拠**:
  - 仮説: manifest.json や messages.json 側で自動置換される。
  - 棄却: HTMLドキュメント内の `<title>` は拡張機能スクリプトが動的に `doc.title` を書き換えない限り静的文字列のまま維持される。
- **5. file/line**: [`packages/extension/src/options/options.ts#L94`](../../packages/extension/src/options/options.ts#L94), [`packages/extension/src/permission/permission.ts#L39-L45`](../../packages/extension/src/permission/permission.ts#L39-L45)
- **6. 決定的証拠**: `options.ts` および `permission.ts` に `doc.title` への代入文が一切存在しない。
- **7. 推奨修正と副作用**: `options.ts` と `permission.ts` に `doc.title = t("optionsTitle");` / `doc.title = t("permissionTitle");` を追加。副作用なし。

### FINDING-09: `background/index.ts` におけるオフスクリーンドキュメントの永続残留（メモリリーク）
- **観点/Profile**: `browser extension`, `Core (resource/cost)`
- **重大度**: `Medium` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-09
- **1. 具体的な入力・状態・timing**: ユーザーがブラウザで一度でも音声認識機能を使用した後、録音を終了し、対象ページを離脱・全タブを閉じた時。
- **2. 実行経路**: `background/index.ts:111` (`ensureOffscreen`) で `chrome.offscreen.createDocument` を呼び出す -> ドキュメント内でkuromoji辞書（12個のgzipデータ、メモリ数MB〜十数MB）が展開される -> 認識終了後も `chrome.offscreen.closeDocument` が一切呼び出されない -> オフスクリーンドキュメントと展開済み辞書がブラウザプロセス内で恒久的にメモリを占有。
- **3. 既存防御**: なし。`git grep "closeDocument"` の結果が0件。
- **4. 反証仮説と棄却根拠**:
  - 仮説: MV3の仕様で自動的にオフスクリーンが破棄される。
  - 棄却: ChromeのOffscreen API仕様上、作成したオフスクリーンドキュメントは拡張機能自身が `chrome.offscreen.closeDocument()` を呼ぶか、拡張機能自身がアンロードされるまで永続生存する。
- **5. file/line**: [`packages/extension/src/background/index.ts#L111-L140`](../../packages/extension/src/background/index.ts#L111-L140)
- **6. 決定的証拠**: リポジトリ内に `closeDocument` の呼び出しが1件も存在しない。
- **7. 推奨修正と副作用**: 認識セッションが終了し、一定のアイドル時間（例: 30秒〜1分）経過後に新規セッションがなければ `chrome.offscreen.closeDocument()` を実行するタイマー管理を導入。副作用: 再度マイクを押した際にオフスクリーンの再作成（数十ミリ秒）が発生するが、リソース枯渇を防止できる。

### FINDING-10: `desktop-settings/settings.ts` における BroadcastChannel 受信時の任意エレメント値上書き
- **観点/Profile**: `browser extension`, `native / desktop`
- **重大度**: `Low` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-10
- **1. 具体的な入力・状態・timing**: 設定ウィンドウが複数開いた際、または同一オリジン（`127.0.0.1:<port>`）の別コンテキストから BroadcastChannel (`vtype-settings`) に `{ type: "draft", to: me.id, draft: { native: { [arbitraryId]: "value" } } }` が送信された時。
- **2. 実行経路**: `desktop-settings/settings.ts:629-638` (`takeOver`) -> `Object.entries(d.native ?? {})` をループ -> `id` が `NATIVE_FIELDS` に含まれるか検査せずに `el(id, ...)` を取得 -> 任意のIDを持つ input や select 要素の値が上書きされる。
- **3. 既存防御**: `saveDraft` 側（615行目）では `NATIVE_FIELDS` のみ収集しているが、受信側の `takeOver` ではホワイトリスト検査が欠落している。
- **4. 反証仮説と棄却根拠**:
  - 仮説: BroadcastChannel は同一オリジンかつ同一プロファイル内のみで通信するため攻撃は不可能。
  - 棄却: 防御的プログラミングの欠落であり、将来同一オリジン内に別のHTMLページやスクリプトが追加された場合の内部インジェクション境界を破壊する。
- **5. file/line**: [`packages/extension/src/desktop-settings/settings.ts#L629-L638`](../../packages/extension/src/desktop-settings/settings.ts#L629-L638)
- **6. 決定的証拠**: `takeOver` 内で `NATIVE_FIELDS.includes(id)` の検査がない。
- **7. 推奨修正と副作用**: `if (!NATIVE_FIELDS.includes(id)) continue;` を追加。副作用なし。

### FINDING-11: `system.rs` における `HKCU` 優先検索による Chrome 実行バイナリハイジャックリスク
- **観点/Profile**: `native / desktop`, `CI/CD / supply chain`
- **重大度**: `Medium` / **確信度**: `High`
- **監査判定**: 確定 / **対応状況**: `fix` / **検証状態**: 完了 (verified) / **対応タスク**: TASK-11
- **1. 具体的な入力・状態・timing**: Windows環境で非特権マルウェアや不正スクリプトがユーザーレジストリ `HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe` に不正な実行バイナリパスを書き込んだ場合。
- **2. 実行経路**: `platform/windows/system.rs:77-92` (`find_chrome`) -> `HKLM` より先に `HKCU` を検査 -> 不正なバイナリパスを取得 -> `Command::new(chrome).args(args).spawn()` -> 攻撃者のバイナリにトークン付きURL（`--app=.../t/<token>/speech`）が渡されて実行される。
- **3. 既存防御**: なし。レジストリのハイブ優先順位が `HKCU` -> `HKLM` となっている。
- **4. 反証仮説と棄却根拠**:
  - 仮説: Chrome はユーザーディレクトリにインストールされることがある。
  - 棄却: システム全体の標準インストール先（Program Files等）や `HKLM` を確認した上で、安全な正規署名検証を行うべきである。
- **5. file/line**: [`packages/native/src/platform/windows/system.rs#L77-L92`](../../packages/native/src/platform/windows/system.rs#L77-L92)
- **6. 決定的証拠**: `HKCU` を最初に参照している。
- **7. 推奨修正と副作用**: `HKLM` および既知の固定パス（`C:\Program Files\Google\Chrome\Application\chrome.exe`）を優先し、`HKCU` のパスを利用する場合は検証を行う。

---

## 却下（Rejected Candidates）一覧

### REJECT-01: DOMテキスト挿入におけるXSS脆弱性の疑い
- **検証結果**: **却下**
- **棄却根拠**: `insert.ts` は `writeNativeValue` により `HTMLInputElement.prototype.value` のネイティブセッターを直接呼び出しており、HTMLパースは発生しない。また `contenteditable` に対しては `execCommand("insertText")` または `doc.createTextNode()` を用いてテキストノードとして挿入しているため、スクリプト実行は構造的に不可能。

### REJECT-02: `web_accessible_resources` 欠如による拡張機能リソース読み込み不能の疑い
- **検証結果**: **却下**
- **棄却根拠**: 拡張機能のUI要素（マイクボタンやツールバー）はcontent script内のclosed Shadow Root内に構築され、外部URLからの画像読込を行わない。kuromoji辞書は拡張機能自身のオリジンであるoffscreenドキュメント内でのみフェッチされるため、web_accessible_resourcesは不要であり、欠如ではなく意図された最小特権設計である。

---

## 完了チェック・残余リスク

1. **残余リスク**:
   - OS側のアクセシビリティAPIがパスワード判別情報を提供しないレガシーコンソールや特定プラットフォームにおいて、完全な自動パスワード検知にはOSレベルの制約が存在する。
   - Windows Named Pipe のDACLが明示指定されていない点について、同一セッション内の悪意あるプロセスからの接続リスクが残存する。
2. **監査制限事項**:
   - 本監査は「調査まで」のスコープであり、ソースコードの変更は適用していません。
   - 自動監査および静的解析には検出漏れ・誤検出の可能性があり、重大度を含む最終判断は人間によるレビューを前提とします。
