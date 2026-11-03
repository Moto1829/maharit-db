# タスク 131: メンテ終了の bincode 1 からの移行を検討する

## 概要（2026-11-03）

`bincode` 1.3.3 はメンテナンス終了（RUSTSEC-2025-0141。2.x 系も含めて開発停止が告知されている）。
`crates/maharit-storage/src/persistence.rs` の保存形式（`PersistentStorage::load` / `save`）に使っており、
バックアップファイルもこの形式に依存している。

## 検討すること
- 移行先（`postcard`、`rmp-serde`、`wincode` など）の比較：サイズ・速度・フォーマットの安定性
- 既存の DB ファイル・バックアップを読めるようにする方法：ファイル先頭にフォーマットのバージョンを持たせ、
  旧形式は bincode 1 で読んで新形式で書き直す、など
- `Backup::verify` / `restore` と persistence_test.py（E2E）での互換性確認
- 読み込み時の入力サイズ上限（壊れたファイルや悪意のあるファイルで巨大な確保をしない）

## 備考
現時点で既知の脆弱性は無いので、緊急度は低い。保存形式の変更になるので、変えるなら MINOR 以上のリリースで行う。

## 対応内容（2026-11-03）

### 方針
別の形式（postcard など）へ移行するのではなく、**v4 のバイト列をそのまま読み書きする小さなコーデックを自前で書き、bincode を外した**。
bincode 1 の既定形式（固定長リトルエンディアン、長さは u64、enum は u32 のタグ）は単純で、対象も
SNode / SEdge / SIndex / SSnapshot と 8 種類の PropertyValue だけなので、自前で書いても約 200 行で済む。
ファイル形式が変わらないため、既存の DB ファイル・バックアップの移行や、バージョン混在の問題が起きない。
将来形式を変える場合は、既存のヘッダのバージョン番号（`VERSION`）で切り替えられる。

### 変更
- `crates/maharit-storage/src/snapshot_codec.rs` を追加し、`persistence.rs` の bincode 呼び出しを置き換え。
  ワークスペースと maharit-storage の依存から bincode を削除（`Cargo.lock` からも消えた）
- 読み込みの堅牢化: すべての要素数・文字列長を残りのバイト数と照合し、ヘッダのペイロード長は `take` で読む。
  壊れた・悪意のあるファイルで巨大なメモリ確保をせず `CorruptedData` を返す。不正な bool・UTF-8・型タグ・末尾の余分なバイトもエラー
  （従来は `vec![0u8; payload_len]` でヘッダの長さ分をそのまま確保していた）

### 互換性の確認
- bincode 版のコードで書いたフィクスチャ `crates/maharit-storage/tests/fixtures/v4_one_index.mgdb` /
  `v4_two_indexes.mgdb`（全 PropertyValue 種別、ラベルなしノード、エッジのプロパティ、unique / 非 unique のインデックス）を追加
  - 読み込んだ内容がすべて期待どおり（`Graph` と `ConcurrentGraph` の両方）
  - 読み込んだグラフを書き直すと、bincode が書いたファイルとバイト単位で一致
- 旧リリースバイナリ（bincode）が書いた実際の DB ファイルを新ビルドで読み、ノードを追加して保存したファイルを旧バイナリで読めることを確認
- 壊れたファイル（巨大なペイロード長・巨大な要素数・途中で切れたデータ・末尾の余分なバイト・未知の型タグ）がエラーになるテストを追加
- 検証: fmt・clippy クリーン、`cargo test --workspace` 全通過（1157 件）、`bash scripts/run_e2e_local.sh` 全スイート通過、
  `cargo audit` の警告は lindera 配下の 2 件のみ（task 132）

## ステータス
完了
