# String operators

Source: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/string-operators/

## 演算子
- `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~`
- `IS NORMALIZED`, `IS NOT NORMALIZED`

## 仕様
- すべて**大小区別**。
- `STRING` 以外に適用すると `null`。

## 正規表現
- Java正規表現に準拠。
- `(?i)` などのフラグは先頭に付与。
- エスケープは**文字列リテラル**と**正規表現**の二重解釈に注意。

## 正規化
- 既定の正規化形式は `NFC`。
- `IS NORMALIZED` / `IS NOT NORMALIZED` は形式指定（`NFD`/`NFKC`/`NFKD`）が可能。
- 非`STRING`では `null`。

## 備考
- 正規表現のユーザー入力はCypherインジェクションに注意（パラメータ推奨）。
