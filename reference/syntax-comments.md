# Comments

Source: https://neo4j.com/docs/cypher-manual/current/syntax/comments/

## 概要
- コメントは実行されず、可読性向上のために利用する。

## 形式
- 単一行コメント: `//` から行末まで。
- 複数行コメント: `/* ... */`。

## 例
- `MATCH (n) RETURN n // end-of-line comment`
- `MATCH (n) /* multi-line
comment */ RETURN n`
- 文字列内の `//` はコメントではない。
