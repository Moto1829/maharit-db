# タスク 157: Docker イメージの既定コマンドで外から接続できず、データがボリュームに入らない

## 現象（2026-11-12、task148 の docs 照合で発見）
- `Dockerfile` の `CMD ["/app/maharit", "server"]` は `127.0.0.1:7687` に bind するため、`docker run -p 7687:7687 maharit-db` で外から接続できない
- `ENV MAHARIT_DATA_DIR=/data` はバイナリが読まない（読むのは `MAHARIT_DATA`）。データファイルは `/app/maharit.db` になり、`/app` は root 所有なので保存にも失敗する
- 監視用 HTTP も既定の `127.0.0.1:9090` で、compose の `9090:9090` が届かない
- compose の `maharit`（REPL）サービスは command が無く、実際にはサーバーとして起動していた

## 対応
- [x] `ENV MAHARIT_DATA=/data/maharit.db`、`CMD [... "server", "--host", "0.0.0.0", "--metrics-bind", "0.0.0.0:9090"]`
- [x] compose: サーバーに `--metrics-bind 0.0.0.0:9090`、REPL サービスに `command: ["/app/maharit"]`
- Docker デーモンの無い環境で作業したため、イメージのビルドは未確認（`docker compose config` は通過）
