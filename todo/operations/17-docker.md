# 17-docker: Docker対応

## 概要
maharit-dbをDockerコンテナで実行できるようにする。

## 実装内容

### Dockerfile
- [x] マルチステージビルド（ビルドステージ + 実行ステージ）
- [x] Rustの公式イメージをベースに使用
- [x] 最小限の実行イメージ（debian-slim）
- [x] 非rootユーザーでの実行

### docker-compose.yml
- [x] サーバー起動設定
- [x] ボリュームマウント（データ永続化）
- [x] ポートマッピング（TCPサーバー実装後）
- [x] 環境変数設定

### 設定
- [x] 環境変数による設定（MAHARIT_DATA_DIR）
- [ ] ヘルスチェックエンドポイント（将来的）

### ドキュメント
- [x] Docker環境での起動方法
- [x] docker-composeでの起動方法
- [x] 設定オプションの説明

ドキュメント: `docs/docker.md`

## 作成ファイル
- `Dockerfile`
- `docker-compose.yml`
- `.dockerignore`

## 検証方法
```bash
# イメージのビルド
docker build -t maharit-db .

# コンテナの起動（REPL）
docker run -it maharit-db

# docker-composeでの起動
docker-compose run maharit
```

## 備考
- TCPサーバー（12-tcp-server）の実装完了後にポートマッピングとヘルスチェックを追加
- 現時点ではREPLの実行環境として使用可能
