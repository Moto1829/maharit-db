# 集計クエリ最適化（COUNT / GROUP BY）

## 概要
`count(n)` と `n.skill, count(n)` が 44〜48 ms かかっており、
`avg(n.age)` や `n.city, count(n)`（3〜5 ms）と比べて10倍以上遅い。
集計パスを最適化して均一なレイテンシを実現する。

## 背景（ベンチマーク根拠）
| クエリ | レイテンシ |
|--------|--------:|
| AGG COUNT all | 47.71 ms |
| AGG COUNT per skill | 44.09 ms |
| AGG AVG age | 4.66 ms |
| AGG COUNT per city | 3.31 ms |

`skill` と `city` は同じ10値なのに差が大きい。
`count(n)` はプロパティ射影が不要なはずなのに遅い。

## 実装内容

### COUNT(*) ショートサーキット
- [x] `count(n)` をノードバインディング確認のみで完結（NodeData 生成をスキップ）
- [x] `COUNT(*)` は `bindings_list.len()` を直接返す（既存動作を確認・維持）

### GROUP BY ハッシュテーブル実装
- [x] `build_aggregated_result_set` に GROUP BY ロジックを実装
  - 非集計項目を暗黙のグループキーとして扱う（Cypher セマンティクス）
  - 挿入順保持の HashMap で各グループのバインディングインデックスを蓄積
  - グループごとに集計関数を適用し1行を生成
  - ORDER BY / SKIP / LIMIT をグループ化後に適用
- [x] `count(n)` カラム名を `"count(n)"` に修正（従来は誤って `"COUNT(*)"` だった）

### 集計演算子の事前計画化
- [ ] `planner.rs` で GROUP BY を含むクエリにハッシュ集計プランを生成（未着手）

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — 集計処理
- `crates/maharit-query/src/planner.rs` — クエリプラン

## ステータス
実装完了（planner の GROUP BY プランノードは未着手）。456 tests passing。
