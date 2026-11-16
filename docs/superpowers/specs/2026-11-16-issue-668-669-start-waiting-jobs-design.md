# 手動操作で待機中の送信・タグ付けジョブを開始する (#668 / #669) 設計

> 状態: 承認済み（2026-11-16）
> 対象 Issue: #668, #669
> 実装 PR: 1本（両 Issue をまとめる）

## 目的

写真に対する手動操作が、その写真にすでに存在する待機行を無視して
「すでに処理待ちです」と案内する現状を改める。待機行がある場合はその行を
開始し、無い場合だけ従来どおり新規行を投入する。

- #668: Discord への手動送信で、`waiting` の送信行を `pending` に昇格する。
- #669: AI タグ付けの手動実行（再タグ付けを含む）で、`waiting` の
  タグ付けジョブを `pending` に昇格する。

## 現状

- Discord 手動送信は `crates/ui/src/app/discord.rs::enqueue_photo_send` が
  `Db::enqueue_send(..., Pending)` を呼ぶ。`idx_send_outbox_active_photo`
  （`waiting` / `pending` / `processing` の部分 UNIQUE インデックス）により、
  待機行があると `INSERT OR IGNORE` が 0 件になり `Ok(None)` が返る。
  現在はこれを一律「送信待ちです」の info トーストにしている。
- AI タグ付けは `crates/ui/src/app/ai.rs::enqueue_manual_retag` が
  `Db::enqueue_tagging_jobs(..., Pending)` を呼ぶ。アクティブジョブの
  部分 UNIQUE インデックスで待機ジョブがあると挿入 0 件になり、
  従来の「対象の写真はすでに処理待ちです。」になる。
- 送信行は `photo_id` / `kind` / `origin` / `status` だけを投入時に記録する。
  圧縮設定・`source`・`profile_id` は送信処理時に確定する。したがって
  「待機行に記録された内容に従う」は、行を作り直さず既存行を再利用する
  ことで満たす（圧縮設定の解決方法は変更しない）。
- タグ付けジョブは `photo_id` / `plugin_id` / `status` を保持する。
  同一写真・同一プラグインのアクティブジョブは最大1件。

## 決定事項

| # | 判断項目 | 決定 | 理由 |
|---|---|---|---|
| Q1 | 待機行の扱い | 削除・作り直さず、同じ行を `waiting` → `pending` に更新する | 行が持つ送信元・種別・作成順を維持し、重複行も作らない。 |
| Q2 | 判定順序 | 先に新規行の投入を試し、投入できなかった場合に待機行の昇格を試す | 待機行が無い通常経路の挙動と SQL を変えない。 |
| Q3 | 待機行がない場合 | 従来どおり新規 `pending` 行を投入する | Issue 指定。 |
| Q4 | 待機行が `pending` / `processing` の場合 | 従来の案内を出す | 開始も投入もできないため。 |
| Q5 | #668 のトースト | 昇格成功時は既存の成功トースト `photos.action.discord_queued` を出す | 「送信待ちです」を出さないという指定を満たす。文言追加はしない。 |
| Q6 | #669 の件数 | 「昇格した件数 + 新規投入した件数」を既存の `integrations.ai.enqueue.queued` に渡す | 実際に開始したジョブ数と一致する。 |
| Q7 | #669 で合計0件の場合 | 既存の `integrations.ai.enqueue.none` を出す | Issue 指定。 |
| Q8 | 対象プラグイン | AI プラグイン (`AI_PLUGIN_ID`) の待機ジョブだけを昇格する | 他プラグインのバックログを巻き込まない。 |
| Q9 | DB API | 送信は `Db::start_waiting_send(photo_id)`、タグ付けは `Db::start_waiting_tagging_jobs(photo_ids, plugin_id)` を追加する | UI から昇格条件を明確に呼べ、DB テストで検証できる。 |

## 仕様

### #668 Discord 手動送信

`enqueue_photo_send` はガードと写真状態の検証後、次の順で処理する。

1. `Db::enqueue_send(photo_id, Photo, Manual, Pending)` を呼ぶ。
   - `Some(id)`: 従来どおり履歴更新、`WakeSends`、成功トースト。
2. `None` の場合は `Db::start_waiting_send(photo_id)` を呼ぶ。
   - `true`: 履歴更新、`WakeSends`、成功トースト。
   - `false`: 「送信待ちです」の info トースト。
   - `Err`: エラートースト。

昇格した行は `origin` / `kind` / `photo_id` / `created_at` を維持する。
`attempts` などは待機行の初期値のまま（`waiting` 行は未着手）。

### #669 AI タグ付け手動実行

`enqueue_manual_retag` は有効性・準備完了の検証後、次の順で処理する。

1. `Db::start_waiting_tagging_jobs(photo_ids, AI_PLUGIN_ID)` で対象写真の
   待機ジョブを `pending` に昇格する。進捗・ステージ・エラー・開始/終了時刻は
   初期化する（全体リリース `release_waiting_tagging_jobs` と同じ）。
2. `Db::enqueue_tagging_jobs(photo_ids, AI_PLUGIN_ID, Pending)` で残りの写真に
   新規ジョブを投入する。
3. 合計件数を `ManualRetagOutcome::Queued(count)` として返す。

`count == 0` のときは既存の info トースト、`count > 0` のときは既存の成功
トーストを出し、`WakeTagging` を送る（`wakes()` は従来どおり）。

## 非対象

- 自動送信・自動タグ付けの投入条件。
- 待機行の一括リリース操作。
- トースト文言・配置などの見た目変更（UI モック不要）。
- DB スキーマ変更。

## テスト

- core: 待機送信行の昇格が対象写真だけに作用し、`origin` を維持すること。
  待機行がない場合・`pending` の場合は `false` を返すこと。
- core: 待機タグ付けジョブの昇格が対象写真・対象プラグインだけに作用し、
  2回目は 0 件になること。
- ui: `enqueue_manual_retag` が待機ジョブを新規作成せず昇格させること。
  すべて `pending` / `processing` の場合は `Queued(0)` になること。
  待機と新規が混在する場合は合計件数になること。
- 検証: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`。
