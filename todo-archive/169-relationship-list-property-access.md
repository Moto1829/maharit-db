# タスク 169: relationships(p) の要素からプロパティを参照できない

## 現象（2026-11-12、task140 で shortest-path.md の例を確認）
```text
MATCH p = (a:City {name: "Tokyo"})-[:ROAD*1..5]->(b:City {name: "Osaka"})
RETURN reduce(total = 0, r IN relationships(p) | total + r.distance)
→ Execution error: type error: arithmetic requires numbers
```
`relationships(p)` や `collect(r)` の要素はリレーションシップの ID（整数）の値になっていて、`r.distance` が null になる。
ノードの値（`nodes(p)` の要素）は `n.name` が使える。経路の重みの合計を Cypher で求められない。

## 作業
- [ ] `Value` にリレーションシップの値（`Value::Relationship(id)` など）を持たせ、プロパティ参照・`type(r)` を使えるようにする
- [ ] `RETURN r` の出力形式（プロトコル上の表現）を決める
- [ ] docs/algorithms/shortest-path.md に Cypher での重みの合計の例を戻す
