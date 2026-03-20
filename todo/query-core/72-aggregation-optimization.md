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
- [ ] `count(n)` / `COUNT(*)` をノードリスト長の返却に最適化（プロパティ射影スキップ）
- [ ] `maharit-query/src/executor.rs` の `execute_return` でカウント専用パスを追加

### GROUP BY ハッシュテーブル最適化
- [ ] 集計キーの正規化処理（文字列のクローン・ハッシュ計算）のコストを計測
- [ ] `String` キーを `Arc<str>` や intern 済みの ID に置き換えて重複排除
- [ ] `COUNT per skill` と `COUNT per city` の差の原因を特定（プロファイリング）

### 集計演算子の事前計画化
- [ ] `planner.rs` で GROUP BY を含むクエリにハッシュ集計プランを生成
- [ ] 小カーディナリティ（< 100 グループ）では配列ソートを、大カーディナリティでは HashMap を選択

## 関連ファイル
- `crates/maharit-query/src/executor.rs` — 集計処理
- `crates/maharit-query/src/planner.rs` — クエリプラン

## ステータス
未着手
