# Type predicate expressions

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/type-predicate-expressions/

## 構文
- `<expr> IS :: <TYPE>` / `<expr> IS NOT :: <TYPE>`
- 代替構文: `IS TYPED`, `IS NOT TYPED`, `:: <TYPE>`

## `null` の扱い
- すべての型に `null` が含まれるため、`IS ::` は `null` でも `true`。
- `NOT NULL` を付与して `null` を排除可能。
- `NULL` 型で `null` のみ判定可能。

## 型の合成
- `ANY` は全型、`NOTHING` は空集合。
- 閉じたユニオン `INTEGER | FLOAT` をサポート。
- `LIST<INNER>` では**全要素が内側型に一致**する必要がある。
- 空リストは任意の内側型に一致（`LIST<NOTHING>` も `true`）。

## プロパティ向け
- `PROPERTY VALUE` でプロパティに格納可能か判定。

## 数値パラメータ
- 外部パラメータの整数サイズは区別されず `INTEGER` 扱い。

## ベクタ型（Cypher 25 / Neo4j 2025.10）
- `VECTOR<TYPE>(DIM)` と `VECTOR` のスーパータイプが使用可能。
- `VECTOR<TYPE>` / `VECTOR(DIM)` で部分一致の判定が可能。
