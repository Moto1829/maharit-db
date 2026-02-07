# Non-linear Patterns

Source: https://neo4j.com/docs/cypher-manual/current/patterns/non-linear-patterns/

## Equijoins
- 同一変数を複数のノード/関係パターンで使うことで同一要素に一致。
- サイクルや往復経路などを表現可能。

## Graph patterns
- 複数パスパターンをカンマ区切りで組み合わせる。
- 共有変数があると**結合**、なければ**直積**。
- 非線形な経路や複数レッグの移動を表現できる。

## Match modeとの関係
- 既定の `DIFFERENT RELATIONSHIPS` では関係の再訪不可。
- `REPEATABLE ELEMENTS` は再訪可能で非線形探索が広がる。
