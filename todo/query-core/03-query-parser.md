# クエリ言語パーサー

**Status**: Completed

## 概要
レクサーが生成したトークン列からAST（抽象構文木）を構築するパーサーを実装する。

## 実装内容

### AST定義
- [x] `Statement` - クエリ文全体
- [x] `Pattern` - ノード・エッジのマッチングパターン
- [x] `NodePattern` - `(variable:Label {props})`
- [x] `EdgePattern` - `-[variable:TYPE {props}]->`
- [x] `Expression` - 式（比較、算術、論理）
- [x] `Clause` - MATCH, WHERE, RETURN, CREATE等

### パーサー実装
- [x] 再帰下降パーサー or Pratt parser
- [x] CREATE文のパース
- [x] MATCH文のパース
- [x] WHERE句のパース
- [x] RETURN句のパース
- [x] DELETE文のパース
- [x] SET句のパース

### エラー処理
- [x] 構文エラーの報告
- [x] エラー回復（可能な範囲で）

## 依存
- `02-query-lexer.md` が完了していること

## 対象クレート
`maharit-query`
