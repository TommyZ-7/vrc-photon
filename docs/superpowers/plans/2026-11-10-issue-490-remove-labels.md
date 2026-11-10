# 余計な表記の削除 (#490) 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Issue #490 に列挙された UI のラベル・ヒント・ツールチップ・注記を、ja / en の i18n キーと call site ごと削除する。削除で中身が空になる見出し・カード・セクション・固定高のキュー行は、領域ごと削除する。

**Architecture:** 既存の画面構造とレイアウトを維持し、表示ノードとキーだけを削除する。説明文を持つ共通スイッチ・ラジオ行は空説明対応にし、空文字のときに説明ノードと余白を描画しない。Photos の未選択インスペクタは列幅を予約したままカードだけを消す。通知は対象がないときモーダルを開かない。圧縮キューの失敗ヒントは `QueueRow::FailedHint` ごと削除する。

**Tech Stack:** Rust 2024, gpui-kit 0.9.1, SQLite, rust-i18n

**Spec:** `docs/superpowers/specs/2026-11-10-issue-490-remove-labels-design.md`

**Mock:** `docs/ui-490-remove-labels-mockup.html`

**Status:** 承認待ち。Phase 0 の docs PR 承認後に Phase 1 を開始する。

## Global Constraints

- 削除のみ。新しい文言・アイコン・設定項目・レイアウトは追加しない。
- Issue #491 の項目のうち、AI カードの状態ラベル以外は扱わない。状態ラベルは本計画で全削除する。
- AI カードの状態ラベル（準備不足・待機中・実行中・失敗）はすべて削除する（Q13）。
- エラーメッセージ、同意・ライセンス文面、破壊的操作の警告文は削除しない。
- 対象キーは `crates/i18n/locales/ja.yml` と `en.yml` の両方から削除する。
- `tr_args!` の登録があるキーは `crates/i18n/tests/placeholders.rs` も更新する。
- 空になった見出し・カード・セクション・固定高の仮想行は、領域ごと削除する。
- スイッチ・ラジオのラベル、操作、選択状態、件数、エラー表示は残す。
- 共通ヘルパーの空説明対応は、既存の説明文の表示を変えない。
- 対象外の未使用キー `compression.use_for_auto`、`settings.cache.unlimited` は削除しない。
- docs とコードは同じ PR に混ぜない。Phase 0 は docs-only PR、Phase 1 は実装 PR。
- 各 PR は最新の `main` から `git worktree` で切る。`main` への直接 push はしない。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- CI が green になるまで merge しない。CI は `ci-watch` の手順で監視する。
- 計画ファイルのチェックボックスは、その作業の完了時に `- [x]` に更新する。

## Review Focus

1. **空説明の余白**: `setting_switch` / `manual_option_row` で空文字を渡したとき、説明 div と `gap` が残らない。既存の説明文は従来どおり出る。
2. **未選択インスペクタ**: 案内カードだけが消え、列幅・ギャラリー幅・詳細パネル開閉が変わらない。
3. **通知なし**: `notification_attention()` が false のときモーダルが開かない。取り込み・ライブラリ・圧縮・デーモンの通知は従来どおり開ける。
4. **`viewer.close`**: クイックルックと通知モーダルの両方が「閉じる」/「Close」になる。キーは残る。
5. **systemd 自動起動**: systemd 管理時は説明なし、それ以外はログイン時起動の説明。スイッチ自体は残る。
6. **AI タグ 0 件**: 空状態の 2 行とタグブロックだけが消え、AI 見出し・fact・再生成ボタンは残る。
7. **AI カードの状態ラベル**: 準備不足・待機中・実行中・失敗のラベルをすべて出さない。件数・ジョブ一覧・操作ボタンは残る。
8. **圧縮履歴 0 件**: 見出しごと消える。写真がある場合は従来どおり履歴を表示する。
9. **圧縮キューの行数**: `QueueRow::FailedHint` の削除で仮想行数・高さ・テスト期待値が変わる。失敗ジョブ自体は表示され続ける。
10. **サイドバー**: 待機中スキャン・未実行取り込みの詳細行が消え、カード高 46 px と見出しは維持される。
11. **docs の整合**: `docs/ui.md` の「閉じる Esc」「メモはまだありません」「案内カード」「24時間の注意書き」「フッター要約」「ロスレス表示」が更新される。

---

## Phase 0: 設計・モック・docs PR

**Files:**
- Create: `docs/superpowers/specs/2026-11-10-issue-490-remove-labels-design.md`
- Create: `docs/superpowers/plans/2026-11-10-issue-490-remove-labels.md`（このファイル）
- Create: `docs/ui-490-remove-labels-mockup.html`
- Modify: `docs/ui.md`

**Interfaces:**
- Produces: 承認済みの設計・モック・計画、更新済み `docs/ui.md`

- [ ] **Step 1: `docs/ui.md` を更新する**

更新箇所:

- §2.3.1 通知センター: 「通知対象がないときは通知モーダルを開かない」を追記する。
- §2.4 写真: 「空のときは案内カードを表示する。」を「写真未選択時はインスペクタ列の幅だけを確保し、案内カードは置かない。」へ変更する。
- §2.5 ビューア: 上部バーの「閉じる Esc」を「閉じる」へ変更する。
- §2.5 メモ: 「「メモはまだありません」に戻る」を「メモ本文を表示しない」へ変更する。
- §2.6 圧縮設定: ロスレス時の「「ロスレスでは品質指定を使用しません」と表示する」を削除し、「品質の増減を無効にする」だけにする。
- §2.7 検索: 「「いずれか（OR）」の注記」を削除し、OR の挙動は本文で説明する。
- §2.8 ダッシュボード: 24 時間保持のアンバー注意書きの文を削除する。
- §2.9 設定: フッターの「言語・サムネイルキャッシュ上限」の表示を削除し、「設定を再読込」だけにする。
- 末尾の参考リンクに設計・計画・モックを追加する。

- [ ] **Step 2: Plannotator で計画を提示する**

```bash
plannotator annotate docs/superpowers/plans/2026-11-10-issue-490-remove-labels.md --gate --json
```

Expected: `decision: "approved"`。注釈が返った場合は計画へ反映して再提示する。

- [ ] **Step 3: Plannotator でモックを提示する**

```bash
plannotator annotate docs/ui-490-remove-labels-mockup.html --gate --json
```

Expected: `decision: "approved"`。Q1〜Q4 の推奨案と、文字だけの削除項目を確認する。

- [ ] **Step 4: docs PR を作る**

```bash
git add docs/ui.md docs/ui-490-remove-labels-mockup.html \
  docs/superpowers/specs/2026-11-10-issue-490-remove-labels-design.md \
  docs/superpowers/plans/2026-11-10-issue-490-remove-labels.md
git commit -m "docs: 余計な表記の削除 #490 の設計・モック・計画"
git push -u origin docs/490-remove-labels
gh pr create --title "docs: #490 余計な表記の削除の設計・モック" \
  --body "Refs #490" --label documentation
```

- [ ] **Step 5: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。docs-only のため Release の bundle は走らない。
green 後 `gh pr merge <n> --merge`、`main` へ戻って pull する。

---

## Phase 1: 実装 PR

### Task 1: 実装用 worktree とベースライン

**Files:**
- None（作業ディレクトリのみ）

- [ ] **Step 1: 最新の `main` から worktree を作る**

```bash
git fetch origin
git worktree add ../vrc-photo-manager-490 -b feat/490-remove-labels origin/main
```

- [ ] **Step 2: worktree でベースラインを確認する**

```bash
cargo test --workspace
```

Expected: PASS。失敗する場合は環境の問題を先に解消する。

---

### Task 2: 共通ヘルパーの空説明対応

**Files:**
- Modify: `crates/ui/src/ui/components.rs`（`setting_switch`, `manual_option_row`）
- Modify: `crates/ui/src/ui/integrations.rs`（`ai_switch_row`）

**Interfaces:**
- Produces: 空文字の説明を渡すと説明ノードを描画しない `setting_switch` / `manual_option_row`
- Produces: 説明引数を持たない `ai_switch_row`

- [ ] **Step 1: `setting_switch` を空説明対応にする**

`description` を `String` に変換した後、`!description.is_empty()` のときだけ説明の `div` を `.child` する。
ラベルの `v_flex` は残し、説明がない場合に `gap(2px)` だけの空行を作らない。

- [ ] **Step 2: `manual_option_row` を空説明対応にする**

同じく `!description.is_empty()` のときだけ説明ノードを描画する。ラジオのタイトル・選択状態・クリックは変えない。

- [ ] **Step 3: `ai_switch_row` から説明引数を削除する**

`integrations.rs` の `ai_switch_row` は 3 か所すべてで説明を削除するため、引数と描画ノードを削除する。
呼び出し側のラベル・値・コールバックは変えない。

- [ ] **Step 4: 既存の説明が残ることを確認する**

```bash
cargo test -p vrc-photon-ui
```

Expected: PASS。空説明対応で既存のスイッチ・ラジオの説明が消えていないことをテストで確認する。

---

### Task 3: Photos / インスペクタ / ビューア

**Files:**
- Modify: `crates/ui/src/ui/library.rs`
- Modify: `crates/ui/src/app/ai.rs`

- [ ] **Step 1: 未選択インスペクタの案内カードを削除する**

`library.rs` の `inspector` で `selected_photo` が `None` のとき、`glass_panel` と 2 つのテキストを削除する。
列幅 `column_width` と `h_full().flex_shrink_0()` は維持し、空の `v_flex` を返す。

- [ ] **Step 2: 「Space で開く」を削除する**

インスペクタヘッダーの右側 `.child(...tr!("photos.inspector.space_hint"))` を削除する。「選択中」は残す。

- [ ] **Step 3: AI タグの空状態を削除する**

`app/ai.rs` の `ai_empty_state_labels` とそのテスト、`library.rs` の import・呼び出しを削除する。
`tags.is_empty()` のときはタグブロックを追加せず、AI 見出し・fact・再生成ボタンは残す。

- [ ] **Step 4: 「読み取り専用」を削除する**

AI メタヘッダーの `photos.ai.read_only` の `.child` を削除する。

- [ ] **Step 5: 圧縮履歴 0 件時にセクションごと削除する**

`derivatives.is_empty()` のとき「圧縮履歴」見出しと空文を出さない。
`derivatives` があるときは従来どおり見出しと履歴行を表示する。

- [ ] **Step 6: メモの空ボックスと Esc 注記を削除する**

`note` が `None` または空白のみのときは本文ボックスを出さない。編集ボタンと見出しは残す。
編集中のフッターから `photos.note.esc_cancel` の `.child` を削除する。

- [ ] **Step 7: テストを更新する**

`ai_empty_state_labels` のテストを削除する。`cargo test -p vrc-photon-ui` を通す。

---

### Task 4: ダッシュボード / 自動起動

**Files:**
- Modify: `crates/ui/src/ui/dashboard_ai.rs`
- Modify: `crates/ui/src/ui/dashboard.rs`
- Modify: `crates/ui/src/app/daemon.rs`
- Modify: `crates/ui/src/ui/settings.rs`

- [ ] **Step 1: AI カードのアイコン・補足・状態ラベルを削除する**

`dashboard_ai.rs` で `IconName::Bot` の 30 px ラッパー、`dashboard.ai.hint` のサブタイトル、
`dashboard.ai.idle_hint` の Idle 条件ブロック、`dashboard.ai.not_ready_hint` のテキストを削除する。
状態ラベルの `(state_label, state_color)` の match と、ラベルを表示する `.child` を削除する。
タイトル・件数・ジョブ一覧・設定/連携ボタンは残す。

- [ ] **Step 2: ログと削除待ちの注記を削除する**

`dashboard.rs` で `dashboard.log_retention_hint` のアンバーカード全体、
`dashboard.deletion.hint` の末尾テキスト、`dashboard.log.open_hint` のテキストを削除する。
ログフォルダを開くボタンと削除待ちの行は残す。

- [ ] **Step 3: systemd 管理時の自動起動説明を削除する**

`app/daemon.rs` の `autostart_description` は、`daemon_active && supervisor == Some("systemd")` のとき
空文字を返す。それ以外は `daemon.autostart.login` を返す。
`settings.rs` の自動起動スイッチは空説明対応の `setting_switch` で表示する。

- [ ] **Step 4: テストを更新する**

`app/daemon.rs` の systemd テストを、systemd 時は空文字、通常時はログイン説明に更新する。

---

### Task 5: サイドバー / 通知

**Files:**
- Modify: `crates/ui/src/ui/shell.rs`
- Modify: `crates/ui/src/ui/notifications.rs`

- [ ] **Step 1: サイドバーの空詳細を描画しない**

`shell.rs` の `CardStatus.detail` は `String` のまま、待機中スキャンと未実行取り込みの `detail` を空文字にする。
描画側は `!status.detail.is_empty()` のときだけ詳細の `div` を追加する。カード高 46 px は維持する。

- [ ] **Step 2: ツールチップを削除する**

「未実行を開始」の `sidebar.start_waiting_hint` と「再スキャン」の `sidebar.rescan_hint` の
`.tooltip(...)` を削除する。ボタンのラベル・動作は変えない。

- [ ] **Step 3: 通知なしでモーダルを開かない**

`notifications.rs` の `open_notification_modal` の先頭で `if !self.notification_attention() { return; }` を追加する。
`items.is_empty()` の `notifications.empty` フォールバックを削除する。
通知対象がある場合の 4 セクション（取り込み・ライブラリ・圧縮・デーモン）は従来どおり描画する。

- [ ] **Step 4: テストを更新する**

`shell.rs` の待機中スキャン・未実行取り込みの詳細期待値を空文字へ更新する。
通知モーダルの既存テストがあれば、空モーダルを開かない条件を追加する。

---

### Task 6: 連携・自動 / Discord

**Files:**
- Modify: `crates/ui/src/ui/integrations.rs`
- Modify: `crates/ui/src/ui/discord.rs`

- [ ] **Step 1: AI の説明を削除する**

`integrations.rs` で以下を削除する。

- `integrations.ai.enabled_hint` / `auto_hint` / `backlog_hint`（`ai_switch_row` の引数ごと）
- `integrations.ai.start_backlog_hint`
- `integrations.ai.backend_hint`
- `integrations.ai.jobs.limit_hint`（右側のテキストと余白スペーサー）
- `integrations.ai.backend_enable_hint` の条件ブロック
- `integrations.ai.jobs.empty_hint`

「実行中のジョブはありません」などの空ラベル、バックログ開始ボタン、バックエンド選択は残す。

- [ ] **Step 2: カードのサブタイトルを削除する**

`integrations.auto.hint`、`integrations.clipboard.hint`、`integrations.discord.hint`、
`integrations.discord.history.hint` のテキストノードを削除する。カード見出しと操作は残す。

- [ ] **Step 3: Discord 送信タイミングの説明を削除する**

`discord.rs` の 3 つの `manual_option_row` から説明を外し、ラベルだけにする。
`manual_option_row` は空説明対応（Task 2）を使う。

---

### Task 7: 設定

**Files:**
- Modify: `crates/ui/src/ui/settings.rs`
- Modify: `crates/ui/src/ui/ai_thresholds.rs`
- Modify: `crates/ui/src/ui/integrations.rs`

- [ ] **Step 1: 表示・起動のスイッチ説明を削除する**

`settings.rs` の表示 4 スイッチ（圧縮状態・ワールド名・日付・サイズ）とトレイ常駐の
`setting_switch` に空文字を渡す。ラベル・値・コールバックは変えない。
`integrations.rs` の「新規写真をすぐに圧縮」も空文字にする。

- [ ] **Step 2: 言語・テーマ・サムネイルの注記を削除する**

`settings.language.hint`、`settings.theme.hint`、`settings.thumbnail.clear_hint`、
`settings.thumbnail.quality_hint`、`settings.thumbnail.size_hint` のテキストノードを削除する。

- [ ] **Step 3: AI カードの注記と probe 警告を削除する**

`settings.ai.install_hint`、`settings.ai.download_note`、`settings.ai.variant.note` を削除する。
`AiPluginCardState::NeedsProbe` の警告ボックスを削除し、未使用になる `ai_warning_box` を削除する。
`AiPluginCardState::NeedsProbe` 自体と「再確認」ボタンは残す。

- [ ] **Step 4: タグ別閾値の説明を削除する**

`ai_thresholds.rs` の `settings.ai.thresholds.hint` のテキストノードを削除する。タイトルと操作は残す。

- [ ] **Step 5: フッター要約を削除する**

`settings.rs` の `settings.footer.summary` の `tr_args!` と表示ノードを削除する。
未使用になる `cache_label` / `language_name` / `active_language` のローカル計算を削除する。
「設定を再読込」ボタンは残す。`settings.cache.unlimited` は列挙外なので残す。

---

### Task 8: 圧縮 / 手動圧縮

**Files:**
- Modify: `crates/ui/src/ui/compression.rs`
- Modify: `crates/ui/src/ui/dialogs.rs`

- [ ] **Step 1: 圧縮キューの失敗ヒント行を削除する**

`QueueRow::FailedHint`、`QUEUE_HINT_ROW_HEIGHT`、`queue_rows` の挿入・高さ・描画の各 match アームを削除する。
`queue_rows` の `failed_job_count` 引数と、呼び出し側の該当引数を削除する。
失敗ジョブの行・並び・再試行は変えない。

- [ ] **Step 2: ロスレス時の品質注記を削除する**

品質行の `if profile.lossless { tr!("compression.lossless_no_quality") } else { tr!("compression.adjust_quality") }` を、
`!profile.lossless` のときだけ「品質を調整」を表示する形にする。ロスレス時の品質ボタン無効化は残す。

- [ ] **Step 3: 保存ヒントのコールアウトを削除する**

`compression.save_hint` のテキストを含む `div`（背景・枠・パディング）ごと削除する。

- [ ] **Step 4: 手動圧縮ダイアログの説明を削除する**

`dialogs.rs` の `dialog.description` / `dialog.description_no_profile` の説明ノードを削除する。
`dialog.output.folder_hint` は `manual_option_row` に空文字を渡す。

- [ ] **Step 5: キュー行のテストを更新する**

`compression.rs` の `QueueRow` テストから `FailedHint` の期待値を削除し、200 失敗時の行数を
`221` から `220` に更新する。`queue_rows` のシグネチャ変更に合わせてテスト呼び出しを直す。

---

### Task 9: 検索 / AI タグピッカー / ギャラリー / 取り込み

**Files:**
- Modify: `crates/ui/src/ui/search.rs`
- Modify: `crates/ui/src/ui/ai_tag_picker.rs`
- Modify: `crates/ui/src/ui/gallery.rs`
- Modify: `crates/ui/src/ui/import.rs`

- [ ] **Step 1: 検索の説明を削除する**

`search.filter_logic` と `search.ai.any` のテキストノードを削除する。
フィルター、AI タグチップ、人数入力、検索 / クリアは残す。

- [ ] **Step 2: AI タグピッカーの説明を削除する**

`ai.tags.picker.hint` のテキストノードを削除する。タイトル、タブ、検索、選択、適用は残す。

- [ ] **Step 3: ギャラリーのエラー案内を削除する**

`gallery.reload_hint` のテキストノードを削除する。エラー文と再試行ボタンは残す。

- [ ] **Step 4: 取り込み先の注記を削除する**

`import.destination_hint` のテキストノードを削除する。取り込み先のパス表示と変更操作は残す。

---

### Task 10: i18n カタログと placeholders

**Files:**
- Modify: `crates/i18n/locales/ja.yml`
- Modify: `crates/i18n/locales/en.yml`
- Modify: `crates/i18n/tests/placeholders.rs`

- [ ] **Step 1: 対象キーを両カタログから削除する**

Issue #490 の対象キー（`settings.display.*.hint`、`settings.thumbnail.quality_hint` / `size_hint`、
`integrations.discord.auto.off_hint` / `immediate_hint` / `waiting_hint`、`dialog.description` /
`description_no_profile` を含む）と、Q13 で全削除する `dashboard.ai.state.not_ready` / `idle` /
`active` / `failed` を ja / en から削除する。

`viewer.close` は削除せず、ja を `"閉じる"`、en を `"Close"` に変更する。

- [ ] **Step 2: `settings.footer.summary` の登録を削除する**

`crates/i18n/tests/placeholders.rs` の `("settings.footer.summary", &["cache", "language"])` を削除する。
他の対象キーに `tr_args!` 登録がないことを確認する。

- [ ] **Step 3: キーの残存を確認する**

```bash
rg -n 'photos\.ai\.(read_only|empty|none_applied)|photos\.inspector\.space_hint|photos\.select_prompt|photos\.no_compression|photos\.note\.(empty|esc_cancel)|dashboard\.(ai\.((hint|idle_hint|not_ready_hint)|state\.(not_ready|idle|active|failed))|log_retention_hint|deletion\.hint|log\.open_hint)|daemon\.autostart\.systemd|sidebar\.(scan\.idle\.detail|import\.none\.detail|start_waiting_hint|rescan_hint)|notifications\.empty|integrations\.(ai\.(enabled_hint|auto_hint|start_backlog_hint|backlog_hint|backend_hint|backend_enable_hint|jobs\.(limit_hint|empty_hint))|auto\.hint|clipboard\.hint|discord\.(hint|history\.hint|auto\.(off_hint|immediate_hint|waiting_hint)))|settings\.(ai\.(needs_probe|thresholds\.hint|install_hint|download_note|variant\.note)|footer\.summary|language\.hint|display\.(status|world|date|size)\.hint|thumbnail\.(clear_hint|quality_hint|size_hint)|theme\.hint|startup\.(tray_hint|immediate_hint))|compression\.(save_hint|queue\.failed_hint|clipboard_system|lossless_no_quality|use_for_auto_hint)|dialog\.(description|description_no_profile|output\.folder_hint)|search\.(filter_logic|ai\.any)|ai\.tags\.picker\.hint|gallery\.reload_hint|import\.destination_hint' crates
```

Expected: 0 件（カタログ・コード・テストのいずれにも残らない）。

- [ ] **Step 4: i18n テストを通す**

```bash
cargo test -p vrc-photon-i18n
```

Expected: PASS。`every_tr_key_exists_in_the_default_catalog` と placeholders の両方が通る。

---

### Task 11: 全体検証

**Files:**
- None

- [ ] **Step 1: ローカル検証を実行する**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: すべて PASS。

- [ ] **Step 2: 実装をコミットする**

```bash
git add crates/ui/src crates/i18n
git commit -m "feat(ui): 余計な表記を削除する (#490)"
git push -u origin feat/490-remove-labels
```

- [ ] **Step 3: PR を作成する**

```bash
gh pr create --title "feat(ui): #490 余計な表記を削除する" \
  --body "Closes #490" --label enhancement
```

PR 本文に以下を書く。

- 対象キーが ja / en から削除されたこと
- 空になった領域ごと削除した箇所（未選択インスペクタ、AI タグ空状態、圧縮履歴、通知、キュー失敗ヒント）
- Q1〜Q4 の決定と、`viewer.close` を通知モーダルにも適用したこと
- 検証コマンドの結果

- [ ] **Step 4: CI green を確認して merge する**

`ci-watch` の手順で `CI` を監視する。green 後 `gh pr merge <n> --merge`。
`main` へ戻って pull し、作業ブランチをローカル・リモートから削除する。

---

## 対象外

- Issue #491 のうち、AI カードの状態ラベル以外（正常時は表示しない、集約、部分削除）。
- 新しい UI 文言・アイコン・設定項目の追加。
- エラー、同意・ライセンス、破壊的操作の警告の削除。
- `compression.use_for_auto`、`settings.cache.unlimited` など、#490 に列挙されていない
  未使用キーの削除。
- ダッシュボード AI カードの状態ラベルは Q13 で全削除する。#491 の状態ラベルの出し分けは
  本計画の削除で置き換わる。
- レイアウト・アニメーション・列幅・カードサイズの変更。

## 受け入れ条件

- Issue #490 の対象表記が ja / en の両 UI から消えている。
- 対象キーが両カタログから削除され、`rg` で呼び出しが残っていない。
- 空になった見出し・カード・セクション・キュー行が残っていない。
- スイッチ・ラジオ・ボタン・件数・エラー表示など、削除対象外の情報と操作が従来どおり動作する。
- AI カードに状態ラベルが表示されない。
- 未選択インスペクタ、通知なし、AI タグ 0 件、圧縮履歴 0 件、ロスレス品質、
  圧縮キュー失敗あり、の各状態で余白だけの領域が残らない。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace` が通る。
