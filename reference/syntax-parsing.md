# Parsing

Source: https://neo4j.com/docs/cypher-manual/current/syntax/parsing/

## 概要
- Cypherは入力 `STRING` をパースしてクエリを解釈する。
- Unicodeやホワイトスペース/改行の扱いが定義されている。

## Unicodeの扱い
- 文字は `\uxxxx` でエスケープ可能。
- 例: `\u00B0` で `º` を検索。
- 使用するUnicodeバージョンはJVMに依存（Java 8/11/17/21で異なる）。

## ホワイトスペース
- キーワード間の区切りとして使用可能で、意味は持たない。
- 複数の空白は1つの空白と同等。
- 対応するUnicode空白コードポイントが広範に定義されている。

## 改行
- 改行も空白として扱われる。
- `\n`, `\r`, `\r\n` がサポート対象。

## 参照
- 文字列リテラルのエスケープ: https://neo4j.com/docs/cypher-manual/current/values-and-types/boolean-numeric-string/#string-literal-escape-sequences
- 名前のエスケープ: https://neo4j.com/docs/cypher-manual/current/syntax/naming/#symbolic-names-escaping-rules
- 正規表現: https://neo4j.com/docs/cypher-manual/current/expressions/predicates/string-operators/#regular-expressions
