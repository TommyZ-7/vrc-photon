# データベースの初期化と schema バージョン

## 概要

VRC PhotoN は SQLite の `user_version` でライブラリ DB の schema バージョンを管理する。現在のビルドが扱うのは **v1** で、`SCHEMA_VERSION = 1` がこのビルドの最大値でもある。新規 DB は v1 の初期スキーマを 1 回のトランザクションで作成する。1.0.0 未満の DB は存在しない前提とし、0.x からの移行は実装しない。

## 新規ライブラリ

`Db::open` は次の順で DB を開く。

1. `user_version == 0` かつユーザーテーブルが 1 つもない場合、`SCHEMA_V1` を `IMMEDIATE` トランザクションで実行し、`user_version = 1` にする。
2. 同じトランザクションで `schema_migrations` に `version = 1` と `applied_at` を記録する。
3. 初期化は 1 トランザクションなので、途中で失敗した場合はテーブルが部分作成されない。
4. 複数プロセスが同時に新規 DB を開いても、書き込みロック取得後に `user_version` を再確認するため、二重初期化にはならない。

## 既存ライブラリ

| `user_version` | 動作 |
|---|---|
| 1 | そのまま開く。 |
| 2 以上 | このビルドより新しい schema としてエラーを返す。VRC PhotoN を更新する。 |
| 0（テーブルあり） | 不明な初期化状態としてエラーを返す。ファイルは変更しない。 |

1.0.0 未満の DB は存在しない前提で、0.x の DB は自動移行しない。旧 0.x の `user_version`（18/20/21 など）は v1 より大きいため「新しすぎる DB」として拒否される。必要ならバックアップして新しいライブラリを作成する。

読み取り専用接続 (`Db::open_read_only`) は移行せず、`user_version = 1` の DB だけを受け付ける。v1 以外では書き込みを行わずにエラーを返す。

## 段階的マイグレーション

将来 schema を変更するときは、次の手順にする。

1. `SCHEMA_VERSION` を 1 つ上げる。
2. `MIGRATIONS` に、直前の版から 1 つだけ上げる SQL を追加する。
3. `SCHEMA_V1` は変更しない。

起動時は `user_version` の続きから `SCHEMA_VERSION` まで、`MIGRATIONS` を 1 版ずつ適用する。新規 DB も v1 作成後に同じステップを適用する。移行は `IMMEDIATE` トランザクション内で行い、ロック取得後に `user_version` を再確認する。各ステップは SQL 実行、`schema_migrations` への記録、`user_version` の更新を同じトランザクションで行い、途中で失敗したら全体をロールバックする。

1.0.0 時点の `MIGRATIONS` は 0 件で、v1 だけが存在する。

## 初期化 SQL の保守

`SCHEMA_V1`、`MIGRATIONS`、`SCHEMA_VERSION` は `crates/core/src/db/schema.rs` にある。`SCHEMA_V1` は v1 の凍結スキーマであり、将来の変更で書き換えない。schema を変更するときは `MIGRATIONS` と `SCHEMA_VERSION` を更新し、`scripts/check-docs-drift.sh` が参照するドキュメントの値を同じ PR で更新する。
