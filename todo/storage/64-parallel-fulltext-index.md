# 全文検索インデックスの並列構築（Rayon）

## 概要

全文検索インデックスの構築（トークン化・BM25スコア計算）が逐次実行。
lindera による日本語形態素解析は CPU バウンドの処理であり、
`rayon` による並列化で最大 4〜8倍の高速化が見込める。

## 現状の問題

ノードのインデックス追加が逐次的に行われており、
日本語テキストを含む大量ノードのインデックス構築で時間がかかる。

```rust
// 推定: 逐次的なインデックス更新
for node in graph.nodes() {
    let tokens = tokenize(text);    // lindera: CPU バウンド・独立
    // inverted_index に追加
}
```

## 実装内容

- [ ] ノードのトークン化フェーズを並列化
  ```rust
  // フェーズ1: 並列トークン化
  let tokenized: Vec<(NodeId, Vec<String>)> = nodes
      .par_iter()
      .map(|node| {
          let tokens = tokenize(&node.properties);  // 独立・CPU バウンド
          (node.id, tokens)
      })
      .collect();

  // フェーズ2: 逐次インデックス更新（HashMap への書き込みは逐次）
  for (node_id, tokens) in tokenized {
      for token in tokens {
          inverted_index.entry(token).or_default().insert(node_id);
      }
  }
  ```
- [ ] BM25 スコア計算も並列化（TF 計算はノードごとに独立）
- [ ] 複数ノードの一括インデックス追加 API を追加
  （現状が 1 件ずつ追加のみなら）
- [ ] `rayon` を `maharit-core/Cargo.toml` に追加（63 と共通）

## 注意

- lindera の `Tokenizer` はスレッドセーフか確認（スレッドローカルに生成が必要な可能性あり）
- `thread_local!` で Tokenizer をキャッシュすると初期化コストを削減できる

## 期待効果

| ワークロード | 期待倍率（8コア） |
|-----------|--------------|
| 日本語テキスト（形態素解析あり） | **4〜8倍** |
| 英語テキスト（単語分割のみ） | 3〜4倍 |

## 対象クレート

`maharit-core`
