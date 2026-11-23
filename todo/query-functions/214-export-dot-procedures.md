# タスク 214: DOT 形式で書き出すプロシージャ（db.export.dot / db.export.subgraph.dot）

## 背景（2026-11-23、task127 の残り）
task127（task124）で docs から外し、docs/visualization/export.md に「Cypher から呼べるエクスポート用のプロシージャはありません」と明記した。
DOT 出力の本体は `maharit-viz/src/dot.rs` の `DotExporter`（`&Graph` を取る）にある。

※ 実装するかどうかは後で決める。決めるまでは着手しない。

## 候補
- `CALL db.export.dot([config]) YIELD dot` — グラフ全体を DOT の文字列で返す
- `CALL db.export.subgraph.dot(nodes, rels [, config]) YIELD dot` — 渡したノード・リレーションシップだけ

## 課題
- `maharit-query` は `maharit-viz` に依存していない（viz は `maharit-client` にも依存する）。
  DOT 生成を `maharit-core` か `maharit-query` に移すか、サーバー側から差し込むか（task215 の仕組み）を決める
- 結果はファイルではなく文字列で返す（サーバーのファイルシステムに書かない）
- 大きなグラフでの応答サイズの上限、ACL で見えないものを出さない

## 作業
- [ ] 置き場所と依存の向きを決める
- [ ] プロシージャを追加し、`DotStyle` 相当の設定をマップで受ける
- [ ] テスト、docs/visualization/export.md（日英）を書き換える
