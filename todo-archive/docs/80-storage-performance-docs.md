# Task 80: ストレージ・パフォーマンスドキュメントの追加

## 概要

WALグループコミットおよび並列アルゴリズムに関するドキュメントを追加する。

## タスク

### タスク1: docs/advanced/transactions.md にWALグループコミットセクションを追加

- [x] `## WAL グループコミット` セクションをファイル末尾に追加
- [x] グループコミットの概念説明
- [x] デフォルト設定（5ms / 100件）の記載
- [x] 同期モードとの違いの説明
- [x] 設定例コードブロック
- [x] パフォーマンストレードオフの表

### タスク2: docs/advanced/parallelization.md を新規作成

- [x] frontmatter（title, parent, nav_order: 6）
- [x] 自動並列化の概要（Rayonを使用）
- [x] 500ノード閾値の説明
- [x] 各アルゴリズムの並列化有無の表
- [x] スレッド数制御の方法（RAYON_NUM_THREADS環境変数）
- [x] パフォーマンス特性

## 参照ファイル

- `crates/maharit-storage/src/wal_group_commit.rs`
- `crates/maharit-core/src/algorithms.rs`
- `docs/advanced/transactions.md`

## ステータス

- [x] 着手中
- [x] 完了
