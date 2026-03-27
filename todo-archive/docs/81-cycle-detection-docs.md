# Task 81: サイクル検出アルゴリズムのドキュメント追加

## 概要

サイクル検出・トポロジカルソート機能は実装されているがドキュメントがないため追加する。

## タスク

### タスク1: docs/algorithms/cycle-detection.md を新規作成

- [ ] frontmatter（title, parent, nav_order: 5）
- [ ] 概要（DAG検証、依存解決などのユースケース）
- [ ] `has_cycle()` の説明・Rust使用例
- [ ] `find_cycles()` の説明・Rust使用例（返り値の形式も説明）
- [ ] `topological_sort()` の説明・Rust使用例（Noneの意味も説明）
- [ ] ユースケース例（タスク依存グラフ、パッケージ依存など）
- [ ] パフォーマンス（計算量: O(V+E)）

### タスク2: docs/algorithms/index.md を更新

- [ ] サイクル検出ページへの言及を追加（既存アルゴリズムと同じ形式）

## 参照ファイル

- `crates/maharit-core/src/algorithms.rs`
- `docs/algorithms/index.md`

## ステータス

- [ ] 着手中
- [ ] 完了
