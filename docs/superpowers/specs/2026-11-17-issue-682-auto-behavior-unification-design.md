# 自動動作の設定を3機能で統一する (#682) 設計

> 状態: 計画中（2026-11-17）
> モック: `docs/ui-682-auto-behavior-unification-mockup.html`
> 対象 Issue: #682
> 実装計画: `docs/superpowers/plans/2026-11-17-issue-682-auto-behavior-unification.md`

## 目的

自動圧縮・Discord 自動送信・AI 自動タグ付けの「自動動作」と「対象範囲」を、
同じ型・同じ文言・同じ操作で扱う。現行は次のようにばらばらで、同じ「自動」でも
意味が違う。

| 機能 | 自動動作 | 対象範囲 |
|---|---|---|
| 自動圧縮 | `auto_compress`（bool）＋`auto_compress_immediate`（bool） | `auto_compress_backlog`（bool、初回スキャン限定） |
| Discord | `discord_auto_mode`（Off / Immediate / Waiting） | `discord_auto_since`（撮影時刻の下限） |
| AI タグ付け | `ai_tagging_auto`（bool） | なし（一括処理ボタンのみ） |

この設計では、3機能とも `AutoBehavior { mode, scope, activated_at }` を持ち、
UI では「自動動作」の3チップと「対象範囲」の2チップを同じ順序で表示する。
ジョブのテーブル・状態・ワーカーは統合しない。

## 決定事項

Issue の仕様に加え、実装前に確認した判断項目。

| # | 判断項目 | 決定 |
|---|---|---|
| Q1 | 「既存の未処理分も含める」の実行契機 | 自動では実行しない。各機能の一括処理ボタンを押したときだけ実行する。 |
| Q2 | 「新規」の基準 | ライブラリへの追加以降（`photos.created_at`）。Discord の撮影時刻カットオフは廃止する。 |
| Q3 | Discord の「既存の未処理分」 | まだ送信に成功しておらず、待機・実行待ち・処理中でもない写真。失敗・取消も含む。削除中は除外。 |
| Q4 | 自動圧縮カードのマスタースイッチ | 廃止し、3値の「しない」をオフ状態にする。圧縮形式などの設定は常に編集可能にする。 |
| Q5 | 共通コントロールの形 | 「自動動作」に3チップ、「対象範囲」に2チップを縦に並べる。 |
| Q6 | 初回セットアップの AI ページ | 「自動動作」と「対象範囲」だけを追加する。プラグイン導入・バックエンド・同意は設定画面のまま。 |
| Q7 | AI「即時」でプラグイン未準備 | 現行どおりジョブを積まない。準備完了後の新規写真から処理する。 |
| Q8 | Discord の既存未処理送信 | 対象件数を示す確認ダイアログを出し、OK で送信キューに積む。 |
| Q9 | 対象範囲の役割 | 一括処理ボタンが対象にする写真の範囲を決める。自動投入の対象は変えない。 |
| Q10 | 自動動作が「しない」のときの一括処理 | 使えなくする。「待機」を選ぶと一括処理を使える。 |

## 設定モデル

### 型

`crates/core/src/settings.rs` に共通型を置く。

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoMode {
    #[default]
    Off,
    Immediate,
    Waiting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoScope {
    #[default]
    NewOnly,
    IncludeBacklog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutoBehavior {
    pub mode: AutoMode,
    pub scope: AutoScope,
    /// 自動動作が最後に有効になった Unix 秒。`None` は有効化の記録なし。
    /// `NewOnly` の一括処理はこの時刻以降に追加された写真を対象にする。
    pub activated_at: Option<i64>,
}
```

`DiscordAutoMode` は削除し、すべて `AutoMode` に置き換える。

### `AppSettings` のフィールド

```rust
pub auto_compress: AutoBehavior,   // 旧 auto_compress + auto_compress_immediate
pub discord_auto: AutoBehavior,    // 旧 discord_auto_mode + discord_auto_since
pub ai_tagging_auto: AutoBehavior, // 旧 ai_tagging_auto (bool)
```

削除するフィールド:

- `auto_compress_immediate`
- `auto_compress_backlog`
- `discord_auto_mode`
- `discord_auto_since`

`auto_compress_keep_original`、`auto_compression`、`discord_enabled`、
`discord_destination`、`discord_consent_at`、`ai_tagging_enabled`、
`ai_tagging_backend`、`ai_tagging_idle_timeout_sec` などは変更しない。

### 既定値

| フィールド | 既定 |
|---|---|
| `auto_compress` | `Immediate` / `NewOnly` / 設定行がない新規インストールでは読み込み時の `now` |
| `discord_auto` | `Off` / `NewOnly` / `None` |
| `ai_tagging_auto` | `Off` / `NewOnly` / `None` |

自動圧縮の既定は現行の `auto_compress = true` + `auto_compress_immediate = true` を
維持する。設定行がない新規インストールでは、`AppSettings::load` が
`auto_compress.activated_at = Some(now)` を入れる。`activated_at = None` は
「有効化の記録なし」を意味し、`NewOnly` の一括処理では全未処理写真を対象にする。

### 有効化時刻 `activated_at`

- 自動動作を `Off` から `Immediate` / `Waiting` に変えたとき、`now` を入れる。
- `Immediate` ↔ `Waiting` の切り替えでは変えない。
- `Off` に戻しても消さない。
- Discord の連携スイッチを `false` から `true` に戻したとき、自動動作が `Off` で
  なければ `now` に更新する（現行の再開時カットオフと同じ考え方）。
- Discord の Webhook を未設定から設定したとき、自動動作が `Off` でなければ
  `now` に更新する（現行と同じ）。
- AI 連携スイッチを `false` から `true` に戻したときも、自動動作が `Off` で
  なければ `now` に更新する。
- 対象範囲を変えても `activated_at` は変えない。

`activated_at` は UI の一括処理にだけ使う。デーモンの自動投入は new_ids だけを
見るため、`activated_at` を参照しない。

### 旧設定 JSON の移行

`AppSettings::load` は JSON を `serde_json::Value` として読み、次の移行を
メモリ上で行ってから `AppSettings` にデシリアライズする。保存時は新形式だけを
書き、旧キーは残さない。

| 旧設定 | 新しい `AutoBehavior` |
|---|---|
| `auto_compress = false` | `mode = Off` |
| `auto_compress = true`, `auto_compress_immediate = true` | `mode = Immediate` |
| `auto_compress = true`, `auto_compress_immediate = false` | `mode = Waiting` |
| `auto_compress_backlog = true` | `scope = IncludeBacklog` |
| `auto_compress_backlog = false` | `scope = NewOnly` |
| 上記の圧縮モードが `Off` 以外 | `activated_at = Some(now)` |
| `discord_auto_mode` | `discord_auto.mode`（そのまま） |
| `discord_auto_since` | `discord_auto.activated_at` |
| Discord の `scope` | `NewOnly` 固定 |
| `ai_tagging_auto = true` | `mode = Immediate`, `scope = NewOnly`, `activated_at = Some(now)` |
| `ai_tagging_auto = false` | `mode = Off`, `scope = NewOnly`, `activated_at = None` |

- 旧 `auto_compress` が欠けている場合は既定 `true`、旧 `auto_compress_immediate`
  が欠けている場合は既定 `true`、旧 `auto_compress_backlog` が欠けている場合は
  既定 `false` として扱う。
- 新形式の `auto_compress` / `discord_auto` / `ai_tagging_auto` がすでに
  オブジェクトなら移行しない。
- 移行した場合は、読み込み時に新形式を `app_settings` へ書き戻す（best-effort）。
  書き戻しに失敗しても読み込みは続行し、次回の保存時に新形式になる。
- 壊れた JSON のバックアップとエラー処理は現行のまま。

## 動作仕様

### 自動投入の共通ルール

自動投入は「新しく索引された写真」だけを対象にする。

- 初回フルスキャン（DB の写真数が0のときのフルスキャン）では、既存ライブラリの
  写真を自動投入しない。`scope` が `IncludeBacklog` でも同じ。
- 取り込みの `ScanRoot`（`full_scan = false`）、監視の差分索引、2回目以降の
  フルスキャンで新規索引された写真は自動投入する。
- `mode = Off` では自動投入しない。
- `mode = Immediate` は `pending`、`Waiting` は `waiting` で積む。

`scope` は自動投入に影響しない。既存写真は、利用者が一括処理ボタンを押したときだけ
対象になる。

### 自動圧縮

- `auto_compress.mode` が `Off` 以外のとき、新規索引写真を
  `auto_compress_keep_original` に従う `policy` で積む。
- 初回フルスキャンでは積まない。
- 一括処理「未圧縮画像を一括圧縮」は `auto_compress.mode` が `Off` のとき無効。
- 一括処理の対象は `auto_compress.scope` に従う。
  - `NewOnly`: `activated_at` が `Some(t)` なら `photos.created_at >= t`。
    `None` なら全未処理写真。
  - `IncludeBacklog`: 全未処理写真。
- 一括処理は対象写真を `waiting` で積み、その後 `release_waiting_jobs` で
  既存の待機ジョブも含めて `pending` に戻す（現行の一括操作と同じ）。
- 圧縮カードのマスタースイッチは置かない。カード内の圧縮設定は常に編集できる。
- 一覧の状態ドットは `mode != Off` で点灯する。
- 元画像保持が `false` で `mode != Off` のとき、現行の警告を出す。

### Discord 自動送信

- `discord_enabled = true`、有効な Webhook、`discord_auto.mode != Off` のとき、
  新規索引・紐付け後の写真を積む。
- 撮影時刻によるカットオフは使わない。初回フルスキャンでは積まない。
- `Immediate` は `pending`、`Waiting` は `waiting`。
- 自動モードを `Off` に戻すと `waiting` の自動送信だけを `cancelled` にする
  （現行維持）。
- 一括処理「未送信を一括送信」は `discord_auto.mode` が `Off` のとき無効。
- 一括処理の対象:
  - `photos.status IN ('present', 'replaced')`
  - 削除要求（`pending` / `blocked` / `running`）がない
  - `send_outbox` に `status = 'sent'` の行がない
  - `send_outbox` に `status IN ('waiting', 'pending', 'processing')` の行がない
  - `scope = NewOnly` なら `photos.created_at >= activated_at`（`None` なら全件）
  - `scope = IncludeBacklog` なら全件
- ボタンを押すと対象件数を数え、確認ダイアログを出す。本文には件数と
  「非公開インスタンスの写真も含まれる」旨を出す。
- OK で `send_outbox` に `kind = 'photo'`, `origin = 'auto'`,
  `status = 'pending'` で積み、`discord_consent_at` が未記録なら記録する。
- 0件ならダイアログを出さず、情報トーストを出す。
- 送信履歴の表示・待機の一括送信・再送・取消は変更しない。

### AI 自動タグ付け

- `ai_tagging_enabled = true` かつ `ai_tagging_auto.mode != Off` のとき、新規索引写真を
  自動投入する。
- `Immediate`: プラグインが準備完了のときだけ `pending` で積む。未準備なら積まない
  （現行どおり）。
- `Waiting`: プラグインが未準備でも `waiting` で積む。準備後に
  「待機中のものを処理」で開始できる。
- 初回フルスキャンでは積まない。
- 一括処理「未タグ付けのものを一括処理」は `ai_tagging_auto.mode` が `Off` のとき無効。
- 一括処理の対象は `scope` に従う。既存のバックログ条件（`photo_ai` 行がない、
  削除要求なし、`present` / `replaced`）に `created_at` の条件を足す。
- 一括処理は対象を `waiting` で積み、その後 `release_waiting_tagging_jobs` で
  既存の待機ジョブも含めて `pending` に戻す（現行と同じ）。
- 待機・処理中・失敗の件数、実行中ジョブ、バックエンド選択は変更しない。

## UI

### 共通コントロール

`crates/ui/src/ui/auto_behavior.rs` に共通コンポーネントを置く。表示は次の順序で
固定し、3機能の詳細カードと初回セットアップの3ページで同じものを使う。

1. 見出し「自動動作」
2. チップ「しない」「即時」「待機」
3. 選択中のモードの説明（1行）
4. 見出し「対象範囲」
5. チップ「有効化以降の新規のみ」「既存の未処理分も含める」
6. 選択中の対象範囲の説明（1行）
7. 補足「対象範囲は一括処理ボタンに適用されます。」

- チップは既存の `chip` を使い、選択中はアクセント色、未選択はパネル色。
- 対象範囲の説明は、一括処理の対象が変わることを明記する。
- 自動動作が `Off` のとき、一括処理ボタンは無効にし、
  「一括処理を使うには「待機」にしてください」と表示する。
- アイコンや新しいバッジは追加しない。

### 連携・自動の詳細カード

- **自動圧縮**: ヘッダーのマスタースイッチを削除し、カード本文を常時表示する。
  本文の先頭に共通コントロール、続けて「自動圧縮後に元画像を保持」、警告、
  「未圧縮画像を一括圧縮」（`Off` のとき無効）、圧縮設定を置く。
  一覧の状態ドットは `mode != Off`。
- **Discord**: ヘッダーのマスタースイッチは残す。本文の自動送信セクションを
  共通コントロールに置き換え、`mode != Off` のとき非公開インスタンスの警告を出す。
  その下に「未送信を一括送信」ボタンを置く。Webhook・送信先・テスト送信・
  送信画像・圧縮設定・送信履歴は変更しない。
- **AI タグ付け**: ヘッダーのマスタースイッチは残す。本文の自動スイッチを
  共通コントロールに置き換え、一括処理ボタンを `Off` のとき無効にする。
  待機・処理中・失敗、実行中ジョブ、バックエンド、設定への導線は変更しない。

### 初回セットアップ

- **自動圧縮**: ヘッダーの「自動圧縮」スイッチと「新規写真をすぐに圧縮」スイッチを
  削除し、共通コントロールを置く。「元画像を保持」、警告、形式、ロスレスは残す。
  本体を淡色にする処理は削除する。
- **Discord**: 連携スイッチ、Webhook、送信先、プレイヤー名、原画優先を残し、
  その下に共通コントロールを置く。連携が `Off` のときは現行どおり淡色にする。
- **AI タグ付け**: 紹介文と BOOTH の案内の下、プレビューの上に共通コントロールを
  置く。プレビューの高さを必要に応じて縮め、600×561 に収める。
  プラグイン導入・有効化・バックエンド・同意は追加しない。

## 移行後の挙動変化

利用者に見える変化として、次をリリースノートと docs に明記する。

1. `auto_compress_backlog = true` でも、初回フルスキャンの既存写真は自動圧縮されない。
   既存写真は「未圧縮画像を一括圧縮」で処理する。
2. Discord の撮影時刻カットオフを廃止する。有効化後にライブラリへ追加された写真は、
   撮影日時が古くても自動送信の対象になる（初回フルスキャンは除く）。
3. AI の `ai_tagging_auto = true` でも、初回フルスキャンの既存写真は自動タグ付け
   されない。既存写真は一括処理で処理する。
4. 自動動作が `Off` のとき、圧縮・AI の一括処理ボタンは使えない。「待機」を選ぶ。
5. 対象範囲は一括処理ボタンにだけ適用される。自動投入は常に新規索引写真だけ。

## 対象外

- 圧縮ジョブ・送信 outbox・タグ付けジョブのテーブル、状態、ワーカーの統合。
- 自動圧縮の元画像保持／置換、圧縮形式・品質・リサイズ。
- Discord の送信先、Webhook、同意、送信内容、送信履歴。
- AI のプラグイン導入、モデル、バックエンド、閾値、同意。
- クリップボード連携。
- 設定の DB スキーマバージョン変更（設定は `app_settings` の JSON 1行のまま）。

## 検証

- 設定の移行テスト（旧 JSON の全組み合わせ、保存後に旧キーが消えること）。
- DB の対象範囲テスト（`created_at` の境界、`since = None`、既存ジョブの除外）。
- デーモンの初回スキャン／ScanRoot／監視のテスト（3機能）。
- Discord 一括送信の対象判定・件数・確認・冪等性。
- AI の `Immediate` 未準備と `Waiting` 未準備の違い。
- UI のモード／対象範囲変更、有効化時刻、`Off` 時の一括処理無効。
- i18n の ja / en と placeholder テスト。
- 1280×820 と狭い幅で、3カードと3セットアップページを目視確認。
