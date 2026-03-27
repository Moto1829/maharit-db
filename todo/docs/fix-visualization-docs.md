# 可視化ドキュメント修正・追加

## ステータス: 完了

## 概要

`docs/visualization/` の可視化ドキュメントを実装と一致するよう修正し、欠けていた ASCII 可視化ドキュメントを追加。

## 変更内容

### タスク1: docs/visualization/export.md 修正

- DOT コード例: `DotExporter::new().with_*()` を削除し、実際の静的メソッド `DotExporter::export()` / `DotExporter::export_with_style()` を使ったコード例に変更
- SVG コード例: `SvgExporter::new().with_*()` を削除し、`SvgExporter::default()` と構造体直接構築（`SvgExporter { layout: ForceDirectedLayout { ... }, ... }`）を使ったコード例に変更
- インタラクティブ SVG・JSON エクスポート・サブグラフエクスポートのセクション（実装に存在しない機能）を削除

### タスク2: docs/visualization/ascii.md 新規作成

- `AsciiRenderer::render` / `render_tree` / `render_layers` の説明と使用例を記載
- 実際のASCII出力のサンプルを掲載
- 循環参照検出（`[circular]` 表示）の説明を追加
- ボックス描画文字（`├` `└` `│` `─`）の説明テーブルを追加

### タスク3: docs/visualization/index.md 更新

- 機能一覧テーブルを追加（DOT/SVG エクスポート・ASCII テキスト可視化へのリンク含む）
- ASCII 可視化への言及を追加

## 関連ファイル

- `docs/visualization/export.md`
- `docs/visualization/ascii.md`（新規）
- `docs/visualization/index.md`
