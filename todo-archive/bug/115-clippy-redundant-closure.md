# タスク 115: `cargo clippy -D warnings` が失敗する（redundant_closure）

## 概要

`cargo clippy --all-targets -- -D warnings` が `maharit-query` で 2 件のエラーを出してビルド失敗する。
Bindings の `Arc<str>` 化（9b3ecfe0）以降に混入したと思われる。

```
error: redundant closure
    --> crates/maharit-query/src/executor.rs:2114:18
2114 |             .any(|item| Self::is_aggregate(item));
    --> crates/maharit-query/src/executor.rs:3285:18
3285 |             .any(|item| Self::is_aggregate(item));
```

## 修正内容

- [x] 2 箇所を `.any(Self::is_aggregate)` に置き換える
- [x] `cargo clippy --all-targets -- -D warnings` がワークスペース全体で通ることを確認（maharit-query 以降のクレートで別の警告が出ないかも確認）
- [x] `cargo fmt --all -- --check` も通ることを確認

## 再発防止

CI で clippy / fmt を実行する（`todo/e2e/77-ci-e2e-pipeline.md` の lint job）。

## 完了内容 (2026-09-30)

- executor.rs の 2 箇所を `.any(Self::is_aggregate)` に修正
- 追加で見つかった `examples/traversal.rs` の `manual_abs_diff` を `goal.abs_diff(current)` に修正
- `cargo fmt --all -- --check` はワークスペース全体で差分があったため、別コミット（style: cargo fmt）で整形
