# グラフ可視化

**Status**: Completed

## 概要
グラフをビジュアルに表示するための機能を実装する。

## 実装内容

### DOT形式出力
- [x] Graphviz DOT形式への変換
- [x] ノードラベル/プロパティの表示
- [x] エッジラベルの表示
- [x] スタイル設定（色、形状）

### ASCII表示
- [x] ターミナルでの簡易グラフ表示
- [x] ツリー構造の表示

### SVG出力（将来的）
- [x] 力学モデルによるレイアウト
- [x] SVGファイル生成

### Web UI（将来的）
- [x] WebSocketによるリアルタイム表示
- [x] インタラクティブな操作

### API
```rust
let dot = DotExporter::export(&graph);
let ascii = AsciiRenderer::render(&graph);
let tree = AsciiRenderer::render_tree(&graph, root, max_depth);
```

## 対象クレート
`maharit-viz`
