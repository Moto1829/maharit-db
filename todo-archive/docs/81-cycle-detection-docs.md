# Task 81: サイクル検出アルゴリズムのドキュメント追加

## 概要

サイクル検出・トポロジカルソート機能は実装されているがドキュメントがないため追加する。

## タスク

### タスク1: docs/algorithms/cycle-detection.md を新規作成

- [x] frontmatter（title, parent, nav_order: 5）
- [x] 概要（DAG検証、依存解決などのユースケース）
- [x] `has_cycle()` の説明・Rust使用例
- [x] `find_cycles()` の説明・Rust使用例（返り値の形式も説明）
- [x] `topological_sort()` の説明・Rust使用例（Noneの意味も説明）
- [x] ユースケース例（タスク依存グラフ、パッケージ依存など）
- [x] パフォーマンス（計算量: O(V+E)）

### タスク2: docs/algorithms/index.md を更新

- [x] サイクル検出ページへの言及を追加（既存アルゴリズムと同じ形式）

## 参照ファイル

- `crates/maharit-core/src/algorithms.rs`
- `docs/algorithms/index.md`

## ステータス

- [x] 着手中
- [x] 完了
