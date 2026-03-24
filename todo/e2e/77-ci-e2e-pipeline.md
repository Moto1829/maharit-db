# Task 77: CI/CD E2E テストパイプライン

## 背景・目的

現在 `.github/workflows/` には `docs.yml`（ドキュメントビルド）のみ存在し、
Rust テストの自動実行も Python E2E スクリプトの CI 統合もない。

今回の不具合はコードレビューや手動テストでは見逃しやすいタイプで、
CI に E2E テストを組み込むことで PR マージ前に検出できるようになる。

## 実装内容

### ファイル: `.github/workflows/test.yml`

#### job 1: unit-test
```yaml
- name: Run unit tests
  run: cargo test --workspace
```

#### job 2: integration-test（smoke test）
```yaml
- name: Build Docker image
  run: docker compose build

- name: Start server
  run: docker compose up -d maharit-server

- name: Wait for healthy
  run: |
    for i in $(seq 1 30); do
      docker compose ps | grep "healthy" && break
      sleep 2
    done

- name: Run smoke test
  run: python3 scripts/smoke_test.py

- name: Stop server
  run: docker compose down
```

#### job 3: replication-test
```yaml
- name: Build replication images
  run: docker compose -f docker-compose.replication.yml build

- name: Start replication cluster
  run: docker compose -f docker-compose.replication.yml up -d

- name: Wait for all nodes healthy
  run: |
    for i in $(seq 1 60); do
      HEALTHY=$(docker compose -f docker-compose.replication.yml ps \
        | grep -c "healthy")
      [ "$HEALTHY" -eq 3 ] && break
      sleep 2
    done

- name: Run replication test
  run: python3 scripts/replication_test.py

- name: Stop cluster
  run: docker compose -f docker-compose.replication.yml down -v
```

### トリガー設定
- `push` to `main`
- `pull_request` to `main`

### キャッシュ設定
- `~/.cargo/registry` をキャッシュして Rust ビルド高速化
- Docker layer キャッシュ（`docker/build-push-action` の `cache-from`）

## 注意点

- replication-test は Docker ビルドを含むため時間がかかる
  → `pull_request` では unit-test + integration-test のみ実行し、
    replication-test は `push to main` のみにする選択肢もある
- フォロワーが起動後すぐに接続拒否する問題（今回の教訓）があるため、
  healthy 確認ループは十分な待機時間を設ける

## 完了条件

- [ ] `cargo test --workspace` が CI で通ること
- [ ] `smoke_test.py` が CI で通ること
- [ ] `replication_test.py` が CI で通ること
- [ ] PR に対して自動でテストが走ること
