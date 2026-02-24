# タスク36: 文字列演算子 (STARTS WITH, ENDS WITH, IS NORMALIZED)

## 概要
Cypherの文字列比較演算子 STARTS WITH, ENDS WITH, IS NORMALIZED を追加する。

## 変更対象
- `crates/maharit-query/src/ast.rs` - BinaryOp / UnaryOp 拡張
- `crates/maharit-query/src/lexer.rs` - トークン追加
- `crates/maharit-query/src/parser.rs` - parse_comparison() 拡張
- `crates/maharit-query/src/executor.rs` - 評価ロジック + テスト
- `crates/maharit-query/Cargo.toml` - unicode-normalization 依存追加
- `Cargo.toml` - workspace依存追加

## ステータス
- [x] 完了
