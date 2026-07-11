# query-core/86: フィルタ無し COUNT の短絡（性能改善 6）

## 概要
`MATCH (n:L) RETURN count(*)` / `count(n)` でも、MATCH が全ノードの binding を
実体化してから件数を数えていた（`COUNT(*)` 自体は O(1) なのに、手前の binding
構築が支配的）。after ベンチで `AGG COUNT all` が 1,000件で 2.76ms と突出していた。

## 対応
- `execute_match` 冒頭に `try_simple_count` を追加。以下をすべて満たす時のみ、
  `nodes_by_label(L).len()` / `node_count()` から**binding を作らず即答**する:
  - 単一セグメント・単一 MATCH（非 OPTIONAL）・単一ノードパターン
  - WHERE / WITH / CALL なし
  - インラインプロパティなし・ラベル 0 or 1 個
  - ORDER BY / SKIP / LIMIT なし
  - 全 RETURN 項目が `count(*)` または `count(<ノード変数>)`（AS エイリアス可）
- 上記以外は従来パス（`None` を返してフォールバック）。
- ユニットテスト追加: count(*)/count(n)/ラベル無し/AS で短絡し正しい件数、
  WHERE・インラインプロパティ時は短絡せず正しくフィルタされること。

## 効果
- フィルタ無し件数クエリが O(1)/O(該当ラベル数) の即答になり、
  大規模ノードでの `count` レイテンシが激減（実測は再ビルド後ベンチで確認予定）。

## ステータス
完了（query 505 テストパス）
