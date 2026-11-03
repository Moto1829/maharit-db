# タスク 129: quick-xml を 0.41 以上へ上げる（優先度: 高）

## 概要（2026-11-03）

maharit-io の GraphML 入出力（`crates/maharit-io/src/graphml_io.rs`）で使っている quick-xml 0.37.5 に、
サービス拒否につながる脆弱性が 2 件ある。どちらも互換範囲外なので `cargo update` では直らない。

- RUSTSEC-2026-0195（high）: `NsReader` で名前空間宣言を際限なく確保し、メモリを使い果たす
- RUSTSEC-2026-0194（high）: 開始タグの属性名の重複チェックが入力サイズの 2 乗の時間になる

信頼できない GraphML ファイルを読み込むと、プロセスのメモリや CPU を使い果たされうる。

## 作業
- ワークスペースの `quick-xml = { version = "0.37", ... }` を 0.41 以上（2026-11 時点の最新は 0.42）に上げる
- API の変更（`serialize` feature、`Reader` / `Event` 周り）に追従する
- maharit-io の GraphML テストを通す。大きな属性数や名前空間を含む入力でのテストも足す
