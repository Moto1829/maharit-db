# String functions

Source: https://neo4j.com/docs/cypher-manual/current/functions/string/

## 主要関数
- `btrim`, `ltrim`, `rtrim`, `trim`
- `left`, `right`, `substring`, `split`, `replace`（limitは2025.06）
- `lower`/`upper` と `toLower`/`toUpper`
- `normalize`（NFC/NFD/NFKC/NFKD）
- `reverse`
- `toString` / `toStringOrNull`

## 重要ルール
- 文字列関数はUnicode文字単位。
- `toString` は数値/真偽/空間/時間型も対応。
