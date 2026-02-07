# Boolean, numeric, and string literals

Source: https://neo4j.com/docs/cypher-manual/current/values-and-types/boolean-numeric-string/

## Boolean
- `true` / `false`

## 数値
- `INTEGER`: 十進/16進(`0x`)/8進(`0o`)、負号可。
- `FLOAT`: 小数・指数表記・`Inf`/`Infinity`/`NaN`。
- 数字の区切りに `_` を使用可能。

## 文字列
- `'` または `"` で囲む。
- エスケープ: `\t`, `\n`, `\r`, `\f`, `\'`, `\"`, `\\`, `\uXXXX`。

## セキュリティ
- 文字列にユーザー入力を埋め込む場合はパラメータを推奨。
