# 自動動作とキュー表示を3機能で統一する (#682 / #684) 設計

> 状態: 計画中（2026-11-17）
> モック: `docs/ui-682-auto-behavior-unification-mockup.html`
> 対象 Issue: #682, #684
> 実装計画: `docs/superpowers/plans/2026-11-17-issue-682-auto-behavior-unification.md`

## 目的

自動圧縮・Discord 自動送信・AI 自動タグ付けの「自動動作」と「一括処理」を、
同じ型・同じ文言・同じ操作で扱う。現行は次のようにばらばらで、同じ「自動」でも
意味が違う。

| 機能 | 自動動作 | 既存写真の扱い |
|---|---|---|
| 自動圧縮 | `auto_compress`（bool）＋`auto_compress_immediate`（bool） | `auto_compress_backlog`（bool、初回スキャン限定） |
| Discord | `discord_auto_mode`（Off / Immediate / Waiting） | `discord_auto_since`（撮影時刻の下限） |
| AI タグ付け | `ai_tagging_auto`（bool） | なし（一括処理ボタンのみ） |

この設計では、3機能とも機能の有効スイッチと `AutoBehavior { mode, scope,
activated_at }` を持ち、UI では「自動動作」のセグメントと「一括処理」ブロックを
同じ順序で表示する。ジョブのテーブル・状態・ワーカーは統合しない。

#684 では、同じ3機能のキュー表示と操作を統一する。

- 連携・自動の各機能詳細に、その機能の待機・実行待ち・実行中を同じ構成で表示する。
- ダッシュボードは3機能を混合したキュー一覧に置き換える。
- 履歴は表示しない。Discord の送信履歴カードと「履歴を消去」を廃止する。
- 失敗行はキューに残さず、通知モーダルで最新の失敗と再試行を出す。
- 機能ごとにキューの一時停止／再開を設ける。停止中も実行中のジョブは完了する。
- 待機行は行単位・一括で削除できる。実行待ち・実行中は削除できない。

## 決定事項

Issue の仕様に加え、実装前に確認した判断項目。

| # | 判断項目 | 決定 |
|---|---|---|
| Q1 | 「既存の未処理分も含める」の実行契機 | 自動では実行しない。各機能の「一括処理」ボタンを押したときだけ実行する。 |
| Q2 | 「新規」の基準 | ライブラリへの追加以降（`photos.created_at`）。Discord の撮影時刻カットオフは廃止する。 |
| Q3 | Discord の「既存の未処理分」 | まだ送信に成功しておらず、待機・実行待ち・処理中でもない写真。失敗・取消も含む。削除中は除外。 |
| Q4 | 自動圧縮カードのマスタースイッチ | 残す。マスタースイッチは機能の有効／無効、3値は自動動作の実行契機とする。 |
| Q5 | 共通コントロールの形 | 「自動動作」は3分割のセグメント、「一括処理」は対象範囲2分割のセグメント＋実行ボタンにする。どちらもカード化せず、カード本文に直接置く。 |
| Q6 | 初回セットアップ | 「自動動作」だけを追加する。対象範囲・一括処理は表示しない。 |
| Q7 | AI「即時」でプラグイン未準備 | 現行どおりジョブを積まない。準備完了後の新規写真から処理する。 |
| Q8 | Discord の既存未処理送信 | 対象件数を示す確認ダイアログを出し、OK で送信キューに積む。 |
| Q9 | 対象範囲の役割 | 「一括処理」ボタンが対象にする写真の範囲を決める。自動投入の対象は変えない。 |
| Q10 | 自動動作が「しない」のときの一括処理 | 使える。機能のマスタースイッチがオフのときだけ一括処理を無効にする。 |
| Q11 | カード内の表示順 | ヘッダー＋マスタースイッチ → 機能固有の設定 → 自動動作 → 一括処理 → キュー の順に統一する。 |
| Q12 | AI カードの設定導線 | 「設定を開く」ボタンと、その案内文を削除する。プラグイン・バックエンド・同意は設定画面に残す。 |
| Q13 | 一括処理のラベル | 「対象範囲」ではなく「一括処理」と表示し、対象範囲チップと実行ボタンを同じブロックに置く。 |
| Q14 | 一時停止の永続化 | 機能ごとの `paused` を設定に保存し、再起動後も維持する。 |
| Q15 | 一時停止の範囲 | ワーカーが新しいジョブを取得するのを止める。自動投入は続ける。 |
| Q16 | キューの内容 | 待機・実行待ち・実行中だけを表示する。失敗・完了・履歴は表示しない。 |
| Q17 | ダッシュボードの停止操作 | 混合キューには置かず、各機能詳細のキューにだけ置く。 |
| Q18 | 待機の削除 | 行単位と一括削除を用意する。対象は待機だけ。一括は件数だけの確認を出す。 |
| Q19 | 失敗通知 | 通知モーダルに最新5件の失敗行（機能・写真・エラー・再試行）を出す。 |
| Q20 | 表記の整理 | ヒント行、重複ラベル、空表示、出力先の注記を置かない（#627 / #629 の「連携・自動タブの余計な表記削除」に合わせる）。 |

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
    /// 機能が最後に有効になった Unix 秒。`None` は有効化の記録なし。
    /// `NewOnly` の一括処理はこの時刻以降に追加された写真を対象にする。
    pub activated_at: Option<i64>,
}
```

`DiscordAutoMode` は削除し、すべて `AutoMode` に置き換える。

### `AppSettings` のフィールド

```rust
pub auto_compress_enabled: bool,   // 自動圧縮機能のマスタースイッチ
pub auto_compress: AutoBehavior,   // 旧 auto_compress_immediate + auto_compress_backlog
pub auto_compress_queue_paused: bool,
pub discord_enabled: bool,         // 現行の連携スイッチ（変更なし）
pub discord_auto: AutoBehavior,    // 旧 discord_auto_mode + discord_auto_since
pub discord_queue_paused: bool,
pub ai_tagging_enabled: bool,      // 現行の連携スイッチ（変更なし）
pub ai_tagging_auto: AutoBehavior, // 旧 ai_tagging_auto (bool)
pub ai_tagging_queue_paused: bool,
```

`*_queue_paused` は #684 の機能別一時停止。既存 JSON にない場合は
`#[serde(default)]` で `false` になる。ジョブのテーブルや状態には持たせない。

削除するフィールド:

- `auto_compress_immediate`
- `auto_compress_backlog`
- `discord_auto_mode`
- `discord_auto_since`

旧 `auto_compress`（bool）は `auto_compress_enabled` に移る。旧 `auto_compress` は
「機能の有効／無効」と「自動動作の有無」を兼ねていたため、マスタースイッチを
復活させ、3値の `Off` とは分離する。

`auto_compress_keep_original`、`auto_compression`、`discord_destination`、
`discord_consent_at`、`ai_tagging_backend`、`ai_tagging_idle_timeout_sec` などは
変更しない。

### 既定値

| フィールド | 既定 |
|---|---|
| `auto_compress_enabled` | `true` |
| `auto_compress` | `Immediate` / `NewOnly` / 設定行がない新規インストールでは読み込み時の `now` |
| `auto_compress_queue_paused` | `false` |
| `discord_auto` | `Off` / `NewOnly` / `None` |
| `discord_queue_paused` | `false` |
| `ai_tagging_auto` | `Off` / `NewOnly` / `None` |
| `ai_tagging_queue_paused` | `false` |

自動圧縮の既定は現行の `auto_compress = true` + `auto_compress_immediate = true` を
維持する。設定行がない新規インストールでは、`AppSettings::load` が
`auto_compress.activated_at = Some(now)` を入れる。`activated_at = None` は
「有効化の記録なし」を意味し、`NewOnly` の一括処理では全未処理写真を対象にする。

### 有効化時刻 `activated_at`

「有効化」は機能のマスタースイッチがオンになった時点、またはオンになっている
機能で自動動作が `Off` から `Immediate` / `Waiting` に変わった時点を指す。

- 自動圧縮: `auto_compress_enabled` が `false` から `true` になったとき `now`。
  オンになっている状態で `auto_compress.mode` が `Off` から `Off` 以外に
  変わったときも `now`。
- Discord: `discord_enabled` が `false` から `true` になったとき `now`。
  オンになっている状態で `discord_auto.mode` が `Off` から `Off` 以外に
  変わったときも `now`。Webhook を未設定から設定したときも、機能がオンなら
  `now` に更新する。
- AI: `ai_tagging_enabled` が `false` から `true` になったとき `now`。
  オンになっている状態で `ai_tagging_auto.mode` が `Off` から `Off` 以外に
  変わったときも `now`。
- `Immediate` ↔ `Waiting` の切り替えでは変えない。
- `Off` に戻しても消さない。マスタースイッチをオフにしても消さない。
- 対象範囲を変えても `activated_at` は変えない。

`activated_at` は UI の一括処理にだけ使う。デーモンの自動投入は new_ids だけを
見るため、`activated_at` を参照しない。

### 旧設定 JSON の移行

`AppSettings::load` は JSON を `serde_json::Value` として読み、次の移行を
メモリ上で行ってから `AppSettings` にデシリアライズする。保存時は新形式だけを
書き、旧キーは残さない。

| 旧設定 | 新しい設定 |
|---|---|
| `auto_compress = false` | `auto_compress_enabled = false`, `auto_compress.mode = Immediate` |
| `auto_compress = true`, `auto_compress_immediate = true` | `auto_compress_enabled = true`, `mode = Immediate` |
| `auto_compress = true`, `auto_compress_immediate = false` | `auto_compress_enabled = true`, `mode = Waiting` |
| `auto_compress_backlog = true` | `auto_compress.scope = IncludeBacklog` |
| `auto_compress_backlog = false` | `auto_compress.scope = NewOnly` |
| 圧縮機能が有効 | `auto_compress.activated_at = Some(now)` |
| 圧縮機能が無効 | `auto_compress.activated_at = None` |
| `discord_auto_mode` | `discord_auto.mode`（そのまま） |
| `discord_auto_since` | `discord_auto.activated_at` |
| Discord の `scope` | `NewOnly` 固定 |
| Discord が有効で `activated_at` がない | `discord_auto.activated_at = Some(now)` |
| `ai_tagging_auto = true` | `mode = Immediate`, `scope = NewOnly` |
| `ai_tagging_auto = false` | `mode = Off`, `scope = NewOnly` |
| AI が有効 | `ai_tagging_auto.activated_at = Some(now)` |
| AI が無効 | `ai_tagging_auto.activated_at = None` |

- 旧 `auto_compress` が欠けている場合は既定 `true`、旧 `auto_compress_immediate`
  が欠けている場合は既定 `true`、旧 `auto_compress_backlog` が欠けている場合は
  既定 `false` として扱う。
- 新形式の `auto_compress` / `discord_auto` / `ai_tagging_auto` がすでに
  オブジェクトなら移行しない。`auto_compress_enabled` がすでにある場合も
  上書きしない。
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
- 機能のマスタースイッチがオフ、または `mode = Off` では自動投入しない。
- `mode = Immediate` は `pending`、`Waiting` は `waiting` で積む。

`scope` は自動投入に影響しない。既存写真は、利用者が「一括処理」ボタンを押した
ときだけ対象になる。

### 自動圧縮

- 自動投入: `auto_compress_enabled = true` かつ `auto_compress.mode != Off` のとき、
  新規索引写真を `auto_compress_keep_original` に従う `policy` で積む。
- 初回フルスキャンでは積まない。
- 一括処理「未圧縮画像を一括圧縮」は `auto_compress_enabled = true` のとき使える。
  `mode = Off` でも使える。
- 一括処理の対象は `auto_compress.scope` に従う。
  - `NewOnly`: `activated_at` が `Some(t)` なら `photos.created_at >= t`。
    `None` なら全未処理写真。
  - `IncludeBacklog`: 全未処理写真。
- 一括処理は対象写真を `waiting` で積み、その後 `release_waiting_jobs` で
  既存の待機ジョブも含めて `pending` に戻す（現行の一括操作と同じ）。
- カードのマスタースイッチは `auto_compress_enabled`。カード内の圧縮設定は
  マスタースイッチがオンのとき編集できる（オフのときは現行どおり淡色）。
- 一覧の状態ドットは `auto_compress_enabled` で点灯する。
- 元画像保持が `false` で機能が有効なとき、現行の警告を出す。

### Discord 自動送信

- 自動投入: `discord_enabled = true`、有効な Webhook、
  `discord_auto.mode != Off` のとき、新規索引・紐付け後の写真を積む。
- 撮影時刻によるカットオフは使わない。初回フルスキャンでは積まない。
- `Immediate` は `pending`、`Waiting` は `waiting`。
- 自動モードを `Off` に戻すと `waiting` の自動送信だけを `cancelled` にする
  （現行維持）。
- 一括処理「未送信を一括送信」は `discord_enabled = true` かつ有効な Webhook の
  とき使える。`discord_auto.mode = Off` でも使える。
- 非公開インスタンスの警告は、カードにも同意ダイアログにも表示しない。
  `warning.private_instances` を削除し、同意ダイアログの本文からも
  非公開インスタンスの記述を削除する。
- 一括処理の対象:
  - `photos.status IN ('present', 'replaced')`
  - 削除要求（`pending` / `blocked` / `running`）がない
  - `send_outbox` に `status = 'sent'` の行がない
  - `send_outbox` に `status IN ('waiting', 'pending', 'processing')` の行がない
  - `scope = NewOnly` なら `photos.created_at >= activated_at`（`None` なら全件）
  - `scope = IncludeBacklog` なら全件
- ボタンを押すと対象件数を数え、確認ダイアログを出す。本文は件数だけを出す。
- OK で `send_outbox` に `kind = 'photo'`, `origin = 'auto'`,
  `status = 'pending'` で積み、`discord_consent_at` が未記録なら記録する。
- 0件ならダイアログを出さず、情報トーストを出す。
- 送信履歴は #684 で廃止する。待機の一括送信はキューの
  「待機をすべて開始」に移す。再送・取消は履歴 UI とともに削除する。

### AI 自動タグ付け

- 自動投入: `ai_tagging_enabled = true` かつ `ai_tagging_auto.mode != Off` のとき、
  新規索引写真を自動投入する。
- `Immediate`: プラグインが準備完了のときだけ `pending` で積む。未準備なら積まない
  （現行どおり）。
- `Waiting`: プラグインが未準備でも `waiting` で積む。準備後に
  キューの「待機をすべて開始」で開始できる（#684）。
- 初回フルスキャンでは積まない。
- 一括処理「未タグ付けのものを一括処理」は `ai_tagging_enabled = true` かつ
  プラグインが準備完了のとき使える。`ai_tagging_auto.mode = Off` でも使える。
- 一括処理の対象は `scope` に従う。既存のバックログ条件（`photo_ai` 行がない、
  削除要求なし、`present` / `replaced`）に `created_at` の条件を足す。
- 一括処理は対象を `waiting` で積み、その後 `release_waiting_tagging_jobs` で
  既存の待機ジョブも含めて `pending` に戻す（現行と同じ）。
- 待機・処理中・失敗の件数、実行中ジョブ、バックエンド選択は変更しない。
- AI カード内の「設定を開く」ボタンと、その案内文は削除する。
  プラグイン導入・有効化・バックエンド・同意は設定画面に残す。

## UI

### 共通コントロール

`crates/ui/src/ui/auto_behavior.rs` に共通コンポーネントを置く。

1. **自動動作** — 3分割のセグメントコントロール。
   - 選択肢: 「しない」「即時」「待機」。
   - 選択中のセグメントをアクセント色で塗る。
   - 説明文は置かない。
2. **一括処理** — 見出し、対象範囲2分割のセグメント、実行ボタン。
   - 選択肢: 「有効化以降の新規のみ」「既存の未処理分も含める」。
   - 説明文は置かない。対象範囲の意味は選択肢の文言で示す。
   - ボタン: 機能ごとのラベル（未圧縮画像を一括圧縮 / 未送信を一括送信 /
     未タグ付けのものを一括処理）。
   - ボタンは機能のマスタースイッチがオンのとき使える。自動動作が `Off` でも
     使える。Discord は有効な Webhook、AI はプラグイン準備完了も必要。
   - 実行ボタンと対象範囲を同じ並びに置き、対象範囲が何に効くかを明確にする。

自動動作と一括処理はカード化せず、見出し・セグメント・ボタンをカード本文に
直接置く。両者の間には区切り線を置く。自動動作のセグメントは3カード＋
3セットアップページで使う。一括処理のセグメントは3カードだけで使い、
セットアップページには表示しない。

### 連携・自動の詳細カード — 表示順を統一

すべてのカードで次の順に並べる。

1. ヘッダー（タイトル＋マスタースイッチ）
2. 機能固有の設定
3. 自動動作（セグメント）
4. 一括処理（対象範囲2分割セグメント＋実行ボタン）
5. キュー（待機・実行待ち・実行中）

- **自動圧縮**: ヘッダーにマスタースイッチを置く。機能固有の設定は
  「自動圧縮後に元画像を保持」、警告、圧縮設定。続けて自動動作、一括処理、
  キュー。失敗行・履歴は置かない。
- **Discord**: ヘッダーにマスタースイッチ。機能固有の設定は Webhook、送信先、
  テスト送信、送信画像、圧縮設定。続けて自動動作、一括処理、キュー。
  送信履歴は置かない。
- **AI タグ付け**: ヘッダーにマスタースイッチ。機能固有の設定はバックエンド。
  続けて自動動作、一括処理、キュー。失敗件数・最新失敗・設定画面へのボタンは
  置かない。
- 自動動作が `Off` でも一括処理は有効。マスタースイッチがオフのときだけ
  カード本文を淡色にし、一括処理も無効にする。

### 初回セットアップ

セットアップページには自動動作のセグメントだけを置き、一括処理は表示しない。

- **自動圧縮**: マスタースイッチ、元画像を保持、形式、ロスレス、自動動作。
  一括処理は表示しない。
- **Discord**: 連携スイッチ、Webhook、送信先、プレイヤー名、原画優先、
  自動動作。一括処理は表示しない。連携が `Off` のときは現行どおり淡色。
- **AI タグ付け**: 紹介文、自動動作、プレビュー、BOOTH の案内。
  一括処理・設定画面への導線は表示しない。

## #684: キュー表示の統一

### 表示モデル

ジョブのテーブル・状態・ワーカーは統合しない。UI だけが3機能の有効な行を
共通の表示モデルに変換する。

```rust
pub enum QueueFeature {
    Compression,
    Discord,
    Tagging,
}

pub enum QueueState {
    Waiting,
    Pending,
    Processing,
}

pub struct QueueEntry {
    pub feature: QueueFeature,
    pub job_id: String,
    pub photo_id: Option<i64>, // Discord のテスト送信は None
    pub photo_name: Option<String>,
    pub state: QueueState,
    pub progress: f32,
    pub detail: String,       // ステージ、送信先、自動作業の別など
    pub created_at: String,
}

pub struct QueueCounts {
    pub waiting: i64,
    pub pending: i64,
    pub processing: i64,
}
```

- 連携・自動の機能詳細は、その機能の `QueueEntry` だけを表示する。
- ダッシュボードは3機能の `QueueEntry` を混合し、`created_at` の古い順に表示する。
- 表示するのは待機・実行待ち・実行中だけ。完了・失敗・取消は表示しない。
- 1画面の表示上限は100行。総件数はヘッダーに出す。
- 写真名は `photos.file_name` を使い、取得できない場合は `photo #id` にする。
  Discord のテスト送信は写真を持たないため「テスト送信」と表示する。

### 機能詳細のキュー

各機能カードの「一括処理」の下に「キュー」セクションを置く。

- ヘッダー:
  - 見出し「キュー」。
  - 件数「待機 N / 実行待ち N / 実行中 N」。
  - 「一時停止」／「再開」ボタン。機能ごとの設定を保存する。
  - 「待機をすべて開始」ボタン（待機が0件なら無効）。
  - 「待機をすべて削除」ボタン（待機が0件なら無効、件数だけの確認）。
- 行:
  - 状態バッジ（待機／実行待ち／実行中）。
  - 写真名。
  - 実行中は進捗バーとステージ。実行待ちは待ち状態。待機は作成時刻。
  - 待機行だけ「削除」ボタンを出す。実行待ち・実行中は削除できない。
- 一時停止中はヘッダーに「一時停止中」を出し、ボタンを「再開」にする。
  行の削除と「待機をすべて開始」は一時停止中でも操作できる。開始したジョブは
  再開まで実行されない。
- 圧縮の「待機をすべて開始」で `keep_original = false` のときは、現行の
  置換確認ダイアログを出す。
- 履歴・失敗行はこのセクションに置かない。失敗は通知モーダルで扱う。

### ダッシュボードの混合キュー

現行の `queue_panel`（圧縮専用）を `mixed_queue_panel` に置き換える。

- ヘッダー:
  - 見出し「キュー」。
  - 件数「待機 N / 実行待ち N / 実行中 N」（3機能合計）。
  - 一時停止／再開は置かない。機能詳細で操作する。
- 行:
  - 機能バッジ（自動圧縮／Discord／AI タグ付け）。
  - 状態バッジ。
  - 写真名。
  - 実行中は進捗バーとステージ、実行待ちは待ち状態、待機は作成時刻。
  - 待機行だけ「削除」ボタンを出す。
- 100行を超える場合は表示上限100件と総件数を出す。仮想リストを使う。
- 空のときは「キューは空です」を出す。
- ダッシュボードの「未完了」統計カードは3機能合計の実行中＋実行待ち＋待機にする。
  既存の圧縮専用カウントはサイドバーの圧縮カードで使い続ける。
- 失敗行・履歴行は表示しない。失敗は通知ベルで知らせる。

### 一時停止

- 設定: `auto_compress_queue_paused` / `discord_queue_paused` /
  `ai_tagging_queue_paused`。既定は `false`。
- ワーカーは新しいジョブを取得する前に停止フラグを見る。実行中のジョブは
  完了まで進める。
- 自動投入は止めない。停止中も新規写真は待機・実行待ちに積まれる。
- 圧縮ワーカーは既存のグローバル `paused` と機能別フラグの OR で停止する。
- Discord ワーカーは `claim_ready_send` の前に `discord_queue_paused` を見る。
- AI ワーカーは既存のグローバル `paused` と機能別フラグの OR で停止する。
- 再開時は各ワーカーの `JobSignal` を起こして再取得させる。
- UI は設定を保存してデーモンを起こす。デーモンは設定を再読込して
  機能別フラグを更新する。

### 待機の削除

DB に待機専用の削除 API を追加する。`pending` / `processing` は削除しない。

- 圧縮: `delete_waiting_job(job_id) -> Result<bool>` /
  `delete_waiting_jobs() -> Result<usize>`
- Discord: `delete_waiting_send(id) -> Result<bool>` /
  `delete_waiting_sends() -> Result<usize>`
- AI: `delete_waiting_tagging_job(job_id, plugin_id) -> Result<bool>` /
  `delete_waiting_tagging_jobs(plugin_id) -> Result<usize>`

すべて `DELETE ... WHERE status = 'waiting'` で、行そのものを消す。これにより
部分ユニークインデックスが空き、後から同じ写真を再投入できる。
一括削除は件数だけの確認ダイアログを出し、OK で実行する。

### 履歴の廃止

- Discord の送信履歴カード、行、メッセージリンク、再送・取消、
  「履歴を消去」を削除する。待機の一括送信はキューの
  「待機をすべて開始」に移す。
- 圧縮のダッシュボードキューから失敗行・再試行を削除する。失敗は通知へ移す。
- AI の連携カードから失敗件数・最新失敗行を削除する。失敗は通知へ移す。
- 完了・送信済み・失敗・取消の terminal 行は DB に残す。理由は次の2つ。
  - 失敗通知の最新行と再試行のため。
  - Discord のテスト送信の結果を解決するため。
- terminal 行を表示する履歴 UI は作らない。`clear_send_history` などの
  履歴消去 UI も置かない。

### 失敗通知

通知ベルとモーダルに「処理の失敗」セクションを追加する。

- 対象: 圧縮・Discord・AI の失敗ジョブ。
- 表示: 機能バッジ、写真名、エラー概要、再試行ボタン。最新5件まで。
- 機能ごとの失敗件数と「すべて再試行」も出す。
- 失敗が新しく増えたときは、現行どおりトーストで知らせる。
- 通知ベルのドットは、いずれかの機能に失敗がある間点灯する。
- 再試行は既存 API を使う。
  - 圧縮: `retry_job(id)` / `retry_all_failed_jobs()`
  - Discord: `retry_send(id)` / 失敗全件の一括再試行は新設
  - AI: `retry_tagging_job(id)`（既存 DAO を UI から使う）/ 失敗全件の
    一括再試行は新設
- 失敗行はキューに戻さない。再試行で `pending` になった行だけがキューに出る。

### 失敗通知のデータ

- 圧縮: `list_jobs_by_status(Failed, 5)` と `count_jobs(Failed)`。
- Discord: `recent_failed_sends(5)` と `count_failed_sends()` を新設。
- AI: `recent_failed_tagging_jobs(plugin_id, 5)` と
  `tagging_job_counts(plugin_id).failed`。
- UI の通知スナップショットは、3機能の失敗行を `FailedJobEntry` に変換し、
  `created_at` の新しい順に最大5件を出す。

## 移行後の挙動変化

利用者に見える変化として、次をリリースノートと docs に明記する。

1. 旧 `auto_compress = false` は「機能のマスタースイッチがオフ」に移行する。
   自動動作の既定は `Immediate` で、マスタースイッチをオンにすると即時圧縮が始まる。
2. `auto_compress_backlog = true` でも、初回フルスキャンの既存写真は自動圧縮されない。
   既存写真は「一括処理」で処理する。
3. Discord の撮影時刻カットオフを廃止する。有効化後にライブラリへ追加された写真は、
   撮影日時が古くても自動送信の対象になる（初回フルスキャンは除く）。
4. AI の `ai_tagging_auto = true` でも、初回フルスキャンの既存写真は自動タグ付け
   されない。既存写真は一括処理で処理する。
5. 自動動作が `Off` でも、機能が有効なら一括処理を使える。
6. AI カードから設定画面へのボタンと案内を削除する。
7. カード内の表示順を「ヘッダー → 機能固有 → 自動動作 → 一括処理 → キュー」に
   統一する。
8. Discord の送信履歴カードと「履歴を消去」を廃止する。完了・送信済み・失敗の
   履歴を表示しない。
9. 失敗行はキューに残さない。通知ベルに最新5件の失敗と再試行を出す。
10. 機能ごとのキューの一時停止／再開を追加する。設定に保存し、再起動後も維持する。
    停止中も実行中のジョブは完了し、自動投入は続く。
11. 待機行を行単位・一括で削除できる。実行待ち・実行中は削除できない。
12. ダッシュボードは3機能を混合したキューに置き換える。圧縮専用キューパネルは廃止する。
13. ダッシュボードの混合キューには一時停止／再開を置かない。機能詳細で操作する。
14. 自動動作と一括処理のヒント行、重複ラベル、空表示、出力先の注記を削除する。
    #627 / #629 の「連携・自動タブの余計な表記削除」に合わせる。
15. 確認ダイアログは件数だけを表示し、注意書きや補足を置かない。

## 対象外

- 圧縮ジョブ・送信 outbox・タグ付けジョブのテーブル、状態、ワーカーの統合。
- 自動圧縮の元画像保持／置換、圧縮形式・品質・リサイズ。
- Discord の送信先、Webhook、同意、送信内容。
- AI のプラグイン導入、モデル、バックエンド、閾値、同意。
- ダッシュボードの AI カードの導線（設定画面へのリンク）は、この Issue では
  変更しない。必要なら別 Issue で扱う。
- サイドバーの圧縮ステータスカードとリボンの圧縮操作は、この Issue では
  変更しない。圧縮専用のショートカットとして残す。
- 履歴・terminal 行の DB 保持期間や prune の変更。表示だけを廃止する。
- クリップボード連携。
- 設定の DB スキーマバージョン変更（設定は `app_settings` の JSON 1行のまま）。

## 検証

- 設定の移行テスト（旧 JSON の全組み合わせ、保存後に旧キーが消えること）。
- マスタースイッチと3値の組み合わせ（自動投入の有無、一括処理の可否）。
- DB の対象範囲テスト（`created_at` の境界、`since = None`、既存ジョブの除外）。
- デーモンの初回スキャン／ScanRoot／監視のテスト（3機能）。
- Discord 一括送信の対象判定・件数・確認・冪等性。
- AI の `Immediate` 未準備と `Waiting` 未準備の違い。
- UI のモード／対象範囲変更、有効化時刻、`Off` 時の一括処理有効。
- 自動動作と一括処理がカード化されず、ヒント行・重複ラベル・空表示がないこと。
  一括処理の対象範囲が2分割セグメントになっていること。
- 確認ダイアログが件数だけを表示し、注意書きや補足がないこと。
- #684:
  - 混合キューの並び順（`created_at` の古い順）、100件上限、3機能合計の件数。
  - 機能詳細のキューが待機・実行待ち・実行中だけを出し、失敗・完了を出さない。
  - 一時停止の保存、再起動後の維持、停止中の新規取得なし、実行中の完了、
    自動投入の継続。
  - 待機の行削除・一括削除が待機だけを対象にし、実行待ち・実行中を消さない。
    削除後に同じ写真を再投入できる。
  - Discord の送信履歴カードと「履歴を消去」がなく、失敗行がキューに出ない。
  - 通知モーダルに3機能の最新5件の失敗と再試行が出る。
  - ダッシュボードに一時停止／再開がない。
- i18n の ja / en と placeholder テスト。
- 1280×820 と狭い幅で、3カード・混合キュー・通知モーダル・
  3セットアップページを目視確認。
