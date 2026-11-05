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

## 調査結果（2026-11-05）
CALL サブクエリに限らず、**通常の MATCH でも再現**していた。パターンの途中・終点に置いた変数が
すでに束縛されていても照合されず、別のノードで上書きされていた。例（FIXTURE: Alice→Bob→Carol）:

```cypher
MATCH (p:Person {name: 'Bob'}), (f)-[:KNOWS]->(p) RETURN f.name, p.name
```
期待 `[Alice, Bob]` に対し、`[Bob, Carol]` も返っていた（p が Carol に上書き）。
`(p)<-[:KNOWS]-(f)` のように束縛済みの変数を始点に置いた場合だけ正しかった。

## 対応内容
- `node_matches_pattern`: パターンの変数が束縛済みなら、そのノードと同じ場合だけ一致とする
  （単一ホップ・可変長パス・shortestPath の終点すべてに効く）
- `match_single_hop`: 束縛済みのリレーションシップ変数も同様に照合（`WITH r MATCH (x)-[r]->(y)`）
- `WITH x` / `WITH x AS y` は元の束縛をそのまま引き継ぐようにした。`Value` にリレーションシップの型が無いため、
  これまで `WITH r` を通すとエッジが整数 ID に変わり、`r.since` や再照合ができなかった
- `WITH … ORDER BY` を、WITH で付けた別名も見える範囲で評価するようにした（`WITH r AS rel ORDER BY rel.since` が並ばなかった）
- conformance テストに 6 ケース追加（終点の束縛・WITH 経由・CALL サブクエリ・可変長パス・エッジ変数の再照合・別名での ORDER BY）

## 残課題
- `Value` にリレーションシップの型が無い問題そのもの（集約の WITH や `relationships(p)` ではエッジが整数 ID になる）は
  task 127 の「関連する式の不足」に記載済み

## 検証
- clippy クリーン、`cargo test --workspace` 全通過、`bash scripts/run_e2e_local.sh` 全スイート通過

## ステータス
完了
