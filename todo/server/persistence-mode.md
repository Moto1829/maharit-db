# サーバーに永続化を追加する（常にファイルベース）

## ステータス: 完了

## 内容
サーバーは常に永続化モードで動作する。SQLite と同様にファイルパスを指定して起動する。

## 実装済み内容
- main.rs: --data <PATH> オプション追加（env: MAHARIT_DATA, default: maharit.db）
  - 起動時: ファイルが存在すれば PersistentStorage::load()
  - 停止時: SIGINT/SIGTERM で PersistentStorage::save()
- tcp_server.rs: with_graph_arc() / graph_arc() メソッド追加
- docker-compose.yml: --data /data/maharit.db を追加
