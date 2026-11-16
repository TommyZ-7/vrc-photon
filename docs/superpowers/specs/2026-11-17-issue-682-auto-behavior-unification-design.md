# 自動動作の設定を3機能で統一する (#682) 設計

> 状態: 計画中（2026-11-17）
> モック: `docs/ui-682-auto-behavior-unification-mockup.html`
> 対象 Issue: #682
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

## 決定事項

Issue の仕様に加え、実装前に確認した判断項目。

| # | 判断項目 | 決定 |
|---|---|---|
| Q1 | 「既存の未処理分も含める」の実行契機 | 自動では実行しない。各機能の「一括処理」ボタンを押したときだけ実行する。 |
| Q2 | 「新規」の基準 | ライブラリへの追加以降（`photos.created_at`）。Discord の撮影時刻カットオフは廃止する。 |
| Q3 | Discord の「既存の未処理分」 | まだ送信に成功しておらず、待機・実行待ち・処理中でもない写真。失敗・取消も含む。削除中は除外。 |
| Q4 | 自動圧縮カードのマスタースイッチ | 残す。マスタースイッチは機能の有効／無効、3値は自動動作の実行契機とする。 |
| Q5 | 共通コントロールの形 | 「自動動作」は3分割のセグメント、「一括処理」は対象範囲2チップ＋実行ボタンにする。 |
| Q6 | 初回セットアップ | 「自動動作」だけを追加する。対象範囲・一括処理は表示しない。 |
| Q7 | AI「即時」でプラグイン未準備 | 現行どおりジョブを積まない。準備完了後の新規写真から処理する。 |
| Q8 | Discord の既存未処理送信 | 対象件数を示す確認ダイアログを出し、OK で送信キューに積む。 |
| Q9 | 対象範囲の役割 | 「一括処理」ボタンが対象にする写真の範囲を決める。自動投入の対象は変えない。 |
| Q10 | 自動動作が「しない」のときの一括処理 | 使える。機能のマスタースイッチがオフのときだけ一括処理を無効にする。 |
| Q11 | カード内の表示順 | ヘッダー＋マスタースイッチ → 機能固有の設定 → 自動動作 → 一括処理 → 状態・履歴 の順に統一する。 |
| Q12 | AI カードの設定導線 | 「設定を開く」ボタンと、その案内文を削除する。プラグイン・バックエンド・同意は設定画面に残す。 |
| Q13 | 一括処理のラベル | 「対象範囲」ではなく「一括処理」と表示し、対象範囲チップと実行ボタンを同じブロックに置く。 |

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
pub discord_enabled: bool,         // 現行の連携スイッチ（変更なし）
pub discord_auto: AutoBehavior,    // 旧 discord_auto_mode + discord_auto_since
pub ai_tagging_enabled: bool,      // 現行の連携スイッチ（変更なし）
pub ai_tagging_auto: AutoBehavior, // 旧 ai_tagging_auto (bool)
```

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
| `discord_auto` | `Off` / `NewOnly` / `None` |
| `ai_tagging_auto` | `Off` / `NewOnly` / `None` |

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

- 自動投入: `ai_tagging_enabled = true` かつ `ai_tagging_auto.mode != Off` のとき、
  新規索引写真を自動投入する。
- `Immediate`: プラグインが準備完了のときだけ `pending` で積む。未準備なら積まない
  （現行どおり）。
- `Waiting`: プラグインが未準備でも `waiting` で積む。準備後に
  「待機中のものを処理」で開始できる。
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
   - 下に選択中の意味を1行で出す。
2. **一括処理** — 見出し、対象範囲2チップ、説明、実行ボタン。
   - チップ: 「有効化以降の新規のみ」「既存の未処理分も含める」。
   - 説明: 「一括処理の対象にする写真の範囲を選びます。」
   - ボタン: 機能ごとのラベル（未圧縮画像を一括圧縮 / 未送信を一括送信 /
     未タグ付けのものを一括処理）。
   - ボタンは機能のマスタースイッチがオンのとき使える。自動動作が `Off` でも
     使える。Discord は有効な Webhook、AI はプラグイン準備完了も必要。
   - 実行ボタンと対象範囲を同じブロックに置き、対象範囲が何に効くかを明確にする。

自動動作のセグメントは3カード＋3セットアップページで使う。一括処理ブロックは
3カードだけで使い、セットアップページには表示しない。

### 連携・自動の詳細カード — 表示順を統一

すべてのカードで次の順に並べる。

1. ヘッダー（タイトル＋マスタースイッチ）
2. 機能固有の設定
3. 自動動作（セグメント）
4. 一括処理（対象範囲＋実行ボタン）
5. 状態・履歴・ジョブ

- **自動圧縮**: ヘッダーにマスタースイッチを置く。機能固有の設定は
  「自動圧縮後に元画像を保持」、警告、圧縮設定。続けて自動動作、一括処理。
  状態・履歴はカード内に置かない（ダッシュボードのキューが担う）。
- **Discord**: ヘッダーにマスタースイッチ。機能固有の設定は Webhook、送信先、
  テスト送信、送信画像、圧縮設定。続けて自動動作、一括処理。最後に送信履歴。
- **AI タグ付け**: ヘッダーにマスタースイッチ。機能固有の設定はバックエンド。
  続けて自動動作、一括処理。最後に待機・処理中・失敗、実行中ジョブ、
  「待機中のものを処理」。設定画面へのボタンは置かない。
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
7. カード内の表示順を「ヘッダー → 機能固有 → 自動動作 → 一括処理 → 状態」に
   統一する。

## 対象外

- 圧縮ジョブ・送信 outbox・タグ付けジョブのテーブル、状態、ワーカーの統合。
- 自動圧縮の元画像保持／置換、圧縮形式・品質・リサイズ。
- Discord の送信先、Webhook、同意、送信内容、送信履歴。
- AI のプラグイン導入、モデル、バックエンド、閾値、同意。
- ダッシュボードの AI カードの導線（設定画面へのリンク）は、この Issue では
  変更しない。必要なら別 Issue で扱う。
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
- i18n の ja / en と placeholder テスト。
- 1280×820 と狭い幅で、3カードと3セットアップページを目視確認。
