# タスク 142: DETACH DELETE で消えたリレーションシップが deleted_edges に数えられない

## 現象
```text
maharit> CREATE (a:P {n:1})-[:K]->(b:P {n:2})
maharit> MATCH (n:P {n:2}) DETACH DELETE n
deleted_nodes | deleted_edges
--------------------------------
1 | 0
maharit> MATCH ()-[r]->() RETURN count(r) AS c
0
```
リレーションシップは削除されているが、件数は 0 と報告される（2026-11-07、v0.2.0 のリリースビルドで確認）。

## 期待
`deleted_edges` に、ノードと一緒に削除したリレーションシップの数（この例では 1）が入る。

## 作業
- [ ] executor の DETACH DELETE で、ノード削除に伴って消えたエッジ数を数える（TCP サーバー経路の `RecordingGraph` でも同じ値になること）
- [ ] テスト追加
- [ ] `docs/quickstart.md`（と `quickstart.en.md`）の注記を削除し、出力例を `1 | 1` にする
