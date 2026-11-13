# ADR-0020: 圧縮設定を機能ごとに分離し、プロファイルを手動圧縮専用にする

- Status: Accepted
- Date: 2026-11-13
- Supersedes in part: [ADR-0013](0013-auto-compression-policy.md), [ADR-0014](0014-manual-compression-policy.md)

## Context

自動圧縮・クリップボード・Discord がプロファイルの役割フラグを共有していたため、
プロファイルの編集・削除・並び替えが3機能の挙動を変えていた。また、手動圧縮の
`replace` が自動圧縮と同じ置換経路を使い、表示用派生の選択がプロファイルの
並び順に依存していた。

## Decision

- `AppSettings` に `auto_compression` / `clipboard_compression` / `discord_compression` を持つ。
- 自動圧縮の出力先は元画像の隣の `compressed/` に固定する。機能設定に出力先を持たせない。
- クリップボードの設定はリサイズだけを持ち、出力は常に PNG とする。
- Discord は送信時に専用設定でエンコードし、ファイルを保存せず派生ファイルにも登録しない。
- `CompressionProfile` の役割フラグ `auto_enabled` / `clipboard` / `discord` を廃止し、
  プロファイルは手動圧縮専用にする。
- 表示用派生は自動圧縮だけが作る。手動圧縮は常に元画像を保持し、表示ファイルを切り替えない。
- schema v19 で `compression_jobs.origin` と `derivatives.origin` を追加し、
  `profile_id` を nullable にする。既存 v18 DB は移行せず作り直す。

## Consequences

- プロファイルの操作が3機能に影響しない。
- 自動圧縮はプロファイルが0件でも動く。
- 手動圧縮は元画像を置換しないため、表示ファイルの切り替えは自動圧縮だけに限定される。
- Discord とクリップボードは写真フォルダに中間ファイルを残さない。
- 既存 v18 DB は作り直しが必要。

## Alternatives

- 非表示プロファイル行として3機能の設定を保存する: プロファイルの意味が曖昧になり、
  削除・並び替えの対象から除外する特別扱いも必要になるため不採用。
- クリップボードで形式・画質を反映する: OS のクリップボードが PNG 前提で、
  形式変換のために二重エンコードする必要があるため不採用。
- スキーマを変えずに予約プロファイルIDで自動圧縮を表す: プロファイル非依存の
  要件と `profile_id` の意味が衝突するため不採用。
