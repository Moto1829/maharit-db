# タスク 125: CALL サブクエリ内で終点だけ束縛済みのパターンが誤った行を返す

## 概要（2026-11-03、task 123 の作業中に発見。変更前のコードでも再現）

`WITH p` で取り込んだ変数をパターンの終点に置くと、束縛済みの値と照合されずに
別のノードが返る（束縛が上書きされているように見える）。

## 再現

conformance_tests の FIXTURE（Alice-KNOWS->Bob-KNOWS->Carol）で:

```cypher
MATCH (p:Person {name: 'Alice'})
CALL { WITH p MATCH (f)-[:KNOWS]->(p) RETURN f.name AS name }
RETURN p.name, name
```

- 期待: 0 行（Alice を KNOWS している人はいない）
- 実際: `["Alice","Alice"]`, `["Alice","Bob"]`

## 調査の手がかり
- `match_path_pattern` → `match_single_hop` で、終点ノードの変数が既に束縛されている場合の
  照合（`node_matches_pattern`）を確認する。
- 通常の MATCH で同じ形（始点が未束縛・終点が束縛済み）が再現するかも確認する。
