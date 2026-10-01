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

- [x] `cargo test --workspace` が CI で通ること
- [x] `smoke_test.py` が CI で通ること
- [x] `replication_test.py` が CI で通ること
- [x] PR に対して自動でテストが走ること

## 完了内容 (2026-10-01)

`.github/workflows/test.yml`（push to main / PR to main / 手動。docs・todo・*.md のみの変更では走らない）

| ジョブ | 内容 | 所要（目安） |
|---|---|---|
| fmt + clippy | `cargo fmt --check`、`cargo clippy --workspace --all-targets -D warnings` | 約 1 分 |
| cargo test | `cargo test --workspace`（差分テスト・ストレステスト・Cypher 適合表を含む） | 約 3 分 |
| E2E | リリースビルド → `scripts/run_e2e_local.sh --replication-runs 3`（単一サーバー 5 スイート＋永続化＋レプリケーション 3 回＋フェイルオーバー）。失敗時はログを artifact 化 | 約 2〜3 分 |
| soak（main / 手動のみ） | 差分テスト 15 seeds、ストレステスト 5 seeds × 300 ops | 約 7 分 |

当初案の Docker compose ではなく、ビルドしたバイナリを直接起動する方式にした（速く、ローカルでも同じ手順を `bash scripts/run_e2e_local.sh` で再現できる）。

### 付随変更
- E2E スクリプトに `MAHARIT_E2E_LOCAL=1` で Docker 起動確認を省略する分岐
- `start_replication_local.sh`: 固定 sleep をやめ、ポートとフォロワー接続（stats の follower_count）を確認して待機（3 秒 → 0.6 秒、CI でのフレーク防止）
- `rust-toolchain.toml` で Rust 1.93.0 に固定（CI 初回は stable=1.98.1 の新 lint で失敗した。上げる時はワークフローと同時に意図的に更新）

### CI で初めて見つかったもの
- Linux 専用コード（`metrics.rs` の `cfg(target_os = "linux")`）の clippy 違反: macOS の clippy では検査されない
